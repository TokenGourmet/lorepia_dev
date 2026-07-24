use tauri::{AppHandle, Runtime, command};

use crate::{NativeChromeExt, NativeChromeState, NativeChromeStatus, Result};

#[command]
pub(crate) async fn set_state<R: Runtime>(
    app: AppHandle<R>,
    payload: NativeChromeState,
) -> Result<NativeChromeStatus> {
    app.native_chrome().set_state(payload)
}

#[command]
pub(crate) async fn status<R: Runtime>(app: AppHandle<R>) -> Result<NativeChromeStatus> {
    app.native_chrome().status()
}
