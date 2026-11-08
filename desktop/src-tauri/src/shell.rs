//! The app stays running with its window closed: closing hides the window,
//! and connections, presence and discovery continue until the user quits
//! from the app menu, the status icon or Cmd+Q. Open at login starts the app
//! the same way, with only its status icon.
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

/// What the login item passes, so the app starts without its window.
pub const AUTOSTART: &str = "--autostart";

/// This launch came from the login item.
pub fn started_at_login() -> bool {
    launched_at_login(std::env::args().skip(1))
}
fn launched_at_login(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == AUTOSTART)
}

/// The login item is a launch agent on macOS and a Run entry on Windows,
/// named after the bundle identifier so the development build has its own.
pub fn autostart_plugin(identifier: &str) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_autostart::Builder::new()
        .app_name(identifier)
        .arg(AUTOSTART)
        .build()
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

async fn blocking<T: Send + 'static>(
    failure: &str,
    f: impl FnOnce() -> Result<T, tauri_plugin_autostart::Error> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{failure} {e}"))
}
#[tauri::command]
pub async fn open_at_login(app: AppHandle) -> Result<bool, String> {
    blocking("Couldn’t check Open at login.", move || {
        app.autolaunch().is_enabled()
    })
    .await
}
#[tauri::command]
pub async fn set_open_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    blocking("Couldn’t change Open at login.", move || {
        let login = app.autolaunch();
        if enabled {
            login.enable()?;
        } else {
            login.disable()?;
        }
        login.is_enabled()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_login_item_starts_hidden() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert!(launched_at_login(args(&[AUTOSTART]).into_iter()));
        assert!(!launched_at_login(args(&[]).into_iter()));
        assert!(!launched_at_login(
            args(&["--display-benchmark", "peer"]).into_iter()
        ));
    }
}
