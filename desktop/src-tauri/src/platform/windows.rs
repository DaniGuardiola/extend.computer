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
    // Only the helper knows the screens' refresh rates.
    viewer_120fps: false,
    input_receive: true,
    input_send: true,
};
/// Input and screen capture need no grants, and there's no Wi-Fi broker.
pub const INPUT_PERMISSIONS: bool = false;
pub const WIFI_OPTIMIZATION: bool = false;
pub const SCREEN_PERMISSION: bool = false;
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
/// The Windows version, such as "Windows 10.0.26100.4349", from `ver`, for
/// diagnostics and problem reports. Windows 11 reports 10.0 with a build
/// number of 22000 or higher.
pub fn os_version() -> String {
    extend_computer_agent::cursor::helper_command("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        // The console code page can be anything; only the digits matter.
        .and_then(|o| windows_version(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_else(|| "Windows".into())
}
/// "Microsoft Windows [Version 10.0.26100.4349]" -> "Windows 10.0.26100.4349".
/// The word "Version" is translated in other display languages.
fn windows_version(ver: &str) -> Option<String> {
    let bracket = ver.split_once('[')?.1.split_once(']')?.0;
    let number = bracket.split_whitespace().next_back()?;
    (number.contains('.') && number.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        .then(|| format!("Windows {number}"))
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
/// Whether the taskbar, and the status icon's notification area with it, is
/// light (Settings > Personalization > Colors, "Choose your mode"). Windows
/// before 10 version 1903 has only the dark taskbar and no such setting.
#[cfg(windows)]
pub fn light_taskbar() -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut u32,
            size: *mut u32,
        ) -> i32;
    }
    // HKEY_CURRENT_USER, sign-extended as the C headers define it.
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let (mut light, mut size) = (0u32, 4u32);
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut light,
            &mut size,
        )
    };
    status == 0 && light != 0
}
#[cfg(not(windows))]
pub fn light_taskbar() -> bool {
    true
}

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

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_version_from_ver() {
        assert_eq!(
            super::windows_version("\r\nMicrosoft Windows [Version 10.0.26100.4349]\r\n")
                .as_deref(),
            Some("Windows 10.0.26100.4349")
        );
        assert_eq!(
            super::windows_version("Microsoft Windows [Versión 10.0.19045.3803]").as_deref(),
            Some("Windows 10.0.19045.3803")
        );
        assert_eq!(super::windows_version("Microsoft Windows"), None);
        assert_eq!(super::windows_version("[Version <bad>]"), None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_taskbar_theme_as_reg_does() {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "SystemUsesLightTheme",
            ])
            .output()
            .unwrap();
        // "    SystemUsesLightTheme    REG_DWORD    0x1"
        let light = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| line.contains("SystemUsesLightTheme"))
            .and_then(|line| line.split_whitespace().next_back())
            .and_then(|value| u32::from_str_radix(value.trim_start_matches("0x"), 16).ok())
            .is_some_and(|value| value != 0);
        assert_eq!(super::light_taskbar(), light);
    }
}
