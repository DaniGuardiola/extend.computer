use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tauri::Manager;
pub mod passkey;
pub mod traffic_lights;

pub fn initialize(app: &tauri::AppHandle) -> Result<()> {
    let resources = crate::permission_flow::resources(app)?;
    crate::wifi_permission::initialize(&resources)?;
    let executable = std::env::current_exe()?;
    let contents = executable
        .parent()
        .and_then(Path::parent)
        .context("Missing app bundle")?;
    extend_computer_agent::low_jitter::configure_app_helper(
        contents.join("Library/LaunchServices/ExtendComputerLowJitter"),
    )
}
fn bundled(helpers: &Path, name: &str) -> PathBuf {
    helpers.join(format!(
        "extend.computer {name}.app/Contents/MacOS/ExtendComputer{name}"
    ))
}
pub fn cursor_helper(app: &tauri::AppHandle) -> Result<PathBuf> {
    let packaged = bundled(&app.path().resource_dir()?.join("helpers"), "Cursor");
    #[cfg(debug_assertions)]
    if !packaged.is_file() {
        return Ok(bundled(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
            "Cursor",
        ));
    }
    Ok(packaged)
}
/// The directory holding the helper apps, given the path of one of them.
pub fn helpers(helper: &Path) -> Result<&Path> {
    helper
        .ancestors()
        .nth(4)
        .context("Missing native helper directory")
}
/// The helper app `name` installed beside the helper at `beside`.
pub fn helper(beside: &Path, name: &str) -> Result<PathBuf> {
    Ok(bundled(helpers(beside)?, name))
}
pub fn computer_name() -> Option<String> {
    std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
}
// Keep realtime scheduling active while the app is in the background, where
// App Nap would otherwise throttle input and display relaying. The activity
// ends with the session; it does not change system power settings.
pub fn session_activity() -> impl Sized {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    struct Activity<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Activity<F> {
        fn drop(&mut self) {
            if let Some(end) = self.0.take() {
                end();
            }
        }
    }
    let process = NSProcessInfo::processInfo();
    let activity = process.beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical,
        &NSString::from_str("Device connection"),
    );
    Activity(Some(move || unsafe { process.endActivity(&activity) }))
}
