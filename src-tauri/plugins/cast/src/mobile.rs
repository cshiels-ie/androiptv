//! Android-side binding: registers the Kotlin `CastPlugin` class with the
//! webview so `plugin:cast|*` invokes reach it.

use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

/// Registers `dev.androiptv.cast.CastPlugin`.
pub fn init<R: Runtime, C: DeserializeOwned>(
  _app: &AppHandle<R>,
  api: PluginApi<R, C>,
) -> tauri::Result<()> {
  api.register_android_plugin("dev.androiptv.cast", "CastPlugin")?;
  Ok(())
}
