//! Chromecast support (Android only).
//!
//! Google's web-sender JS SDK does not run inside the Android WebView, so
//! device discovery, session management and media loading happen natively
//! in Kotlin (`android/src/main/java/dev/androiptv/cast/`) via the
//! AndroidX Cast SDK. The UI drives it with `plugin:cast|*` invokes
//! (`is-available`, `connect`, `load`, `disconnect`, `state`); on desktop
//! the plugin registers nothing and the UI hides the cast button.

use tauri::{
  plugin::{Builder, TauriPlugin},
  Runtime,
};

#[cfg(mobile)]
mod mobile;

/// Initializes the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
  Builder::new("cast")
    .setup(|app, api| {
      #[cfg(mobile)]
      mobile::init(app, api)?;
      Ok(())
    })
    .build()
}
