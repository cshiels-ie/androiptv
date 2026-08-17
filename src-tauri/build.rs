fn main() {
    tauri_build::build();
    // Expose the rustc target triple to the crate at compile time: the
    // runtime ffmpeg sidecar lookup (server/ffmpeg.rs) needs the exact
    // externalBin name, which is target-triple-based.
    println!(
        "cargo:rustc-env=ANDROIPTV_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
