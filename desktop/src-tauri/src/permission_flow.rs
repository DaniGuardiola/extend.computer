//! Main-thread adapter for extend.computer's bundled Swift PermissionFlow bridge.
use anyhow::{ensure, Context, Result};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::{
    ffi::{CStr, CString},
    sync::OnceLock,
};

pub fn open(resources: &Path, permission: &str) -> Result<()> {
    ensure!(
        matches!(permission, "listen" | "post"),
        "Unknown permission."
    );
    #[cfg(target_os = "macos")]
    {
        static LIBRARY: OnceLock<Result<usize, String>> = OnceLock::new();
        let library = LIBRARY
            .get_or_init(|| {
                let path = CString::new(
                    resources
                        .join("libExtendComputerPermissions.dylib")
                        .to_string_lossy()
                        .as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                // The handle stays alive for the app lifetime: Swift owns a live panel/controller.
                let handle =
                    unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
                if handle.is_null() {
                    let error = unsafe { libc::dlerror() };
                    return Err(if error.is_null() {
                        "Could not load permission guidance".into()
                    } else {
                        unsafe { CStr::from_ptr(error) }
                            .to_string_lossy()
                            .into_owned()
                    });
                }
                Ok(handle as usize)
            })
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?;
        let symbol = unsafe {
            libc::dlsym(
                *library as *mut _,
                c"extend_computer_permissions_open".as_ptr(),
            )
        };
        ensure!(
            !symbol.is_null(),
            "Permission guidance entry point is missing."
        );
        let show: unsafe extern "C" fn(i32) = unsafe { std::mem::transmute(symbol) };
        unsafe { show(if permission == "listen" { 0 } else { 1 }) };
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = resources;
        anyhow::bail!("Permission guidance is currently available on macOS.")
    }
}

pub fn resources(app: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    use tauri::Manager;
    let packaged = app.path().resource_dir()?;
    if packaged
        .join("libExtendComputerPermissions.dylib")
        .is_file()
    {
        return Ok(packaged);
    }
    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("permission-resources");
        if development
            .join("libExtendComputerPermissions.dylib")
            .is_file()
        {
            return Ok(development);
        }
    }
    Err(anyhow::anyhow!(
        "Permission guidance is missing. Rebuild or reinstall extend.computer."
    ))
    .context("Could not open macOS permissions")
}
