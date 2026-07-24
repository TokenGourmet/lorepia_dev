use serde::de::DeserializeOwned;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{NativeBackStatus, Result, SetEnabledRequest};

const PLUGIN_IDENTIFIER: &str = "dev.lorepia.nativeback";

pub(crate) fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<NativeBack<R>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "NativeBackPlugin")?;
    Ok(NativeBack(handle))
}

pub struct NativeBack<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> NativeBack<R> {
    pub(crate) fn complete(&self) -> Result<NativeBackStatus> {
        self.0.run_mobile_plugin("complete", ()).map_err(Into::into)
    }

    pub(crate) fn pop(&self) -> Result<NativeBackStatus> {
        self.0.run_mobile_plugin("pop", ()).map_err(Into::into)
    }

    pub(crate) fn prepare(&self) -> Result<NativeBackStatus> {
        self.0.run_mobile_plugin("prepare", ()).map_err(Into::into)
    }

    pub(crate) fn set_enabled(&self, payload: SetEnabledRequest) -> Result<NativeBackStatus> {
        self.0
            .run_mobile_plugin("setEnabled", payload)
            .map_err(Into::into)
    }

    pub(crate) fn status(&self) -> Result<NativeBackStatus> {
        self.0.run_mobile_plugin("status", ()).map_err(Into::into)
    }
}
