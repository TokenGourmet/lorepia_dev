mod credential_commands;
mod provider_stream;
mod storage_commands;

use credential_commands::{
    CredentialVaultState, delete_provider_credential, get_provider_credential_status,
    save_provider_api_key,
};
use lorepia_core::{LorePiaCore, ProductBootstrap};
use provider_stream::{
    ProviderStreamRegistry, ack_provider_stream, cancel_provider_stream,
    get_provider_stream_snapshot, start_provider_stream,
};
use storage_commands::{
    StorageState, create_chat, delete_chat, get_app_preferences, get_storage_status, list_chats,
    load_chat_messages, update_app_preferences,
};
use tauri::{Manager, State};

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
        .manage(CredentialVaultState::default())
        .manage(ProviderStreamRegistry::default())
        .setup(|app| {
            let storage = StorageState::open(app.path().app_local_data_dir());
            app.manage(storage);
            Ok(())
        })
        .invoke_handler(with_product_app_commands!(generate_product_handler))
        .run(tauri::generate_context!())
        .expect("failed to run LorePia");
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_lorepia_client_MainActivity_initNdkContext(
    env: jni::JNIEnv,
    _class: jni::objects::JObject,
    context: jni::objects::JObject,
) {
    use jni::objects::GlobalRef;
    use std::ffi::c_void;
    use std::sync::Mutex;

    // Android keyring initialization may be retried after a transient JNI
    // failure. The retained reference and ndk-context pointer are committed
    // together so the Java Context cannot be collected while Rust uses it.
    static CONTEXT_REFERENCE: Mutex<Option<GlobalRef>> = Mutex::new(None);
    let Ok(mut retained_reference) = CONTEXT_REFERENCE.lock() else {
        return;
    };
    if retained_reference.is_some() {
        return;
    }
    let Ok(reference) = env.new_global_ref(&context) else {
        return;
    };
    let Ok(vm) = env.get_java_vm() else {
        return;
    };
    let vm = vm.get_java_vm_pointer() as *mut c_void;
    unsafe {
        ndk_context::initialize_android_context(vm, reference.as_obj().as_raw() as _);
    }
    *retained_reference = Some(reference);
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
        assert_eq!(
            COMMANDS,
            &[
                "get_product_bootstrap",
                "get_provider_credential_status",
                "save_provider_api_key",
                "delete_provider_credential",
                "start_provider_stream",
                "ack_provider_stream",
                "cancel_provider_stream",
                "get_provider_stream_snapshot",
                "get_storage_status",
                "create_chat",
                "list_chats",
                "load_chat_messages",
                "delete_chat",
                "get_app_preferences",
                "update_app_preferences",
            ]
        );
    }
}
