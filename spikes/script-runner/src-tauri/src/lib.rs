const NATIVE_COMMANDS: &[&str] = &[];

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    debug_assert!(NATIVE_COMMANDS.is_empty());
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run LorePia script-runner spike");
}

#[cfg(test)]
mod tests {
    use super::NATIVE_COMMANDS;

    #[test]
    fn native_command_surface_is_empty() {
        assert!(NATIVE_COMMANDS.is_empty());
    }
}
