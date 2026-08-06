//! Embedded LAN HTTP server (axum) for the TV page, a small JSON API, and
//! two stream backends:
//!
//! - `/proxy/hls/{id}` — smart HLS passthrough proxy that rewrites remote
//!   playlists so they play same-origin (see [`hls_proxy`]).
//! - `/stream/ts/{id}` — ffmpeg-backed remux of raw MPEG-TS (or any
//!   non-HLS) channel URLs into live HLS (see [`ffmpeg`]).
//!
//! The server binds `0.0.0.0:4040` (falling back to an ephemeral port if
//! 4040 is taken) and runs on Tauri's own async runtime, so no second
//! tokio runtime is ever created.

pub mod ffmpeg;
pub mod hls_proxy;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

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

/// Information about the running LAN server, for the frontend status view.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub url: String,
    pub ips: Vec<String>,
    pub port: u16,
}

/// Shared state for every HTTP handler: database, HTTP client and the
/// ffmpeg session store.
#[derive(Clone)]
pub struct ServerState {
    pub db: Arc<Db>,
    pub http: reqwest::Client,
    pub sessions: Arc<ffmpeg::SessionStore>,
}

/// Current server info, or an error if the server has not started yet.
pub fn server_info() -> Result<ServerInfo, String> {
    let ips = crate::net::local_ips();
    let port = PORT
        .get()
        .copied()
        .ok_or_else(|| "server not started".to_string())?;
    let url = ips
        .first()
        .map(|ip| format!("http://{ip}:{port}"))
        .unwrap_or_default();
    Ok(ServerInfo { url, ips, port })
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

/// Binds the TCP listener (4040, or an ephemeral port if that's taken),
/// publishes the port in [`PORT`] and serves the router forever.
pub async fn spawn_server(state: ServerState) {
    let listener = match tokio::net::TcpListener::bind("0.0.0.0:4040").await {
        Ok(listener) => listener,
        Err(first_err) => match tokio::net::TcpListener::bind("0.0.0.0:0").await {
            Ok(listener) => {
                eprintln!("[server] port 4040 busy ({first_err}); using an ephemeral port");
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

/// Builds the axum router for the LAN server.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/api/status", get(status))
        .route("/api/groups", get(groups))
        .route("/api/channels", get(channels))
        .route("/api/logo", get(logo))
        .route("/api/play/{id}", get(play))
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

async fn groups(
    State(st): State<ServerState>,
) -> Result<Json<Vec<Group>>, (StatusCode, Json<serde_json::Value>)> {
    st.db.groups_all()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))
}

#[derive(Deserialize)]
struct ChannelQuery {
    group: Option<i64>,
    q: Option<String>,
    limit: Option<i64>,
}

async fn channels(
    Query(q): Query<ChannelQuery>,
    State(st): State<ServerState>,
) -> Result<Json<Vec<Channel>>, (StatusCode, Json<serde_json::Value>)> {
    let limit = q.limit.unwrap_or(500);
    let result = match &q.q {
        Some(query) if !query.trim().is_empty() => st.db.search_channels(query, None, limit),
        _ => match q.group {
            Some(group) => st.db.channels_by_group(group),
            None => st.db.channels_all(limit),
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
    match proxy_stream(&st.http, url.as_str(), None).await {
        Ok(ok) => ok.into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

/// Resolves the play URL for a channel: HLS channels go through the
/// same-origin proxy, everything else through the ffmpeg remuxer.
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
    let is_hls = channel.url.to_lowercase().contains(".m3u8");
    let url = if is_hls {
        let escaped: String =
            url::form_urlencoded::byte_serialize(channel.url.as_bytes()).collect();
        format!("/proxy/hls/{id}?u={escaped}")
    } else {
        format!("/stream/ts/{id}/index.m3u8")
    };
    Json(json!({ "kind": if is_hls { "hls" } else { "ts" }, "url": url })).into_response()
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
pub async fn proxy_stream(
    client: &reqwest::Client,
    url: &str,
    range: Option<&str>,
) -> Result<(StatusCode, HeaderMap, Body), String> {
    let mut request = client.get(url);
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
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
        if let Some(value) = response.headers().get(name) {
            headers.insert(name, value.clone());
        }
    }

    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let body = Body::from_stream(response.bytes_stream());
    Ok((status, headers, body))
}
