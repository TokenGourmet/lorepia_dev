use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(target_os = "android")]
mod android;
mod commands;
mod error;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod fallback;
#[cfg(target_os = "ios")]
mod ios;
mod models;

#[cfg(target_os = "android")]
use android::NativeChrome;
pub use error::{Error, Result};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use fallback::NativeChrome;
#[cfg(target_os = "ios")]
use ios::NativeChrome;
pub use models::{NativeChromeAppearance, NativeChromeState, NativeChromeStatus, NativeChromeTab};

trait NativeChromeExt<R: Runtime> {
    fn native_chrome(&self) -> &NativeChrome<R>;
}

impl<R: Runtime, T: Manager<R>> NativeChromeExt<R> for T {
    fn native_chrome(&self) -> &NativeChrome<R> {
        self.state::<NativeChrome<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("native-chrome")
        .invoke_handler(tauri::generate_handler![
            commands::set_state,
            commands::status,
        ])
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let native_chrome = android::init(app, api)?;
            #[cfg(target_os = "ios")]
            let native_chrome = ios::init(app, api)?;
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let native_chrome = fallback::init(app, api)?;
            app.manage(native_chrome);
            Ok(())
        })
        .build()
}
