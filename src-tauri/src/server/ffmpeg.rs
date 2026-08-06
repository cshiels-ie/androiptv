//! ffmpeg-backed TS → HLS remux sessions.
//!
//! Channels whose URL is not HLS (raw MPEG-TS, for example) can't be
//! played by a plain browser. For those, a per-session ffmpeg subprocess
//! remuxes the stream (`-c copy`) into a small live HLS package in a temp
//! dir, which the server then serves (`/stream/ts/{id}/index.m3u8` and
//! `/stream/ts/{id}/seg/{name}`). Idle sessions are killed by the ticker
//! spawned in `super::spawn_server_ticker`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::ServerState;
use crate::db::Channel;

/// Maximum number of concurrent ffmpeg sessions; beyond this the least
/// recently used session is evicted.
const MAX_SESSIONS: usize = 4;

/// Sessions idle for longer than this are killed by the ticker.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// One live ffmpeg session: the subprocess plus its working directory.
pub struct Session {
    pub child: Child,
    pub dir: PathBuf,
    pub last_access: Instant,
}

/// Registry of ffmpeg sessions, one per channel id.
pub struct SessionStore {
    inner: Mutex<HashMap<u64, Session>>,
    /// Last 20 stderr lines per session (diagnostics). Shared (Arc) because
    /// the stderr drainer runs on its own spawned task.
    errors: Arc<Mutex<HashMap<u64, VecDeque<String>>>>,
    /// Set when spawn failed, so manifest requests get a good message.
    spawn_errors: Mutex<HashMap<u64, String>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            errors: Arc::new(Mutex::new(HashMap::new())),
            spawn_errors: Mutex::new(HashMap::new()),
        }
    }

    /// Refreshes the access time of a session.
    pub fn touch(&self, id: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(sess) = inner.get_mut(&id) {
                sess.last_access = Instant::now();
            }
        }
    }

    /// Ensures a live ffmpeg session exists for `id`, spawning one if
    /// needed (and evicting the least recently used session when at
    /// capacity). Never holds a lock across an `.await`.
    pub async fn ensure_session(&self, id: u64, url: &str) -> Result<(), String> {
        let mut displaced: Option<Session> = None;
        {
            let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
            match inner.get_mut(&id) {
                Some(existing) => match existing.child.try_wait() {
                    Ok(None) => {
                        // Still running: just refresh the access time.
                        existing.last_access = Instant::now();
                        return Ok(());
                    }
                    _ => {
                        // Dead or unknown state: tear down and respawn.
                        displaced = inner.remove(&id);
                    }
                },
                None => {
                    // New session: evict the oldest session if at capacity.
                    if inner.len() >= MAX_SESSIONS {
                        let victim = inner
                            .iter()
                            .min_by_key(|(_, s)| s.last_access)
                            .map(|(k, _)| *k);
                        if let Some(victim) = victim {
                            displaced = inner.remove(&victim);
                            self.spawn_errors.lock().unwrap().remove(&victim);
                        }
                    }
                }
            }
        }

        if let Some(mut old) = displaced {
            let _ = old.child.kill().await;
            let _ = old.child.wait().await;
            let _ = std::fs::remove_dir_all(&old.dir);
        }

        let dir = session_dir(id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create session dir {}: {e}", dir.display()))?;

        let bin = match ffmpeg_bin() {
            Some(bin) => bin,
            None => {
                let msg =
                    "ffmpeg unavailable — install ffmpeg or set ANDROIPTV_FFMPEG".to_string();
                self.spawn_errors.lock().unwrap().insert(id, msg.clone());
                return Err(msg);
            }
        };

        let mut command = Command::new(bin);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-user_agent",
                "AndroIPTV/0.1.0",
                "-i",
                url,
                "-c",
                "copy",
                "-f",
                "hls",
                "-hls_time",
                "4",
                "-hls_list_size",
                "8",
                "-hls_flags",
                "delete_segments+independent_segments",
                "-hls_segment_filename",
            ])
            .arg(dir.join("seg_%05d.ts"))
            .arg(dir.join("index.m3u8"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                let msg =
                    format!("failed to start ffmpeg: {e} — install ffmpeg or set ANDROIPTV_FFMPEG");
                self.spawn_errors.lock().unwrap().insert(id, msg.clone());
                return Err(msg);
            }
        };

        // Drain stderr into the per-session ring buffer (last 20 lines).
        if let Some(stderr) = child.stderr.take() {
            let errors = self.errors.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut ring = match errors.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    let queue = ring.entry(id).or_default();
                    if queue.len() >= 20 {
                        queue.pop_front();
                    }
                    queue.push_back(line);
                }
            });
        }

        // Fresh session: clear any stale error state and register it.
        self.spawn_errors.lock().unwrap().remove(&id);
        self.errors.lock().unwrap().remove(&id);
        self.inner.lock().unwrap().insert(
            id,
            Session {
                child,
                dir,
                last_access: Instant::now(),
            },
        );
        Ok(())
    }

    /// Kills sessions idle for longer than [`IDLE_TIMEOUT`] and clears
    /// their error state. The caller (`super::spawn_server_ticker`) drives
    /// the interval; no sleeping happens here.
    pub async fn tick(&self) -> Result<(), String> {
        let stale: Vec<u64> = {
            let inner = self.inner.lock().map_err(|e| e.to_string())?;
            inner
                .iter()
                .filter(|(_, s)| s.last_access.elapsed() > IDLE_TIMEOUT)
                .map(|(k, _)| *k)
                .collect()
        };
        for id in stale {
            let dead = self.inner.lock().map_err(|e| e.to_string())?.remove(&id);
            if let Some(mut sess) = dead {
                let _ = sess.child.kill().await;
                let _ = sess.child.wait().await;
                let _ = std::fs::remove_dir_all(&sess.dir);
                self.errors.lock().unwrap().remove(&id);
                self.spawn_errors.lock().unwrap().remove(&id);
            }
        }
        Ok(())
    }
}

/// Resolves the ffmpeg binary: `ANDROIPTV_FFMPEG` env var, then the Tauri
/// sidecar next to the main executable (where `bundle.externalBin` drops
/// it), then plain `ffmpeg` on PATH.
fn ffmpeg_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ANDROIPTV_FFMPEG") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let mut candidate = parent.join("ffmpeg");
            #[cfg(windows)]
            {
                candidate.set_extension("exe");
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let plain = PathBuf::from("ffmpeg");
    if plain.is_file() {
        return Some(plain);
    }
    // PATH search (`is_file` on a bare name can't see PATH).
    if let Ok(path) = std::env::var("PATH") {
        #[cfg(windows)]
        let name = "ffmpeg.exe";
        #[cfg(not(windows))]
        let name = "ffmpeg";
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Session working directory, derived from the channel id so it can be
/// recomputed anywhere.
fn session_dir(id: u64) -> PathBuf {
    std::env::temp_dir().join("androiptv").join(format!("session-{id}"))
}

/// Looks up a channel, mapping a missing channel to 404 and DB errors to
/// 500.
fn lookup_channel(st: &ServerState, id: u64) -> Result<Channel, Response> {
    match st.db.get_channel(id as i64) {
        Ok(Some(channel)) => Ok(channel),
        Ok(None) => Err(
            (StatusCode::NOT_FOUND, Json(json!({"error": "channel not found"}))).into_response(),
        ),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()),
    }
}

/// The stored spawn error for `id`, if any.
fn spawn_error_of(st: &ServerState, id: u64) -> Option<String> {
    st.sessions.spawn_errors.lock().unwrap().get(&id).cloned()
}

/// Serves the live HLS playlist for a non-HLS channel. The rusqlite
/// lookups above are single quick queries; `url` is owned, so no DB lock
/// is held across the awaits below.
pub async fn handle_manifest(
    Path(id): Path<u64>,
    State(st): State<ServerState>,
) -> impl IntoResponse {
    if let Some(msg) = spawn_error_of(&st, id) {
        return (StatusCode::BAD_GATEWAY, Json(json!({"error": msg}))).into_response();
    }
    let channel = match lookup_channel(&st, id) {
        Ok(channel) => channel,
        Err(resp) => return resp,
    };
    if let Err(e) = st.sessions.ensure_session(id, &channel.url).await {
        return (StatusCode::BAD_GATEWAY, Json(json!({"error": e}))).into_response();
    }

    match tokio::fs::read(session_dir(id).join("index.m3u8")).await {
        Ok(bytes) => {
            st.sessions.touch(id);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "starting"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Serves one HLS segment of a live ffmpeg session.
pub async fn handle_segment(
    Path((id, name)): Path<(u64, String)>,
    State(st): State<ServerState>,
) -> impl IntoResponse {
    // Only plain file names are allowed: this rejects any path traversal
    // attempt ("..", slashes, backslashes, colons, ...).
    let name_ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !name_ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid segment name"})),
        )
            .into_response();
    }

    if let Some(msg) = spawn_error_of(&st, id) {
        return (StatusCode::BAD_GATEWAY, Json(json!({"error": msg}))).into_response();
    }
    let channel = match lookup_channel(&st, id) {
        Ok(channel) => channel,
        Err(resp) => return resp,
    };
    if let Err(e) = st.sessions.ensure_session(id, &channel.url).await {
        return (StatusCode::BAD_GATEWAY, Json(json!({"error": e}))).into_response();
    }

    match tokio::fs::read(session_dir(id).join(&name)).await {
        Ok(bytes) => {
            st.sessions.touch(id);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "video/mp2t"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "segment not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
