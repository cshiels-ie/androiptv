//! Embedded LAN HTTP server (axum) for the TV page, a small JSON API, and
//! two stream backends:
//!
//! - `/proxy/hls/{id}` — smart HLS passthrough proxy that rewrites remote
//!   playlists so they play same-origin (see [`hls_proxy`]). With `probe=1`
//!   (used for non-`.m3u8` URLs) it also sniffs for HLS: playlists are
//!   served directly, anything else 302s to the ffmpeg remuxer.
//! - `/stream/ts/{id}` — ffmpeg-backed remux of raw MPEG-TS (or any
//!   non-HLS) channel URLs into live HLS (see [`ffmpeg`]).
//!
//! The server binds `0.0.0.0:4040` (falling back to an ephemeral port if
//! 4040 is taken) and runs on Tauri's own async runtime, so no second
//! tokio runtime is ever created.

pub mod ffmpeg;
pub mod hls_proxy;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::db::{Channel, Db, Group};

/// Set once, the first time the server binds, and never changed afterwards.
pub static PORT: OnceLock<u16> = OnceLock::new();

/// Episode ids live in the same `/proxy/hls/{id}` + `/stream/ts/{id}` id
/// namespace as channel ids (ffmpeg session keys, temp dirs). Both are
/// small sequential integers, so an episode id would collide with a
/// channel id; shifting episode ids by this offset keeps them disjoint.
/// The offset id is purely a namespacing token — the proxy never
/// DB-lookups it, and `ffmpeg.rs` maps it back via `lookup_stream`.
pub const EPISODE_ID_OFFSET: u64 = 1 << 30;

/// Information about the running LAN server, for the frontend status view.
/// `host`/`ip_override`/`port_pref` are the user's configured preferences
/// (None = automatic), so the UI can render a host/port picker that
/// matches what the server actually advertises.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    pub ips: Vec<String>,
    pub port: u16,
    pub host: String,
    pub ip_override: Option<String>,
    pub port_pref: Option<u16>,
}

/// Shared state for every HTTP handler: database, HTTP client, the ffmpeg
/// session store and the probe-decision cache.
#[derive(Clone)]
pub struct ServerState {
    pub db: Arc<Db>,
    pub http: reqwest::Client,
    pub sessions: Arc<ffmpeg::SessionStore>,
    /// Channel ids whose probe=1 request found a non-HLS (binary) stream.
    /// The probe opens a real upstream connection and abandons it mid-body,
    /// and the player refreshes the playlist URL every few seconds — so
    /// without a cache, connection-limited panels see a fresh aborted
    /// connection per refresh and refuse the ffmpeg one. Cached for
    /// [`crate::server::hls_proxy::PROBE_CACHE_TTL`].
    pub probe_cache: Arc<Mutex<HashMap<u64, Instant>>>,
}

/// Current server info, or an error if the server has not started yet.
/// `ip_override` (user-chosen host for the advertised URL) and
/// `port_pref` (configured port, applied at next startup) come from the
/// settings table; `ips` stays the full detected list so the UI can offer
/// every interface.
pub fn server_info(
    ip_override: Option<&str>,
    port_pref: Option<u16>,
) -> Result<ServerInfo, String> {
    let ips = crate::net::local_ips();
    let port = PORT
        .get()
        .copied()
        .ok_or_else(|| "server not started".to_string())?;
    let host = match ip_override {
        Some(ip) if !ip.is_empty() => ip.to_string(),
        _ => ips.first().cloned().unwrap_or_default(),
    };
    let url = if host.is_empty() {
        String::new()
    } else {
        format!("http://{host}:{port}")
    };
    Ok(ServerInfo {
        url,
        ips,
        port,
        host,
        ip_override: ip_override.map(str::to_string),
        port_pref,
    })
}

/// Spawns a background loop that ticks the ffmpeg session store every 30
/// seconds, so idle sessions are torn down.
///
/// We prefer `tauri::async_runtime::spawn` because lib.rs runs inside
/// Tauri's own tokio runtime (spawning a second runtime would clash). If
/// this module is ever used outside Tauri, swap this for `tokio::spawn`.
pub fn spawn_server_ticker(state: ServerState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = state.sessions.tick().await {
                eprintln!("[server] session tick failed: {e}");
                // On error: sleep a little and continue the loop.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });
}

/// Binds the TCP listener (a user-configured port if any, else 4040, else
/// an ephemeral port if taken), publishes the port in [`PORT`] and serves
/// the router forever.
pub async fn spawn_server(state: ServerState, preferred_port: Option<u16>) {
    // Candidate order: user preference, then the default 4040.
    let candidates: Vec<u16> = preferred_port
        .into_iter()
        .chain(std::iter::once(4040))
        .collect();
    let listener = match bind_first(&candidates).await {
        Some(listener) => listener,
        None => match tokio::net::TcpListener::bind("0.0.0.0:0").await {
            Ok(listener) => {
                eprintln!("[server] all preferred ports busy; using an ephemeral port");
                listener
            }
            Err(e) => {
                eprintln!("[server] failed to bind a TCP listener: {e}");
                return;
            }
        },
    };
    let addr = listener.local_addr().unwrap();
    let _ = PORT.set(addr.port());
    eprintln!("[server] TV server listening on http://{addr}");
    if let Err(e) = axum::serve(listener, router(state)).await {
        eprintln!("[server] server error: {e}");
    }
}

/// Tries each port in order, returning the first successful listener.
async fn bind_first(ports: &[u16]) -> Option<tokio::net::TcpListener> {
    for &port in ports {
        match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => return Some(listener),
            Err(e) => eprintln!("[server] port {port} busy ({e}); trying next"),
        }
    }
    None
}

/// Builds the axum router for the LAN server.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/api/status", get(status))
        .route("/api/groups", get(groups))
        .route("/api/channels", get(channels))
        .route("/api/logo", get(logo))
        .route("/api/play/{id}", get(play))
        .route("/api/play/episode/{id}", get(play_episode))
        .route("/api/epg", get(epg))
        .route("/proxy/hls/{id}", get(hls_proxy::handle_hls))
        .route("/stream/ts/{id}/index.m3u8", get(ffmpeg::handle_manifest))
        .route("/stream/ts/{id}/seg/{name}", get(ffmpeg::handle_segment))
        .fallback(not_found)
        .with_state(state)
        .layer(middleware::from_fn(cors_headers))
}

/// The LAN server must be readable cross-origin: the desktop webview
/// (origin `tauri://localhost`, or `localhost:1420` in dev) calls this
/// server over the LAN, and hls.js issues XHRs to `/proxy/hls/...` from
/// that origin. TV browsers are same-origin and unaffected. GET + Range
/// are CORS-safelisted, so no preflight is involved.
async fn cors_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Range"),
    );
    response
}

const TV_PAGE: &str = include_str!("../../resources/tv/index.html");

async fn index_page() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], TV_PAGE)
}

#[derive(serde::Serialize)]
struct StatusBody {
    name: &'static str,
    version: &'static str,
    port: u16,
    channel_count: i64,
}

async fn status(State(st): State<ServerState>) -> Json<StatusBody> {
    Json(StatusBody {
        name: "AndroIPTV",
        version: env!("CARGO_PKG_VERSION"),
        port: PORT.get().copied().unwrap_or(0),
        channel_count: st.db.channel_count().unwrap_or(0),
    })
}

/// `?kind=live|vod|series`, defaulting to `live` so the TV page (which
/// sends no query and is live-only) never sees VOD/series groups.
#[derive(Deserialize)]
struct GroupsQuery {
    kind: Option<String>,
}

async fn groups(
    Query(q): Query<GroupsQuery>,
    State(st): State<ServerState>,
) -> Result<Json<Vec<Group>>, (StatusCode, Json<serde_json::Value>)> {
    st.db.groups_all(q.kind.as_deref().unwrap_or("live"))
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))
}

#[derive(Deserialize)]
struct ChannelQuery {
    group: Option<i64>,
    q: Option<String>,
    limit: Option<i64>,
}

/// The TV page is live-only: search and the no-group fallback are
/// hard-filtered to kind 'live' so movies/series never leak in.
async fn channels(
    Query(q): Query<ChannelQuery>,
    State(st): State<ServerState>,
) -> Result<Json<Vec<Channel>>, (StatusCode, Json<serde_json::Value>)> {
    let limit = q.limit.unwrap_or(500);
    let result = match &q.q {
        Some(query) if !query.trim().is_empty() => {
            st.db.search_channels(query, None, Some("live"), limit)
        }
        _ => match q.group {
            Some(group) => st.db.channels_by_group(group),
            None => st.db.channels_all(limit, "live"),
        },
    };
    result
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))
}

#[derive(Deserialize)]
struct LogoQuery {
    u: String,
}

/// Proxies a channel logo image. Logos are images: no range support needed.
async fn logo(Query(q): Query<LogoQuery>, State(st): State<ServerState>) -> impl IntoResponse {
    let url = match url::Url::parse(&q.u) {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid url"})),
            )
                .into_response()
        }
    };
    // Many provider CDNs hotlink-protect logos: send the logo's own origin
    // as Referer so they treat us as a first-party page, not a leech.
    let referer = url.origin().ascii_serialization();
    match proxy_stream(&st.http, url.as_str(), None, &[("referer", referer)]).await {
        Ok((status, mut headers, body)) => {
            // Logos are static; let WebViews reuse them across list
            // renders instead of re-fetching through the proxy.
            headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("public, max-age=86400"),
            );
            (status, headers, body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

/// Resolves the play URL for a channel: HLS goes through the same-origin
/// proxy; live non-HLS gets probed (HLS-at-.ts panels skip the remuxer,
/// real TS redirects to ffmpeg); VOD/series native files play directly,
/// non-native files (mkv/…) are remuxed to HLS.
async fn play(Path(id): Path<u64>, State(st): State<ServerState>) -> impl IntoResponse {
    let channel = match st.db.get_channel(id as i64) {
        Ok(Some(channel)) => channel,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "channel not found"})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    // Series have no single stream URL; the UI opens them to browse
    // episodes instead.
    if channel.kind == "series" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "series has no single stream — open it to browse episodes"})),
        )
            .into_response();
    }
    Json(play_info_for(&channel.kind, &channel.url, id)).into_response()
}

/// Play resolution for a single series episode (lazy-fetched row). The
/// episode id is offset into its own namespace in proxy/ffmpeg paths so it
/// can never collide with a channel id.
async fn play_episode(Path(id): Path<u64>, State(st): State<ServerState>) -> impl IntoResponse {
    let ep = match st.db.get_episode(id as i64) {
        Ok(Some(ep)) => ep,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "episode not found"})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    Json(play_info_for("episode", &ep.url, id + EPISODE_ID_OFFSET)).into_response()
}

/// Builds the same-origin play payload for a stream. `kind` is the stored
/// row kind ("live"/"vod") or "episode"; `id` is the proxy/ffmpeg
/// namespace id (channel id, or episode id + EPISODE_ID_OFFSET).
///
/// Every payload carries all three playable URLs so the desktop player
/// can switch backend on the fly (Auto / hls.js / native / ffmpeg / raw):
/// `url` (default proxy path), `ts` (ffmpeg remux manifest) and `direct`
/// (raw upstream URL for a last-resort native attempt).
fn play_info_for(kind: &str, url: &str, id: u64) -> serde_json::Value {
    let escaped: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
    let make = |kind: &str, path: String| {
        json!({
            "kind": kind,
            "url": path,
            "ts": format!("/stream/ts/{id}/index.m3u8"),
            "direct": url,
        })
    };
    if url.to_lowercase().contains(".m3u8") {
        make("hls", format!("/proxy/hls/{id}?u={escaped}"))
    } else if kind == "vod" || kind == "episode" {
        if is_browser_native(url) {
            // Browser-native files (mp4/webm/mov/…) play straight from the
            // panel — the player uses `direct`; `url` stays the byte-proxy
            // path as a fallback.
            make("file", format!("/proxy/hls/{id}?u={escaped}"))
        } else {
            // mkv/ts/avi etc. → ffmpeg remux to HLS (the remuxer drops
            // subtitle tracks, so these survive).
            make("ts", format!("/proxy/hls/{id}?u={escaped}&probe=1"))
        }
    } else {
        // Live: probe non-HLS URLs first — Xtream panels often serve HLS
        // even at `.ts` URLs. Real TS redirects to the ffmpeg remuxer.
        make("ts", format!("/proxy/hls/{id}?u={escaped}&probe=1"))
    }
}

/// Extensions browsers can play natively. Anything else needs the ffmpeg
/// remuxer.
fn is_browser_native(url: &str) -> bool {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "mp4" | "m4v" | "webm" | "mov" | "ogv" | "ogg" | "mp3" | "aac" | "m4a"
    )
}

/// EPG endpoint: contract stub for a later milestone.
async fn epg() -> Json<serde_json::Value> {
    Json(json!([]))
}

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
}

/// Fetches `url` (honoring an optional `Range` header) and relays the
/// upstream status, selected headers and body stream straight to the
/// caller — a transparent byte proxy for segments, keys and logos.
/// `extra_headers` are sent upstream with the request (e.g. a Referer for
/// hotlink-protected logo CDNs).
pub async fn proxy_stream(
    client: &reqwest::Client,
    url: &str,
    range: Option<&str>,
    extra_headers: &[(&str, String)],
) -> Result<(StatusCode, HeaderMap, Body), String> {
    let mut request = client.get(url);
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    for (name, value) in extra_headers {
        request = request.header(*name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {e}"))?;

    let mut headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
    ] {
        if let Some(value) = response.headers().get(&name) {
            headers.insert(name, value.clone());
        }
    }

    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let body = Body::from_stream(response.bytes_stream());
    Ok((status, headers, body))
}
