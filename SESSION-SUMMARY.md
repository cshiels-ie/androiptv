# AndroIPTV — Session Summary (2026-08-06)

## Status

Pushed to private repo: **`cshiels-ie/androiptv`** → https://github.com/cshiels-ie/androiptv
(59 files, initial commit on `main`).

## Audits

Three parallel read-only agents audited the codebase:

- **Backend**: ~90% feature-complete, all 9 Tauri commands registered, LAN server wired into the
  setup hook — but the crate **had never compiled**.
- **Frontend**: zero invoke/command mismatches, all imports resolve.
- **Configs**: TV build broken, `tauri` shim missing, Android packaging absent.

## Fixes applied (code only — no heavy builds on-device)

| Severity | Issue | Fix |
|---|---|---|
| 🔴 Blocker | `db.rs` borrow/move bug (crate never compiled) | Rewrote `import_channels` — each batch gets its own transaction + statement; group count now accurate |
| 🔴 Blocker | TV build couldn't resolve its entry (`vite.tv.config.ts`) | Added `root: "tv"` and fixed `outDir` / input paths |
| 🔴 Blocker | Embedded TV page was a 117-byte placeholder | TV bundle now builds (**verified: 532 KB self-contained, zero external requests**); `copy-tv.mjs` inlines JS/CSS into the HTML |
| 🟠 High | No CORS headers on LAN server → in-app playback blocked | Cross-origin headers middleware added to the axum router |
| 🟠 High | `npm run tauri` broken (scoped packages skipped by shim script) | `make-bins.mjs` now descends into `@scope` package dirs |
| 🟡 Medium | Player / hls.js issues | `Hls.Events.ERROR` handler in `Player.tsx`; play-on-`MANIFEST_PARSED` in `tv/player.ts`; `r.ok` check in `PlayerView.tsx`; `get_channel` return type fixed; dead code removed |
| 🟡 Low | Config hygiene | Unused Cargo deps removed; `android.minSdkVersion` added to `tauri.conf.json`; `tv/` now included in type-checking (tsc passes clean) |

## CI: Android APK builds (GitHub Actions)

`.github/workflows/build-android.yml` builds the APK **in the cloud**:

1. `npm ci`
2. Build the TV bundle
3. **Compile a minimal static Android ffmpeg from source** (only what `-c copy` TS→HLS remux
   needs — no codecs)
4. `tauri android init`
5. Patch `AndroidManifest.xml` (INTERNET permission + cleartext HTTP)
6. `tauri android build --apk --target aarch64`
7. Upload the APK as a run artifact

- Triggered on push to `main`, `v*` tags, or manually (Actions tab).
- First run downloads Gradle/NDK: expect ~15–25 min; later runs are faster.
- **Known limitation**: the minimal ffmpeg has no TLS backend, so raw-TS channels over `https`
  won't remux (HLS channels unaffected).

## Verified on-device (all lightweight, node-based)

- `tsc --noEmit` — clean across `src/` and `tv/`
- `npm run tv:build` — bundle builds and inlines correctly
- `npm run build` (tsc + vite build + tv:build) — full frontend pipeline green

## Next steps

- Watch the first CI run; fix anything it surfaces
- Sideload the APK on a phone and test the TV-server flow
- Desktop builds (`tauri build`) should run on a real desktop machine
