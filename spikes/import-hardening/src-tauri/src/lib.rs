mod probe;

include!("app_commands.rs");

macro_rules! generate_import_hardening_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[tauri::command]
async fn run_import_hardening_m1_probe(
    app: tauri::AppHandle,
) -> Result<probe::ProbeReceipt, probe::ProbeError> {
    use tauri::Manager;

    let app_local_directory = app
        .path()
        .app_local_data_dir()
        .map_err(|_| probe::path_unavailable_error())?;

    tauri::async_runtime::spawn_blocking(move || {
        probe::with_process_lock(|| probe::run_probe_in_directory(&app_local_directory))
    })
    .await
    .unwrap_or_else(|_| Err(probe::internal_state_error()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(with_import_hardening_app_commands!(
            generate_import_hardening_handler
        ))
        .run(tauri::generate_context!())
        .expect("failed to run LorePia import-hardening spike");
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
        const COMMANDS: &[&str] = with_import_hardening_app_commands!(command_names);
        assert_eq!(COMMANDS, &["run_import_hardening_m1_probe"]);
    }
}
