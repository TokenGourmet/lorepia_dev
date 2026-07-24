use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{NativeChromeState, NativeChromeStatus, Result};

pub(crate) fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<NativeChrome<R>> {
    Ok(NativeChrome(PhantomData))
}

pub struct NativeChrome<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> NativeChrome<R> {
    pub(crate) fn set_state(&self, _payload: NativeChromeState) -> Result<NativeChromeStatus> {
        Ok(NativeChromeStatus::default())
    }

    pub(crate) fn status(&self) -> Result<NativeChromeStatus> {
        Ok(NativeChromeStatus::default())
    }
}
