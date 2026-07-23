use tauri::{AppHandle, Runtime, command};

use crate::{NativeBackExt, NativeBackStatus, Result, SetEnabledRequest};

#[command]
pub(crate) async fn complete<R: Runtime>(app: AppHandle<R>) -> Result<NativeBackStatus> {
    app.native_back().complete()
}

#[command]
pub(crate) async fn pop<R: Runtime>(app: AppHandle<R>) -> Result<NativeBackStatus> {
    app.native_back().pop()
}

#[command]
pub(crate) async fn prepare<R: Runtime>(app: AppHandle<R>) -> Result<NativeBackStatus> {
    app.native_back().prepare()
}

#[command]
pub(crate) async fn set_enabled<R: Runtime>(
    app: AppHandle<R>,
    payload: SetEnabledRequest,
) -> Result<NativeBackStatus> {
    app.native_back().set_enabled(payload)
}

#[command]
pub(crate) async fn status<R: Runtime>(app: AppHandle<R>) -> Result<NativeBackStatus> {
    app.native_back().status()
}
