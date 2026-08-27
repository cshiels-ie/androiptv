//! AndroIPTV — Tauri v2 backend.
//!
//! Responsibilities:
//! - SQLite persistence of playlists/channels (crate::db)
//! - M3U / Xtream Codes imports (crate::m3u, crate::xtream)
//! - An embedded axum server on 0.0.0.0 so a Smart TV browser on the
//!   LAN can browse and play channels with no casting app. It proxies
//!   HLS (with playlist URL rewriting) and remuxes raw TS via ffmpeg
//!   (crate::server).
//!
//! The server runs on `tauri::async_runtime` (Tauri's own tokio
//! runtime) — we never create a second runtime, which would clash.

mod commands;
mod db;
mod m3u;
mod net;
mod server;
mod xtream;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;

use crate::db::Db;

/// Shared app state exposed to commands and the HTTP server.
pub struct AppData {
    pub db: Arc<Db>,
    pub http: reqwest::Client,
    /// Streaming client with no total-request timeout, used to proxy long
    /// media (native VOD files, HLS segments) end-to-end without the 60s
    /// cap truncating a long movie.
    pub stream_http: reqwest::Client,
}

fn app_data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_cast::init())
        .invoke_handler(tauri::generate_handler![
            commands::import_m3u,
            commands::import_xtream,
            commands::list_playlists,
            commands::delete_playlist,
            commands::list_groups,
            commands::search_channels,
            commands::channels_by_group,
            commands::get_channel,
            commands::series_episodes,
            commands::get_server_info,
            commands::set_server_prefs,
        ])
        .setup(|app| {
            let data_dir = app_data_dir(app)?;
            let db = Arc::new(Db::open(&data_dir.join("androiptv.db"))?);

            // Browser-ish UA: provider CDNs (logos, playlists) commonly
            // reject the bare "reqwest/x.y.z" default. The browser-media
            // default headers make anti-leech panels treat us as a first
            // -party media request (see build_client).
            let http = build_client(Duration::from_secs(60))?;
            // Long media streams (native VOD files through the byte proxy)
            // must never be truncated by a total-request timeout, so they
            // use a dedicated client with only a connect/read timeout.
            let stream_http = build_client(Duration::from_secs(0))?;

            app.manage(AppData {
                db: db.clone(),
                http: http.clone(),
                stream_http: stream_http.clone(),
            });

            // Embedded LAN TV server + ffmpeg session sweeper. A saved
            // port preference (set_server_prefs) wins over the default
            // 4040; it binds once at startup.
            let port_pref = db
                .get_setting("server_port")
                .ok()
                .flatten()
                .and_then(|v| v.parse::<u16>().ok());
            let state = server::ServerState {
                db,
                http,
                stream_http: stream_http.clone(),
                // Writable dir for staging a runnable copy of the Android
                // ffmpeg sidecar (the extracted native-lib path is noexec).
                sessions: Arc::new(server::ffmpeg::SessionStore::new(
                    data_dir.join("ffmpeg"),
                )),
                probe_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            };
            server::spawn_server_ticker(state.clone());
            tauri::async_runtime::spawn(async move {
                server::spawn_server(state, port_pref).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Builds a reqwest client with a browser-like identity, so provider CDNs
/// (logos, playlists, streams) treat the proxy as a real first-party media
/// request instead of rejecting a bare `reqwest/x.y.z` UA.
///
/// `total_timeout` is the whole-request deadline (0 disables it — the
/// streaming client must never cap a long movie mid-file; it keeps a 10s
/// connect timeout and a 30s per-read idle timeout so a stalled upstream
/// still fails rather than hanging forever).
fn build_client(total_timeout: Duration) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        );
    if !total_timeout.is_zero() {
        builder = builder.timeout(total_timeout);
    }
    // Look like a real browser to anti-leech panels: some serve streams fine
    // to a browser but reject requests that lack the Accept-Language /
    // Sec-Fetch-* headers a browser media request carries (the panel then
    // answers 403/5xx). These are harmless to panels that don't check.
    builder = builder.default_headers({
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        h.insert(reqwest::header::ACCEPT, HeaderValue::from_static("*/*"));
        h.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        h.insert(
            reqwest::header::HeaderName::from_static("sec-fetch-dest"),
            HeaderValue::from_static("video"),
        );
        h.insert(
            reqwest::header::HeaderName::from_static("sec-fetch-mode"),
            HeaderValue::from_static("no-cors"),
        );
        h.insert(
            reqwest::header::HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
        h
    });
    Ok(builder.build()?)
}
