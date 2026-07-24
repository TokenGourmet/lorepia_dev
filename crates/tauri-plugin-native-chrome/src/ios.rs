use serde::de::DeserializeOwned;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{NativeChromeState, NativeChromeStatus, Result};

tauri::ios_plugin_binding!(init_plugin_native_chrome);

pub(crate) fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<NativeChrome<R>> {
    let handle = api.register_ios_plugin(init_plugin_native_chrome)?;
    Ok(NativeChrome(handle))
}

pub struct NativeChrome<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> NativeChrome<R> {
    pub(crate) fn set_state(&self, payload: NativeChromeState) -> Result<NativeChromeStatus> {
        self.0
            .run_mobile_plugin("setState", payload)
            .map_err(Into::into)
    }

    pub(crate) fn status(&self) -> Result<NativeChromeStatus> {
        self.0.run_mobile_plugin("status", ()).map_err(Into::into)
    }
}
