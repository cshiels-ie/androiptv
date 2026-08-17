# Add project specific ProGuard rules here.
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# CastPlugin is resolved reflectively by Tauri's invoke dispatch and
# CastOptionsProvider is referenced from the merged AndroidManifest —
# keep them through release minification.
-keep class dev.androiptv.cast.** { *; }
