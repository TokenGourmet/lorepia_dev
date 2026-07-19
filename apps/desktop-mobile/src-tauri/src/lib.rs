use lorepia_core::{LorePiaCore, ProductBootstrap};
use tauri::State;

include!("app_commands.rs");

macro_rules! generate_product_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[tauri::command]
fn get_product_bootstrap(core: State<'_, LorePiaCore>) -> ProductBootstrap {
    core.product_bootstrap()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(LorePiaCore::new())
        .invoke_handler(with_product_app_commands!(generate_product_handler))
        .run(tauri::generate_context!())
        .expect("failed to run LorePia");
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
        const COMMANDS: &[&str] = with_product_app_commands!(command_names);
        assert_eq!(COMMANDS, &["get_product_bootstrap"]);
    }
}
