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
use android::NativeBack;
pub use error::{Error, Result};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use fallback::NativeBack;
#[cfg(target_os = "ios")]
use ios::NativeBack;
pub use models::{NativeBackStatus, SetEnabledRequest};

trait NativeBackExt<R: Runtime> {
    fn native_back(&self) -> &NativeBack<R>;
}

impl<R: Runtime, T: Manager<R>> NativeBackExt<R> for T {
    fn native_back(&self) -> &NativeBack<R> {
        self.state::<NativeBack<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("native-back")
        .invoke_handler(tauri::generate_handler![
            commands::complete,
            commands::pop,
            commands::prepare,
            commands::set_enabled,
            commands::status,
        ])
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let native_back = android::init(app, api)?;
            #[cfg(target_os = "ios")]
            let native_back = ios::init(app, api)?;
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let native_back = fallback::init(app, api)?;
            app.manage(native_back);
            Ok(())
        })
        .build()
}
