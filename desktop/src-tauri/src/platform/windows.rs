//! Windows builds keep their helpers beside the app executable.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// The display helper mirrors the screen in H.264 with the pointer drawn in,
// and views another screen in H.264. Extending needs the extend.computer
// display driver, so only the helper's report adds it, on a PC that has the
// driver. There's no display input injection yet.
pub const ABILITIES: crate::runtime::Abilities = crate::runtime::Abilities {
    source_mirror: true,
    source_extend: false,
    display_control: false,
    viewer: true,
    viewer_input: true,
    encode_hevc: false,
    decode_hevc: false,
    cursor_send: false,
    cursor_overlay: false,
    input_receive: true,
    input_send: true,
};
/// Input needs no grants, and there's no Wi-Fi broker.
pub const INPUT_PERMISSIONS: bool = false;
pub const WIFI_OPTIMIZATION: bool = false;
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
    extend_computer_agent::cursor::helper_command("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}
/// Keep the PC from sleeping while idle during a session, which would drop
/// the connection. Windows ties the request to the calling thread, so the
/// guard can't leave it, and ends it on drop.
#[cfg(windows)]
pub fn session_activity() -> impl Sized {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    struct Awake(std::marker::PhantomData<*const ()>);
    impl Drop for Awake {
        fn drop(&mut self) {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
    Awake(std::marker::PhantomData)
}
#[cfg(not(windows))]
pub fn session_activity() -> impl Sized {}
/// Windows has no App Nap to hold off while connections are allowed.
pub fn available_activity() -> impl Sized {}

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
