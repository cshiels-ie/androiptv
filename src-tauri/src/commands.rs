//! Tauri command handlers: thin adapters between the frontend, the importer
//! modules and the SQLite database. All DB work runs on the blocking pool
//! (`tauri::async_runtime::spawn_blocking` with a cloned `Arc<Db>`) so heavy
//! imports never stall the async runtime.

use std::sync::Arc;

use url::Url;

use crate::db::{Channel, Db, Group, ImportStats, Playlist};
use crate::m3u::{parse_m3u, ParsedChannel};
use crate::server::{server_info, ServerInfo};
use crate::xtream::{fetch_live, verify_auth, XtreamConfig};
use crate::AppData;

/// Run `f` on the blocking pool with a cloned `Arc<Db>` and flatten the
/// error types into a `String`.
async fn run_db<T: Send + 'static>(
    db: Arc<Db>,
    f: impl FnOnce(&Db) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&db))
        .await
        .map_err(|e| e.to_string())? // JoinError -> String
        .map_err(|e| e.to_string()) // rusqlite::Error -> String
}

/// Derive a playlist name from a URL host or a local file name.
fn derived_name(source: &str) -> String {
    if source.starts_with("http://") || source.starts_with("https://") {
        Url::parse(source)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "remote playlist".to_string())
    } else {
        std::path::Path::new(source)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("local playlist")
            .to_owned()
    }
}

#[tauri::command]
pub async fn import_m3u(
    state: tauri::State<'_, AppData>,
    source: String,
    name: Option<String>,
) -> Result<ImportStats, String> {
    let is_url = source.starts_with("http://") || source.starts_with("https://");
    let playlist_name = name.unwrap_or_else(|| derived_name(&source));

    let db = Arc::clone(&state.db);

    // Remote playlists are fetched on the async runtime (a blocking read
    // here would stall it); local files are read on the blocking pool,
    // together with the parse and the DB insert.
    let fetched = if is_url {
        let resp = state
            .http
            .get(source.as_str())
            .send()
            .await
            .map_err(|e| format!("could not fetch playlist {source}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "could not fetch playlist {source}: HTTP {}",
                resp.status()
            ));
        }
        Some(
            resp.text()
                .await
                .map_err(|e| format!("could not read playlist response from {source}: {e}"))?,
        )
    } else {
        None
    };

    tauri::async_runtime::spawn_blocking(move || {
        let content = match fetched {
            Some(text) => text,
            None => std::fs::read_to_string(&source)
                .map_err(|e| format!("could not read playlist file {source}: {e}"))?,
        };
        let mut parsed: Vec<ParsedChannel> = Vec::new();
        parse_m3u(content.as_bytes(), |ch| parsed.push(ch))?;
        let playlist_id = db
            .create_playlist(&playlist_name, "m3u", &source, None)
            .map_err(|e| e.to_string())?;
        db.import_channels(playlist_id, &parsed)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn import_xtream(
    state: tauri::State<'_, AppData>,
    base: String,
    username: String,
    password: String,
    name: Option<String>,
) -> Result<ImportStats, String> {
    let cfg = XtreamConfig {
        base: base.trim().to_string(),
        username,
        password,
    };
    // The client is cheap to clone (Arc inside); pass references to the
    // async helpers.
    let http = state.http.clone();

    // Validate credentials before pulling anything.
    verify_auth(&cfg, &http).await?;
    let channels = fetch_live(&cfg, &http).await?;

    let base_trimmed = cfg.base.trim_end_matches('/').to_string();
    let playlist_name = name.unwrap_or_else(|| {
        Url::parse(&base_trimmed)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "xtream".to_string())
    });
    // Keep the panel endpoint as the source; credentials live in their own
    // columns so live URLs can be rebuilt later.
    let source_url = format!("{base_trimmed}/player_api.php");

    let db = Arc::clone(&state.db);
    let user = cfg.username.clone();
    let pass = cfg.password.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let parsed: Vec<ParsedChannel> = channels
            .into_iter()
            .map(|c| ParsedChannel {
                name: c.name,
                url: c.url,
                logo_url: c.logo_url,
                tvg_id: None,
                tvg_chno: None,
                group: c.group,
            })
            .collect();
        let playlist_id = db
            .create_playlist(
                &playlist_name,
                "xtream",
                &source_url,
                Some((&base_trimmed, &user, &pass)),
            )
            .map_err(|e| e.to_string())?;
        db.import_channels(playlist_id, &parsed)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_playlists(state: tauri::State<'_, AppData>) -> Result<Vec<Playlist>, String> {
    run_db(Arc::clone(&state.db), |db| db.list_playlists()).await
}

#[tauri::command]
pub async fn delete_playlist(state: tauri::State<'_, AppData>, id: i64) -> Result<(), String> {
    run_db(Arc::clone(&state.db), move |db| db.delete_playlist(id)).await
}

#[tauri::command]
pub async fn list_groups(
    state: tauri::State<'_, AppData>,
    playlist_id: i64,
) -> Result<Vec<Group>, String> {
    run_db(Arc::clone(&state.db), move |db| db.list_groups(playlist_id)).await
}

#[tauri::command]
pub async fn search_channels(
    state: tauri::State<'_, AppData>,
    query: String,
    playlist_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Channel>, String> {
    let db = Arc::clone(&state.db);
    let limit = limit.unwrap_or(500);
    run_db(db, move |d| d.search_channels(&query, playlist_id, limit)).await
}

#[tauri::command]
pub async fn channels_by_group(
    state: tauri::State<'_, AppData>,
    group_id: i64,
) -> Result<Vec<Channel>, String> {
    run_db(Arc::clone(&state.db), move |db| db.channels_by_group(group_id)).await
}

#[tauri::command]
pub async fn get_channel(
    state: tauri::State<'_, AppData>,
    id: i64,
) -> Result<Option<Channel>, String> {
    run_db(Arc::clone(&state.db), move |db| db.get_channel(id)).await
}

#[tauri::command]
pub fn get_server_info() -> Result<ServerInfo, String> {
    server_info()
}
