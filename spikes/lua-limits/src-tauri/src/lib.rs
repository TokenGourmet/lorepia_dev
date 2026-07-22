mod probe;

include!("app_commands.rs");

macro_rules! generate_lua_limits_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[tauri::command]
async fn run_lua_limits_m1_probe() -> Result<probe::ProbeReceipt, probe::ProbeError> {
    tauri::async_runtime::spawn_blocking(probe::run_probe)
        .await
        .unwrap_or_else(|_| Err(probe::internal_state_error()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(with_lua_limits_app_commands!(generate_lua_limits_handler))
        .run(tauri::generate_context!())
        .expect("failed to run LorePia Lua-limits spike");
}

#[cfg(test)]
mod command_surface_tests {
    macro_rules! command_names {
        ($($command:ident),+ $(,)?) => {
            &[$(stringify!($command)),+]
        };
    }

    #[test]
    fn native_command_surface_is_exact() {
        const COMMANDS: &[&str] = with_lua_limits_app_commands!(command_names);
        assert_eq!(COMMANDS, &["run_lua_limits_m1_probe"]);
    }
}
