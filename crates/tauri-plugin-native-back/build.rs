const COMMANDS: &[&str] = &["complete", "pop", "prepare", "set_enabled", "status"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
