# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

AndroIPTV is a cross-platform IPTV player (Tauri v2 + React/TS desktop UI, Android via Tauri) with an embedded axum LAN server (`0.0.0.0:4040`, ephemeral fallback) that serves a single-file "TV page" to Smart TV browsers, letting them browse and play channels without any casting protocol. No tests exist for the frontend; the only automated tests are Rust unit tests (`cargo test` in `src-tauri/`).

## Commands

```sh
npm install            # postinstall runs scripts/make-bins.mjs + ensure-esbuild.mjs
npm run dev            # Vite dev server for the desktop UI (port 1420)
npm run tv:build       # build tv/ → dist-tv → inline into src-tauri/resources/tv/index.html
npm run build          # tsc (type-checks src/ AND tv/) + vite build + tv:build
npm run tauri dev      # desktop dev app (Tauri CLI is a devDependency)
npm run tauri build    # desktop release
npm run tauri android build -- --apk   # local Android build (CI is the recommended path)
node scripts/fetch-ffmpeg.mjs          # download static ffmpeg sidecar for the current platform
cargo test             # Rust unit tests, run from src-tauri/ (m3u.rs, net.rs have tests)
```

There is no linter configured; `tsc --noEmit` (or `npm run build`, which runs tsc first) is the frontend correctness gate.

## Architecture

### Two independent frontends, one Rust backend

- **`src/`** — React desktop UI (Vite, `vite.config.ts`). Talks to the backend exclusively through typed `invoke()` wrappers in `src/services/api.ts` (command names match the Rust `#[tauri::command]` handlers in `src-tauri/src/commands.rs` — keep them in sync).
- **`tv/`** — vanilla TypeScript TV page (no framework, own Vite build `vite.tv.config.ts`, targets `es2017` for weak Tizen-era TV browsers). Hash router (`#/channels`, `#/play/<id>`), D-pad/arrow-key focus navigation, hls.js with native-HLS fallback. Talks to the embedded server same-origin via `tv/api.ts`.
- **`src-tauri/src/`** — Rust backend: `db.rs` (SQLite), `m3u.rs` (streaming M3U parser), `xtream.rs` (Xtream Codes API import), `net.rs` (LAN IP detection), `commands.rs` (9 Tauri commands), `server/` (axum LAN server).
- **`src-tauri/plugins/cast/`** — Android-only Chromecast Tauri plugin (native AndroidX Cast SDK; the web-sender JS SDK doesn't run in the Android WebView). Kotlin lives in `plugins/cast/android/src/main/java/dev/androiptv/cast/`; the UI drives it with `plugin:cast|*` invokes (`is-available|connect|load|disconnect|state`) and polls `state` every 2s. It registers nothing on desktop — the cast button in `PlayerView.tsx` hides when the probe invoke rejects. The `tauri-android` gradle project is provisioned by the CLI into the generated host project; the plugin just declares `implementation(project(":tauri-android"))`.

### The TV page pipeline (important invariant)

The TV page is embedded in the Rust binary, so it must be rebuilt before any cargo build:

1. `npm run tv:build` → Vite builds `tv/` into a single-file bundle at `dist-tv/index.html` (everything inlined, zero external requests).
2. `scripts/copy-tv.mjs` inlines the JS/CSS chunks into the HTML and writes it to `src-tauri/resources/tv/index.html`.
3. `server/mod.rs` embeds it via `include_str!("../../resources/tv/index.html")` and serves it at `/`.

`src-tauri/resources/tv/index.html` is a **generated artifact** — edit `tv/`, never that file. Stale TV changes are a classic symptom of skipping step 1.

### The LAN server (`src-tauri/src/server/`)

Runs on Tauri's own async runtime (`tauri::async_runtime` / `spawn_server_ticker`) — never create a second tokio runtime in this crate. Router: `/` (TV page), `/api/status|groups|channels|logo|play/{id}|epg`, and two stream backends:

- **`/proxy/hls/{id}`** (`hls_proxy.rs`) — smart HLS passthrough: fetches the remote playlist (master or media), rewrites every URI to recurse through this same-origin proxy, streams segments/keys untouched with Range support. Detects playlists by content-type, URL suffix, or `#EXTM3U` body marker.
- **`/stream/ts/{id}/...`** (`ffmpeg.rs`) — for non-HLS (raw MPEG-TS) channels: a per-channel ffmpeg subprocess remuxes to a small live HLS package in a temp dir — video copied untouched (`-c:v copy`), audio re-encoded to AAC (no browser MSE decodes AC3/EAC3, which IPTV TS streams commonly use). The first manifest takes ~4s, so hls.js clients retry 503s patiently (see the `manifestLoadingMaxRetry` config in `src/components/Player.tsx` and `tv/player.ts`). Sessions capped at 4 (`MAX_SESSIONS`), idle ones killed by the ticker (60s `IDLE_TIMEOUT`). Segment filenames are strictly validated against path traversal.

Play URL resolution: `/api/play/{id}` in `server/mod.rs` routes `.m3u8` URLs through the HLS proxy; everything else goes through the proxy **with `probe=1`** — it sniffs the upstream (content-type, URL suffix, `#EXTM3U` body marker): HLS playlists are served directly, binary TS gets a 302 to the ffmpeg remuxer (`/stream/ts/{id}/index.m3u8`). This lets Xtream panels that serve HLS at `.ts` URLs skip the ffmpeg session entirely. The `cors_headers` middleware adds `Access-Control-Allow-*` so the desktop webview (origin `tauri://localhost`) can call the server cross-origin; TV browsers are same-origin and unaffected.

### ffmpeg sidecar

Resolved in this order (`ffmpeg.rs::ffmpeg_bin`): `ANDROIPTV_FFMPEG` env var → sidecar next to the executable (Tauri `bundle.externalBin: ["binaries/ffmpeg"]`, named `ffmpeg-<target-triple>` in `src-tauri/binaries/`) → PATH. Missing ffmpeg degrades gracefully: HLS channels work, raw-TS channels return an error with "install ffmpeg or set ANDROIPTV_FFMPEG". Android sidecars are per-ABI, e.g. `ffmpeg-aarch64-linux-android`.

### Database (`db.rs`)

One `rusqlite::Connection` behind a `Mutex` (Connection is Send but not Sync). All DB work runs on the blocking pool (`tauri::async_runtime::spawn_blocking` with a cloned `Arc<Db>`); commands use the `run_db` helper in `commands.rs`. Schema: `playlists` (holds Xtream base/credentials for later URL rebuild), `groups`, `channels` (with `kind` column forward-compatible with EPG/VOD). Imports are batched (`BATCH = 5000`), each batch in its own transaction.

## Environment quirks

- **The filesystem forbids symlinks** (Android storage layer): `.npmrc` sets `bin-links=false` and `scripts/make-bins.mjs` (postinstall) generates regular-file shims in `node_modules/.bin`. Never reintroduce npm bin-links or rely on symlinks.
- **Android manifest patches** (INTERNET permission + `android:usesCleartextTraffic="true"`) are applied by CI after `tauri android init`; the in-app WebView needs cleartext to call the local `http://<host-ip>:4040` server.
- **CI**: `.github/workflows/build-android.yml` builds the arm64 APK and a minimal static Android ffmpeg (no codecs — only `-c copy` remux is needed, and no TLS backend, so https raw-TS won't remux on Android). `.github/workflows/build-desktop.yml` builds desktop bundles on a Linux/Windows/macOS matrix (each job fetches its ffmpeg sidecar via `scripts/fetch-ffmpeg.mjs` before `tauri build`).
- `scripts/gen-icons.mjs` regenerates the play-triangle icons in `src-tauri/icons/`; `npx tauri icon <source.png>` scaffolds the full set first.
- `SESSION-SUMMARY.md` is a dated build-out summary; don't treat it as current documentation.
