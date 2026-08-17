//! Smart HLS passthrough proxy.
//!
//! Fetches any remote HLS playlist (master or media), rewrites every URI
//! inside it so it resolves through this same-origin proxy
//! (`/proxy/hls/{id}?u=<urlencoded absolute url>`), and serves it with the
//! proper content type. Segments, keys and other media are streamed
//! through untouched (with Range support), so the whole playlist chain
//! recurses through this handler and the player only ever talks to our
//! origin.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use super::ServerState;

/// How long a negative probe decision ("this URL is not HLS") is trusted.
/// The player re-requests the same probe URL on every playlist refresh, so
/// this bounds the number of aborted upstream connections per channel.
pub const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Deserialize)]
pub struct HlsQuery {
    pub u: String,
    /// `probe=1` (set only on the initial channel request, never on
    /// recursed segment/playlist fetches): if the upstream is NOT an HLS
    /// playlist, redirect to the ffmpeg remuxer instead of streaming the
    /// binary through. Xtream panels often serve HLS even at `.ts` URLs,
    /// so probing lets them skip the ffmpeg session entirely.
    pub probe: Option<String>,
}

pub async fn handle_hls(
    Path(id): Path<u64>,
    Query(q): Query<HlsQuery>,
    headers: HeaderMap,
    State(st): State<ServerState>,
) -> impl IntoResponse {
    // Only http(s) upstreams are allowed.
    let parsed = match url::Url::parse(&q.u) {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u,
        _ => return bad_request(),
    };

    // A previous probe already found this channel to be non-HLS: skip the
    // upstream connection entirely (each probe aborts a real connection
    // mid-body, and panels with per-connection limits then refuse the
    // ffmpeg session).
    if q.probe.as_deref() == Some("1") {
        if let Some(at) = st.probe_cache.lock().unwrap().get(&id).copied() {
            if at.elapsed() < PROBE_CACHE_TTL {
                return redirect_to_remux(id);
            }
        }
    }

    // Fetch (the client's default redirect policy follows redirects).
    let mut response = match st.http.get(q.u.as_str()).send().await {
        Ok(r) => r,
        Err(_) => return bad_gateway(),
    };
    if response.error_for_status_ref().is_err() {
        return bad_gateway();
    }
    // Clone the final URL up front: consuming the body below (`bytes()` /
    // `chunk()`) moves the response, so `response.url()` is not available
    // afterwards.
    let final_url = response.url().clone();

    // A playlist is identified by its content type, its final URL, or the
    // body marker `#EXTM3U`.
    let playlist_by_headers = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_lowercase().contains("mpegurl"))
        .unwrap_or(false)
        || response.url().path().to_lowercase().ends_with(".m3u8");

    if playlist_by_headers {
        // Playlists are small: buffer the whole body and rewrite it.
        let body = match response.bytes().await {
            Ok(b) => b,
            Err(_) => return bad_gateway(),
        };
        return serve_playlist(&body, &final_url, id);
    }

    // Headers don't identify a playlist, but some servers label playlists
    // with a generic content type: peek at the first bytes.
    match response.chunk().await {
        Ok(Some(first)) if first.starts_with(b"#EXTM3U") => {
            let mut body = first.to_vec();
            while let Ok(Some(chunk)) = response.chunk().await {
                body.extend_from_slice(&chunk);
            }
            serve_playlist(&body, &final_url, id)
        }
        _ => {
            // The upstream is not HLS (binary TS, image, ...).
            if q.probe.as_deref() == Some("1") {
                // The caller wanted to know whether this URL serves HLS;
                // it doesn't — fall back to the ffmpeg remuxer. The player
                // follows the same-origin redirect transparently.
                st.probe_cache.lock().unwrap().insert(id, std::time::Instant::now());
                return redirect_to_remux(id);
            }
            // Segment, key or image: stream it through the shared proxy
            // helper, which issues a fresh request and honors Range (the
            // response above is already partially consumed).
            let range = headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok());
            match super::proxy_stream(&st.http, parsed.as_str(), range, &[]).await {
                Ok(ok) => ok.into_response(),
                Err(_) => bad_gateway(),
            }
        }
    }
}

/// 302 to the ffmpeg remuxer for this channel's namespace id.
fn redirect_to_remux(id: u64) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("/stream/ts/{id}/index.m3u8"))],
        (),
    )
        .into_response()
}

fn bad_request() -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid url"}))).into_response()
}

fn bad_gateway() -> Response {
    (StatusCode::BAD_GATEWAY, Json(json!({"error": "upstream error"}))).into_response()
}

/// Serves an already-fetched playlist body, rewritten to go through this
/// proxy.
fn serve_playlist(body: &[u8], base: &url::Url, id: u64) -> Response {
    let rewritten = rewrite_playlist(&String::from_utf8_lossy(body), base, id);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        rewritten.into_bytes(),
    )
        .into_response()
}

/// Rewrites every URI in an HLS playlist so all media references resolve
/// through our same-origin proxy. `base` is the absolute URL the playlist
/// was fetched from; `Url::join` handles absolute URLs, relative paths and
/// `../` segments for us.
fn rewrite_playlist(text: &str, base: &url::Url, id: u64) -> String {
    text.split('\n')
        .map(|line| rewrite_line(line, base, id))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rewrite_line(line: &str, base: &url::Url, id: u64) -> String {
    // Lines that embed URIs inside attributes (#EXT-X-KEY, #EXT-X-MEDIA,
    // #EXT-X-MAP, ...) get every URI="..." occurrence rewritten in place.
    if line.contains("URI=\"") {
        let mut out = String::with_capacity(line.len() + 64);
        let mut rest = line;
        while let Some(pos) = rest.find("URI=\"") {
            // Everything up to and including `URI="`.
            out.push_str(&rest[..pos + 5]);
            let after = &rest[pos + 5..];
            match after.find('"') {
                Some(end) => {
                    let value = &after[..end];
                    let absolute = base
                        .join(value)
                        .map(|u| u.to_string())
                        .unwrap_or_else(|_| value.to_string());
                    out.push_str(&proxied(&absolute, id));
                    // Keep the closing quote for the next pass.
                    rest = &after[end..];
                }
                None => {
                    // Unterminated URI attribute: emit the tail verbatim.
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        return out;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        // Tags (#EXTINF:, #EXT-X-STREAM-INF, #EXT-X-VERSION, #EXTM3U,
        // comments) and blank lines pass through unchanged.
        line.to_string()
    } else {
        // Bare URI line: segment, key or sub-playlist reference.
        let absolute = base
            .join(trimmed)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| trimmed.to_string());
        proxied(&absolute, id)
    }
}

/// Emits the same-origin proxy URL for an absolute upstream URL.
fn proxied(url: &str, id: u64) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
    format!("/proxy/hls/{id}?u={encoded}")
}
