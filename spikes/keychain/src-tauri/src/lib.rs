mod backend;
mod probe;

include!("app_commands.rs");

macro_rules! generate_keychain_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[tauri::command]
async fn run_keychain_m1_probe() -> Result<probe::ProbeReceipt, probe::ProbeError> {
    tauri::async_runtime::spawn_blocking(|| {
        probe::with_process_lock(|| {
            let store = backend::platform_store().map_err(probe::probe_error_from_store_failure)?;
            probe::execute_probe(store.as_ref(), &probe::OsRandom)
        })
    })
    .await
    .unwrap_or_else(|_| Err(probe::internal_state_error()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(with_keychain_app_commands!(generate_keychain_handler))
        .run(tauri::generate_context!())
        .expect("failed to run LorePia keychain spike");
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_lorepia_spike_keychain_MainActivity_initNdkContext(
    env: jni::JNIEnv,
    _class: jni::objects::JObject,
    context: jni::objects::JObject,
) {
    use jni::objects::GlobalRef;
    use std::ffi::c_void;
    use std::sync::Mutex;

    // Serialize initialization while allowing a transient JNI lookup failure
    // to be retried. The retained GlobalRef and ndk-context pointer are
    // committed together under the same lock.
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
        const COMMANDS: &[&str] = with_keychain_app_commands!(command_names);
        assert_eq!(COMMANDS, &["run_keychain_m1_probe"]);
    }
}
