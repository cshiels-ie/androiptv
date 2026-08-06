# src-tauri/binaries

Tauri `externalBin` executables, named `ffmpeg-<target-triple>` (with a `.exe` suffix on Windows) — e.g. `ffmpeg-x86_64-unknown-linux-gnu`. `tauri.conf.json` references them via `"externalBin": ["binaries/ffmpeg"]`.

- **Desktop:** run `node scripts/fetch-ffmpeg.mjs` to download the current platform's static build.
- **Android:** build static executables once with [ffmpeg-android-maker](https://github.com/Javernaut/ffmpeg-android-maker) and place them here as `ffmpeg-aarch64-linux-android` (plus the other ABIs you ship).
