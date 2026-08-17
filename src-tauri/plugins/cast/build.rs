const COMMANDS: &[&str] = &[
  "is-available",
  "connect",
  "load",
  "disconnect",
  "state",
];

fn main() {
  tauri_plugin::Builder::new(COMMANDS)
    .android_path("android")
    .build();
}
