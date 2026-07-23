const COMMANDS: &[&str] = &["complete", "pop", "prepare", "set_enabled", "status"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).ios_path("ios").build();
}
