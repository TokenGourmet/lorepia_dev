include!("src/app_commands.rs");

macro_rules! command_names {
    ($($command:ident),+ $(,)?) => {
        &[$(stringify!($command)),+]
    };
}

const APP_COMMANDS: &[&str] = with_lua_limits_app_commands!(command_names);

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to build LorePia Lua-limits command permissions");
}
