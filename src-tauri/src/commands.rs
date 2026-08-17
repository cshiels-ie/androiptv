//! Tauri command handlers: thin adapters between the frontend, the importer
//! modules and the SQLite database. All DB work runs on the blocking pool
//! (`tauri::async_runtime::spawn_blocking` with a cloned `Arc<Db>`) so heavy
//! imports never stall the async runtime.

use std::sync::Arc;

use url::Url;

use crate::db::{Channel, Db, Episode, Group, ImportStats, NewChannel, Playlist};
use crate::m3u::{parse_m3u, ParsedChannel};
use crate::server::{server_info, ServerInfo};
use crate::xtream::{fetch_live, fetch_series, fetch_series_episodes, fetch_vod, verify_auth, XtreamConfig};
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
    // Live + VOD + series in one import; panels without a VOD/series
    // section answer with an error object, which the fetchers treat as
    // empty rather than failing the whole import.
    let channels = fetch_live(&cfg, &http).await?;
    let vod = fetch_vod(&cfg, &http).await?;
    let series = fetch_series(&cfg, &http).await?;

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
        let mut items: Vec<NewChannel> =
            Vec::with_capacity(channels.len() + vod.len() + series.len());
        items.extend(channels.into_iter().map(|c| NewChannel {
            name: c.name,
            url: c.url,
            logo_url: c.logo_url,
            tvg_id: None,
            tvg_chno: None,
            kind: "live".into(),
            remote_id: None,
            group: c.group,
        }));
        items.extend(vod.into_iter().map(|i| NewChannel {
            name: i.name,
            url: i.url,
            logo_url: i.logo_url,
            tvg_id: None,
            tvg_chno: None,
            kind: "vod".into(),
            remote_id: i.remote_id,
            group: i.group,
        }));
        items.extend(series.into_iter().map(|i| NewChannel {
            name: i.name,
            url: i.url,
            logo_url: i.logo_url,
            tvg_id: None,
            tvg_chno: None,
            kind: "series".into(),
            remote_id: i.remote_id,
            group: i.group,
        }));
        let playlist_id = db
            .create_playlist(
                &playlist_name,
                "xtream",
                &source_url,
                Some((&base_trimmed, &user, &pass)),
            )
            .map_err(|e| e.to_string())?;
        db.import_items(playlist_id, &items).map_err(|e| e.to_string())
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
    kind: Option<String>,
) -> Result<Vec<Group>, String> {
    run_db(Arc::clone(&state.db), move |db| db.list_groups(playlist_id, kind.as_deref())).await
}

#[tauri::command]
pub async fn search_channels(
    state: tauri::State<'_, AppData>,
    query: String,
    playlist_id: Option<i64>,
    kind: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Channel>, String> {
    let db = Arc::clone(&state.db);
    let limit = limit.unwrap_or(500);
    run_db(db, move |d| d.search_channels(&query, playlist_id, kind.as_deref(), limit)).await
}

/// Episodes for a series channel, fetched lazily from the panel on first
/// open and cached in the DB afterwards (a failed fetch leaves the old
/// cache untouched).
#[tauri::command]
pub async fn series_episodes(
    state: tauri::State<'_, AppData>,
    channel_id: i64,
) -> Result<Vec<Episode>, String> {
    let db = Arc::clone(&state.db);
    let http = state.http.clone();

    let channel = run_db(Arc::clone(&db), move |d| d.get_channel(channel_id))
        .await?
        .ok_or_else(|| "channel not found".to_string())?;
    if channel.kind != "series" {
        return Err("not a series channel".to_string());
    }
    if let Some(eps) = run_db(Arc::clone(&db), move |d| d.episodes_for_channel(channel_id)).await? {
        return Ok(eps);
    }
    let creds = run_db(Arc::clone(&db), move |d| d.playlist_xtream_creds(channel.playlist_id))
        .await?
        .ok_or_else(|| "not an Xtream source".to_string())?;
    let series_id = channel
        .remote_id
        .as_deref()
        .and_then(|r| r.parse::<i64>().ok())
        .ok_or_else(|| "series id missing".to_string())?;

    let cfg = XtreamConfig {
        base: creds.0,
        username: creds.1,
        password: creds.2,
    };
    let raw = fetch_series_episodes(&cfg, &http, series_id).await?;
    let eps: Vec<Episode> = raw
        .into_iter()
        .map(|e| Episode {
            id: 0,
            channel_id,
            season: e.season,
            episode_num: e.episode_num,
            title: e.title,
            url: e.url,
            logo_url: e.logo_url,
        })
        .collect();
    let for_db = eps.clone();
    run_db(db, move |d| d.replace_episodes(channel_id, &for_db)).await?;
    Ok(eps)
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

/// Reads the saved server prefs (host override, port) and builds the
/// advertised ServerInfo from them.
async fn load_server_info(db: Arc<Db>) -> Result<ServerInfo, String> {
    let (ip_override, port_pref) = run_db(db, |d| {
        let ip_override = d.get_setting("server_ip_override")?;
        let port_pref = d.get_setting("server_port")?;
        Ok((ip_override, port_pref))
    })
    .await?;
    let ip_override = ip_override.as_deref().filter(|s| !s.is_empty());
    let port_pref = port_pref.and_then(|v| v.parse::<u16>().ok());
    server_info(ip_override, port_pref)
}

#[tauri::command]
pub async fn get_server_info(state: tauri::State<'_, AppData>) -> Result<ServerInfo, String> {
    load_server_info(Arc::clone(&state.db)).await
}

/// Stores the LAN server host/port preferences and returns the updated
/// server info. The host override applies immediately (every advertised
/// URL, QR code and status chip); the port is applied on the next app
/// start because the listener binds once at startup. Pass None (or an
/// empty/“auto” host) to clear a preference and go back to automatic.
#[tauri::command]
pub async fn set_server_prefs(
    state: tauri::State<'_, AppData>,
    ip_override: Option<String>,
    port: Option<u16>,
) -> Result<ServerInfo, String> {
    // Sanitize the host: allow a scheme accidentally pasted from an
    // address bar, but nothing else — it must end up as a bare host/IP
    // (no slashes, ports or whitespace).
    let ip_override = match ip_override {
        Some(ip) => {
            let mut cleaned = ip.trim().to_string();
            for prefix in ["http://", "https://"] {
                if let Some(stripped) = cleaned.strip_prefix(prefix) {
                    cleaned = stripped.to_string();
                }
            }
            if cleaned.is_empty() || cleaned.len() > 253 || cleaned.chars().any(|c| {
                c.is_whitespace() || c == '/' || c == ':'
            }) {
                return Err(format!("invalid host: {ip:?}"));
            }
            let cleared = matches!(cleaned.as_str(), "" | "auto");
            (!cleared).then_some(cleaned)
        }
        None => None,
    };

    let db = Arc::clone(&state.db);
    run_db(db, move |d| {
        match &ip_override {
            Some(ip) => d.set_setting("server_ip_override", ip)?,
            None => d.set_setting("server_ip_override", "")?,
        }
        match port {
            Some(p) => d.set_setting("server_port", &p.to_string())?,
            None => d.set_setting("server_port", "")?,
        }
        Ok(())
    })
    .await?;

    load_server_info(Arc::clone(&state.db)).await
}
