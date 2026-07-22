include!("src/app_commands.rs");

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

macro_rules! command_names {
    ($($command:ident),+ $(,)?) => {
        &[$(stringify!($command)),+]
    };
}

const APP_COMMANDS: &[&str] = with_lua_limits_app_commands!(command_names);

fn bundle_vendored_lua_for_ios_staticlib() {
    if env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some(OsStr::new("ios")) {
        return;
    }

    let lua_lib_dir = PathBuf::from(
        env::var_os("DEP_LUA_LIB").expect("mlua-sys must expose its vendored Lua library path"),
    );
    let source_archive = lua_lib_dir.join("liblua5.4.a");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));
    let bundled_archive = out_dir.join("liblorepia_lua54_ios_bundle.a");
    fs::copy(&source_archive, &bundled_archive)
        .expect("failed to stage vendored Lua 5.4 for the iOS staticlib");

    println!("cargo:rerun-if-changed={}", source_archive.display());
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static:+bundle=lorepia_lua54_ios_bundle");
}

fn main() {
    bundle_vendored_lua_for_ios_staticlib();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to build LorePia Lua-limits command permissions");
}
