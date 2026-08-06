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
}

fn app_data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::import_m3u,
            commands::import_xtream,
            commands::list_playlists,
            commands::delete_playlist,
            commands::list_groups,
            commands::search_channels,
            commands::channels_by_group,
            commands::get_channel,
            commands::get_server_info,
        ])
        .setup(|app| {
            let data_dir = app_data_dir(app)?;
            let db = Arc::new(Db::open(&data_dir.join("androiptv.db"))?);

            let http = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

            app.manage(AppData {
                db: db.clone(),
                http: http.clone(),
            });

            // Embedded LAN TV server + ffmpeg session sweeper.
            let state = server::ServerState {
                db,
                http,
                sessions: Arc::new(server::ffmpeg::SessionStore::new()),
            };
            server::spawn_server_ticker(state.clone());
            tauri::async_runtime::spawn(async move {
                server::spawn_server(state).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
