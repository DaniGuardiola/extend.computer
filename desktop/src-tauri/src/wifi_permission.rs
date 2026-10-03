//! App-owned ServiceManagement bridge; no shell, installer script, or sudo prompt.
use anyhow::{ensure, Context, Result};
use std::path::Path;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{ffi::CString, sync::OnceLock};
    struct Bridge {
        status: unsafe extern "C" fn() -> i32,
        allow: unsafe extern "C" fn(i32) -> i32,
    }
    static BRIDGE: OnceLock<Bridge> = OnceLock::new();

    pub fn initialize(resources: &Path) -> Result<()> {
        if BRIDGE.get().is_some() {
            return Ok(());
        }
        let path = CString::new(
            resources
                .join("libExtendComputerPermissions.dylib")
                .to_string_lossy()
                .as_bytes(),
        )?;
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        ensure!(
            !handle.is_null(),
            "Wi-Fi permission setup is missing. Rebuild or reinstall extend.computer."
        );
        let status = unsafe { libc::dlsym(handle, c"extend_computer_wifi_status".as_ptr()) };
        let allow = unsafe { libc::dlsym(handle, c"extend_computer_wifi_allow".as_ptr()) };
        ensure!(
            !status.is_null() && !allow.is_null(),
            "Wi-Fi permission setup is outdated. Rebuild or reinstall extend.computer."
        );
        // Keep the library loaded for the application lifetime.
        let bridge = Bridge {
            status: unsafe {
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn() -> i32>(status)
            },
            allow: unsafe {
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(i32) -> i32>(allow)
            },
        };
        let _ = BRIDGE.set(bridge);
        Ok(())
    }
    pub fn status() -> i32 {
        BRIDGE.get().map_or(-2, |b| unsafe { (b.status)() })
    }
    pub fn open(repair: bool) -> Result<()> {
        let bridge = BRIDGE
            .get()
            .context("Wi-Fi permission setup is unavailable. Reopen the installed app.")?;
        super::registration_result(unsafe { (bridge.allow)(i32::from(repair)) })
    }
}

fn registration_result(status: i32) -> Result<()> {
    match status {
        1 | 2 | 4 => Ok(()), // Enabled, or waiting for the user's native Settings approval.
        -2 => anyhow::bail!("Open a signed extend.computer app to allow Wi-Fi optimization. For development, use desktop:dev."),
        _ => anyhow::bail!("Could not enable Wi-Fi optimization. Check Login Items & Extensions in System Settings, then try again."),
    }
}
pub fn initialize(resources: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::initialize(resources)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = resources;
        Ok(())
    }
}
pub fn status() -> i32 {
    #[cfg(target_os = "macos")]
    {
        macos::status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        -2
    }
}
/// Call on the GUI main thread, so ServiceManagement uses the app's identity.
pub fn open(repair: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::open(repair)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = repair;
        anyhow::bail!("Wi-Fi optimization is available on macOS.")
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn pending_approval_is_not_an_installation_error() {
        assert!(super::registration_result(1).is_ok());
        assert!(super::registration_result(2).is_ok());
        assert!(super::registration_result(4).is_ok());
        for status in [-2, -1, 0, 3] {
            assert!(super::registration_result(status).is_err());
        }
    }
}
