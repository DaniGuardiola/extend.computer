//! Windows builds keep their helpers beside the app executable.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn initialize(_: &tauri::AppHandle) -> Result<()> {
    Ok(())
}
pub fn cursor_helper(_: &tauri::AppHandle) -> Result<PathBuf> {
    helper(&std::env::current_exe()?, "Cursor")
}
/// The helper `name` installed beside the helper (or app) at `beside`.
pub fn helper(beside: &Path, name: &str) -> Result<PathBuf> {
    let directory = beside.parent().context("Missing app directory")?;
    Ok(directory.join(format!("ExtendComputer{name}.exe")))
}
/// The host name as the user typed it; `COMPUTERNAME` is always upper case.
pub fn computer_name() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("hostname")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}
pub fn session_activity() -> impl Sized {}

pub mod passkey {
    pub fn available() -> bool {
        false
    }
    pub fn sign_in(
        _: &tauri::AppHandle,
        _: &crate::accounts::Ceremony,
        _: bool,
    ) -> anyhow::Result<serde_json::Value> {
        anyhow::bail!("Passkeys in the app are not available on this platform.")
    }
}
pub mod traffic_lights {
    pub fn apply(_: &tauri::AppHandle) {}
}
