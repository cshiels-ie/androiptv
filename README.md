# AndroIPTV

A cross-platform IPTV app for **Desktop (Windows / macOS / Linux)** and **Android**, built with
[Tauri v2](https://v2.tauri.app), a React + TypeScript frontend, and an embedded axum LAN server
that lets a Smart TV browser (e.g. Samsung Internet) open `http://<host-ip>:4040` to browse
channels and stream them **without casting** — no Chromecast, no DLNA, no extra app on the TV.

## Features

- **M3U / M3U8 + Xtream Codes import** — playlists are parsed locally and stored in SQLite
  (bundled rusqlite; no external DB).
- **Channel groups & search** — browse playlists by group, search across channels.
- **In-app playback** — the desktop app plays HLS streams with [hls.js](https://github.com/video-dev/hls.js).
- **Embedded TV server** — a small axum server inside the app serves the same-origin TV page
  (channel list, groups, search, playback) to any browser on the LAN; a QR code in the app makes
  pairing the TV a one-scan step.
- **HLS passthrough proxy** — channel HLS streams are proxied through the embedded server with
  URL rewriting, so the TV browser never sees origin/CORS problems.
- **Raw-TS → HLS remux** — raw `.ts` channels are remuxed into HLS on the fly with the bundled
  ffmpeg using `-c copy` (zero re-encode, near-zero CPU).

## Architecture

```
┌──────────────────────────── Tauri app ────────────────────────────┐
│ React desktop UI (src/)          TV page (tv/ → single-file HTML) │
│        │ invoke() commands                │ served same-origin    │
│        ▼                                 ▼                         │
│  Rust backend (src-tauri/src) ── axum server (0.0.0.0:4040)       │
│  ─ db     : SQLite storage (playlists, groups, channels)          │
│  ─ m3u    : M3U/M3U8 playlist parser                              │
│  ─ xtream : Xtream Codes API import                               │
│  ─ server : embedded LAN server (status, groups, channels, play)  │
│     ├─ hls_proxy : HLS passthrough proxy with URL rewriting       │
│     └─ ffmpeg    : raw-TS → HLS remux (`-c copy`)                 │
└───────────────────────────────────────────────────────────────────┘
```

- The **TV page** is built as a single self-contained HTML file (all assets inlined, no external
  requests) and embedded into the Rust binary via `include_str!`, then served by the embedded
  server. Because the TV browser talks to the server **same-origin**, there is no CORS setup.
- The embedded server binds `0.0.0.0` and uses port **4040** (with automatic fallback if busy);
  the active URL + QR code are shown in the app's **"TV Server"** tab.
- The backend (`src-tauri/src/`) ships `db.rs` (SQLite), `m3u.rs` (M3U parser), `xtream.rs`
  (Xtream Codes API), `net.rs` (LAN IP detection), `commands.rs` (Tauri commands) and `server/`
  (`mod.rs` router, `hls_proxy.rs`, `ffmpeg.rs`).

## Prerequisites

- **Node.js 20+**
- **Rust 1.78+** via [rustup](https://rustup.rs)
- **Linux only** — system libraries:
  ```sh
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
                   libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```
- **Tauri CLI** — either
  `cargo install tauri-cli --version '^2'` or use the npm package
  `@tauri-apps/cli` (already a devDependency, invoked as `npm run tauri`).

## Build & run

```sh
npm install
npm run tv:build     # builds the TV page → dist-tv → copies into src-tauri/resources/tv/
npm run tauri dev    # desktop dev app
```

- **Desktop release:** `npm run tauri build`
- **ffmpeg:** `node scripts/fetch-ffmpeg.mjs` downloads the static ffmpeg binary for your current
  platform into `src-tauri/binaries/` (desktop only — Android needs a static build made once with
  ffmpeg-android-maker, see below). Skip it to develop without remux support; raw-TS channels
  simply won't play.
- **Icons:** `scripts/gen-icons.mjs` regenerates the play-triangle icons in `src-tauri/icons/`
  (dependency-free pure-Node PNG/ICO writer). Run `npx tauri icon <source.png>` once to scaffold
  the full icon set (tauri icon overwrites `src-tauri/icons/`), then re-run `gen-icons.mjs` to
  restore the design.

## CI: Android APK builds (recommended)

A [GitHub Actions workflow](.github/workflows/build-android.yml) builds the arm64-v8a APK in the
cloud — nothing heavy ever compiles on your device:

- **Trigger:** push to `main`, a `v*` tag, or manually (Actions → "Build Android APK" → Run workflow — you can pick the ABI(s) and APK vs AAB).
- **What it does:** `npm ci` → builds the TV bundle → compiles a minimal static Android ffmpeg
  (TS→HLS remux uses only `-c copy`, so no codecs are needed) → `tauri android init` → patches
  `AndroidManifest.xml` (INTERNET + cleartext HTTP) → `tauri android build --apk --target aarch64`.
- **Artifact:** the APK is attached to the run's summary ("Artifacts" dropdown).
- **Note:** the CI ffmpeg build omits a TLS backend, so raw-TS channels over `https` won't remux;
  HLS channels are unaffected.

Local Android builds still work (below) if you prefer to run Gradle yourself.

## CI: Desktop bundles

`.github/workflows/build-desktop.yml` builds the desktop installers on GitHub's
runners (push to `main`, `v*` tags, or manual run):

- **Linux** (ubuntu-22.04) → `.deb` / `.rpm` / `.AppImage`
- **Windows** → `.msi` / `.exe` (NSIS)
- **macOS** → `.dmg` / `.app`

Each job runs `npm ci`, downloads the platform's ffmpeg sidecar
(`node scripts/fetch-ffmpeg.mjs`), then `npm run tauri build`; bundles are
attached to the run's summary as artifacts. Manual runs can pick the
platform(s) and bundle type(s) (e.g. `deb,appimage`).

## Android

1. **Install the Android SDK** (Android Studio or command line):
   - SDK Platform **API 34**, Build Tools **34.0.0**, NDK **26.1.10909125** (Tauri supports NDK 25–26 only), JDK **17**.
2. **Set environment variables:**
   ```sh
   export ANDROID_HOME=$HOME/Android/Sdk
   export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/26.1.10909125
   export JAVA_HOME=/path/to/jdk-17
   ```
3. **Add Rust targets:**
   ```sh
   rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
   ```
4. **Initialize the Android project:**
   ```sh
   npm run tauri android init
   ```
5. **Manual edits after init** (Tauri preserves these on rebuilds) in
   `src-tauri/gen/android/app/src/main/AndroidManifest.xml`:
   - add `android:usesCleartextTraffic="true"` to the `<application>` element — required so the
     in-app WebView can call the local HTTP server (`http://<host-ip>:4040`);
   - make sure `<uses-permission android:name="android.permission.INTERNET"/>` is present.
6. **ffmpeg (optional but recommended):** the CI workflow builds the aarch64 static executable
   automatically from source. For local builds, use
   [ffmpeg-android-maker](https://github.com/Javernaut/ffmpeg-android-maker) once and place the
   result at `src-tauri/binaries/ffmpeg-aarch64-linux-android` (and per the other ABIs you ship).
   If ffmpeg is missing, the app still works — only raw-TS channels won't play on the TV (HLS
   channels are unaffected).
7. **Build:**
   ```sh
   npm run tauri android build -- --apk
   ```
   The debug APK can be sideloaded directly; release builds additionally need a signing keystore.

## Using the TV feature

1. Run the app on a machine on the **same Wi-Fi** as the TV.
2. Open the **"TV Server"** tab in the app and point the TV browser (Samsung Internet, Chrome, Safari…)
   at the shown URL — or scan the QR code.
3. Browse groups/channels, search, and press play. HLS streams play via the HLS passthrough proxy;
   raw-TS channels are remuxed on the fly by the bundled ffmpeg (`-c copy`).
4. **Windows firewall:** if the TV can't reach the app, allow inbound connections on **Private**
   networks for the app in Windows Defender Firewall.

## Project layout

```
androiptv/
├── src/                        # React desktop UI (Vite)
│   ├── components/             # ChannelList, Player, PlaylistManager, QrPanel, SearchBar
│   ├── pages/                  # Home, Channels, PlayerView, TvCast
│   ├── services/               # api.ts (Tauri command wrappers), types.ts
│   ├── App.tsx · main.tsx · styles/app.css
├── tv/                         # standalone TV page source (own Vite build → dist-tv)
│   ├── index.html · main.ts · api.ts · player.ts · styles.css
├── scripts/
│   ├── gen-icons.mjs           # dependency-free PNG/ICO icon generator
│   ├── copy-tv.mjs             # inlines dist-tv JS/CSS into index.html → src-tauri/resources/tv/
│   └── fetch-ffmpeg.mjs        # downloads the current platform's static ffmpeg
├── src-tauri/
│   ├── src/                    # Rust backend: db, m3u, xtream, net, commands, server/
│   ├── resources/tv/           # embedded TV bundle (include_str!)
│   ├── binaries/               # ffmpeg-<target-triple>[.exe] (externalBin)
│   ├── icons/                  # generated app icons
│   ├── capabilities/ · Cargo.toml · tauri.conf.json · build.rs
├── index.html · package.json · tsconfig.json · vite.config.ts · vite.tv.config.ts
```

## License

MIT
