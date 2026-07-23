use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{NativeBackStatus, Result, SetEnabledRequest};

pub(crate) fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<NativeBack<R>> {
    Ok(NativeBack(PhantomData))
}

pub struct NativeBack<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> NativeBack<R> {
    pub(crate) fn complete(&self) -> Result<NativeBackStatus> {
        Ok(NativeBackStatus::default())
    }

    pub(crate) fn pop(&self) -> Result<NativeBackStatus> {
        Ok(NativeBackStatus::default())
    }

    pub(crate) fn prepare(&self) -> Result<NativeBackStatus> {
        Ok(NativeBackStatus::default())
    }

    pub(crate) fn set_enabled(&self, _payload: SetEnabledRequest) -> Result<NativeBackStatus> {
        Ok(NativeBackStatus::default())
    }

    pub(crate) fn status(&self) -> Result<NativeBackStatus> {
        Ok(NativeBackStatus::default())
    }
}
