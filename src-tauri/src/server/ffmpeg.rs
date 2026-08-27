//! ffmpeg-backed TS → HLS remux sessions.
//!
//! Channels whose URL is not HLS (raw MPEG-TS, for example) can't be
//! played by a plain browser. For those, a per-session ffmpeg subprocess
//! remuxes the stream into a small live HLS package in a temp dir: video
//! is copied untouched (`-c:v copy`), audio is re-encoded to AAC because
//! no browser MSE decodes AC3/EAC3, which IPTV TS streams commonly use.
//! The server then serves the result (`/stream/ts/{id}/index.m3u8` and
//! `/stream/ts/{id}/seg/{name}`). Idle sessions are killed by the ticker
//! spawned in `super::spawn_server_ticker`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
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
/// recently used session is evicted (dead sessions first).
const MAX_SESSIONS: usize = 6;

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
    /// A writable directory we can stage a copy of the Android ffmpeg
    /// sidecar into (Android's extracted `lib/` path can't be executed in
    /// place). On desktop this is unused for staging but kept harmless.
    staging_dir: PathBuf,
}

impl SessionStore {
    pub fn new(staging_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            errors: Arc::new(Mutex::new(HashMap::new())),
            spawn_errors: Mutex::new(HashMap::new()),
            staging_dir,
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

    /// Ensures an ffmpeg session exists for `id`, spawning one if needed
    /// (evicting a dead session first, then the least recently used one,
    /// when at capacity). Never holds a lock across an `.await`.
    ///
    /// `kind` distinguishes live streams from VOD/episode files. Files are
    /// remuxed as a complete VOD playlist (`-hls_playlist_type vod`, no
    /// sliding window, no segment deletion) paced with `-re`; when ffmpeg
    /// finishes at EOF the session keeps serving the finished package
    /// instead of respawning from the start.
    pub async fn ensure_session(&self, id: u64, url: &str, kind: &str) -> Result<(), String> {
        let vod = kind == "vod" || kind == "episode";
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
                        // Dead or unknown state.
                        if vod && manifest_has_endlist(&existing.dir) {
                            // A VOD file remuxed to completion: keep the
                            // session around so the finished package keeps
                            // being served; the ticker reaps it once the
                            // player stops requesting it.
                            existing.last_access = Instant::now();
                            return Ok(());
                        }
                        let tail = self.stderr_tail_of(id);
                        displaced = inner.remove(&id);
                        if vod {
                            // Dead before the playlist completed: a real
                            // failure (bad URL, missing decoder, panel 403,
                            // ...). Stop respawning — a respawn would reset
                            // the movie to 0:00 anyway — and surface why.
                            let msg = if tail.is_empty() {
                                "ffmpeg exited before the stream was remuxed (no error output captured)"
                                    .to_string()
                            } else {
                                format!("ffmpeg exited before the stream was remuxed:\n{tail}")
                            };
                            self.spawn_errors.lock().unwrap().insert(id, msg.clone());
                            return Err(msg);
                        }
                        if !tail.is_empty() {
                            eprintln!("[ffmpeg] session {id} died:\n{tail}");
                        }
                    }
                },
                None => {
                    // New session: evict when at capacity, preferring dead
                    // children over live ones, then least recently used.
                    if inner.len() >= MAX_SESSIONS {
                        // `min_by_key` hands the closure a shared reference,
                        // but `try_wait` needs `&mut` on the child: scan
                        // manually, keeping the (alive, last_access) minimum.
                        let mut victim: Option<(u64, bool, Instant)> = None;
                        for (k, s) in inner.iter_mut() {
                            let alive =
                                s.child.try_wait().map(|w| w.is_none()).unwrap_or(true);
                            if victim
                                .map(|(_, a, t)| (a, t) > (alive, s.last_access))
                                .unwrap_or(true)
                            {
                                victim = Some((*k, alive, s.last_access));
                            }
                        }
                        if let Some((victim_id, _, _)) = victim {
                            displaced = inner.remove(&victim_id);
                            self.spawn_errors.lock().unwrap().remove(&victim_id);
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

        let bin = match ffmpeg_bin(&self.staging_dir) {
            Some(bin) => bin,
            None => {
                let msg =
                    "ffmpeg unavailable — install ffmpeg or set ANDROIPTV_FFMPEG".to_string();
                self.spawn_errors.lock().unwrap().insert(id, msg.clone());
                return Err(msg);
            }
        };

        let mut command = Command::new(bin);
        // `-re` must precede `-i`: it paces the *input* at real time, so
        // the playlist edge tracks the movie instead of racing ahead of
        // the player (which would make VOD effectively start midway).
        if vod {
            command.arg("-re");
        }
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-user_agent",
                "AndroIPTV/0.1.0",
                "-i",
                url,
                // Explicit stream selection: first video + first audio, both
                // optional (audio-only radio channels must still work). With
                // no -map, ffmpeg auto-selects *every* stream, and a subtitle
                // track (webvtt/srt/ass — very common on IPTV panels) cannot
                // be muxed into the HLS output: the webvtt encoder fails and
                // ffmpeg aborts with "Nothing was written into output file",
                // killing the whole remux session. -sn drops subtitles.
                "-map",
                "0:v:0?",
                "-map",
                "0:a:0?",
                "-sn",
                // Video stays zero-encode; audio is re-encoded to AAC because
                // no browser MSE decodes AC3/EAC3 (common in IPTV streams).
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-ac",
                "2",
                "-b:a",
                "128k",
                "-f",
                "hls",
            ]);
        if vod {
            // VOD: one complete playlist (no sliding window, nothing
            // deleted), terminated with #EXT-X-ENDLIST at EOF so hls.js
            // treats it as a file and starts at 0:00.
            command.args([
                "-hls_time", "6",
                "-hls_list_size", "0",
                "-hls_playlist_type", "vod",
                "-hls_flags", "independent_segments",
            ]);
        } else {
            // Live: a small sliding window; segments are deleted as they
            // fall out so a long-running session never fills the disk.
            command.args([
                "-hls_time", "4",
                "-hls_list_size", "8",
                "-hls_flags", "delete_segments+independent_segments",
            ]);
        }
        command
            .arg("-hls_segment_filename")
            .arg(dir.join("seg_%05d.ts"))
            .arg(dir.join("index.m3u8"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
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

    /// Last captured stderr lines for a session (most recent last), for
    /// surfacing ffmpeg failures in HTTP responses and the log.
    pub fn stderr_tail_of(&self, id: u64) -> String {
        let ring = match self.errors.lock() {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        match ring.get(&id) {
            Some(queue) => queue.iter().cloned().collect::<Vec<_>>().join("\n"),
            None => String::new(),
        }
    }
}

/// True if the session's playlist was completed with `#EXT-X-ENDLIST`
/// (ffmpeg writes it at EOF for `-hls_playlist_type vod` remuxes).
fn manifest_has_endlist(dir: &PathBuf) -> bool {
    match std::fs::read(dir.join("index.m3u8")) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).contains("#EXT-X-ENDLIST"),
        Err(_) => false,
    }
}

/// Resolves a runnable ffmpeg binary, in order: the `ANDROIPTV_FFMPEG` env
/// var, the Tauri sidecar next to the main executable (staged into
/// `staging_dir` on Android so it can actually run), then plain `ffmpeg` on
/// PATH.
///
/// Tauri names external binaries by target triple: `ffmpeg-<triple>` on
/// desktop, `libffmpeg-<triple>.so` on Android (the `lib` prefix and `.so`
/// suffix let the APK package a plain executable as a native library; the
/// installer extracts it beside the app's own libs, which is what
/// `current_exe()` points at).
///
/// Android detail: the extracted native-lib path isn't freely executable in
/// place (W^X / noexec on the APK-extracted `lib/` dir), so the sidecar is
/// copied into our writable app dir and chmod +x'd, then run from there.
/// The staged path is cached so the copy happens once per process.
fn ffmpeg_bin(staging_dir: &PathBuf) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ANDROIPTV_FFMPEG") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Some(staged) = staged_sidecar(staging_dir) {
        return Some(staged);
    }

    // Plain `ffmpeg` / PATH fallback (`is_file` on a bare name can't see
    // PATH).
    let plain = PathBuf::from("ffmpeg");
    if plain.is_file() {
        return Some(plain);
    }
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

/// Finds the Tauri ffmpeg sidecar beside the current executable and returns
/// a runnable path to it. On desktop this is the sidecar itself (already
/// executable in place); on Android it is a staged copy in `staging_dir`.
/// The result is cached in a per-process [`OnceLock`] so the Android copy
/// runs at most once.
fn staged_sidecar(staging_dir: &PathBuf) -> Option<PathBuf> {
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))]
    let staging_dir = staging_dir;
    static STAGED: OnceLock<Option<PathBuf>> = OnceLock::new();
    STAGED
        .get_or_init(|| {
            let src = locate_sidecar()?;
            #[cfg(target_os = "android")]
            return stage_android_sidecar(&src, staging_dir);
            #[cfg(not(target_os = "android"))]
            Some(src)
        })
        .clone()
}

/// Looks for the externalBin sidecar next to the current executable, trying
/// the desktop and Android (native-lib) names for the build's target triple.
fn locate_sidecar() -> Option<PathBuf> {
    let parent = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // The target triple (emitted by build.rs) matches the externalBin name
    // for the running build, on every ABI.
    let triple = env!("ANDROIPTV_TARGET");
    for name in [
        format!("ffmpeg-{triple}"),
        format!("libffmpeg-{triple}.so"),
        "ffmpeg".to_string(),
    ] {
        // `mut` is only used on Windows (set_extension); silence the
        // unused_mut warning on other platforms.
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut candidate = parent.join(&name);
        #[cfg(windows)]
        {
            candidate.set_extension("exe");
        }
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Copies the Android sidecar into a writable, executable location and
/// chmods it so `Command::new` can actually run it.
#[cfg(target_os = "android")]
fn stage_android_sidecar(src: &PathBuf, staging_dir: &PathBuf) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let Ok(_) = std::fs::create_dir_all(staging_dir) else {
        return None;
    };
    let file_name = src.file_name()?;
    let dest = staging_dir.join(file_name);
    // Overwrite so a stale/corrupt copy is replaced.
    std::fs::copy(src, &dest).ok()?;
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).ok()?;
    Some(dest)
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

/// Resolves the upstream URL and kind for a stream by namespace id.
/// Episode ids (>= EPISODE_ID_OFFSET) map back to the episodes table;
/// smaller ids are channels. The kind drives the remux mode: "vod" and
/// "episode" get the file behavior (complete playlist, `-re`, finished
/// sessions), everything else the live sliding window.
fn lookup_stream(st: &ServerState, id: u64) -> Result<(String, String), Response> {
    if id >= super::EPISODE_ID_OFFSET {
        match st.db.get_episode((id - super::EPISODE_ID_OFFSET) as i64) {
            Ok(Some(ep)) => Ok((ep.url, "episode".to_string())),
            Ok(None) => Err(
                (StatusCode::NOT_FOUND, Json(json!({"error": "episode not found"}))).into_response(),
            ),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()),
        }
    } else {
        let channel = lookup_channel(st, id)?;
        Ok((channel.url, channel.kind))
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
    let (url, kind) = match lookup_stream(&st, id) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    if let Err(e) = st.sessions.ensure_session(id, &url, &kind).await {
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The session is still starting: refresh the access time so the
            // ticker never kills a slow-starting session mid-boot, and let
            // the player keep polling. The stderr tail is included because
            // every ffmpeg failure mode (missing decoder, panel 403, bad
            // URL) looks identical to a slow start without it.
            st.sessions.touch(id);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "starting",
                    "stderr": st.sessions.stderr_tail_of(id),
                })),
            )
                .into_response()
        }
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
    let (url, kind) = match lookup_stream(&st, id) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    if let Err(e) = st.sessions.ensure_session(id, &url, &kind).await {
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
