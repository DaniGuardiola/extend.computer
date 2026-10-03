//! Sparkle owns downloading, signature verification, installation, and native UI.
//! Keep its bridge loaded for the application lifetime, on the macOS main thread.
use crate::runtime::Desktop;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::sync::{Arc, OnceLock, Weak};
use tauri::Manager;

static DESKTOP: OnceLock<Weak<Desktop>> = OnceLock::new();
#[cfg(target_os = "macos")]
static LIBRARY: OnceLock<usize> = OnceLock::new();

#[derive(Serialize)]
pub struct UpdateSettings {
    available: bool,
    automatic_checks: bool,
    automatic_downloads: bool,
    channel: String,
    version: &'static str,
}
extern "C" fn busy() -> bool {
    DESKTOP.get().and_then(Weak::upgrade).is_none_or(|app| {
        app.snapshot()
            .map_or(true, |snapshot| snapshot.session.is_some())
    })
}
extern "C" fn prepare() -> bool {
    DESKTOP
        .get()
        .and_then(Weak::upgrade)
        .is_some_and(|app| app.prepare_update())
}
extern "C" fn cancel() {
    if let Some(app) = DESKTOP.get().and_then(Weak::upgrade) {
        app.cancel_update();
    }
}
extern "C" fn cleanup() {
    if let Some(app) = DESKTOP.get().and_then(Weak::upgrade) {
        app.shutdown();
    }
}
#[cfg(target_os = "macos")]
unsafe fn symbol<T: Copy>(name: &std::ffi::CStr) -> Result<T> {
    let handle = *LIBRARY
        .get()
        .ok_or_else(|| anyhow::anyhow!("Updates are unavailable in this build."))?;
    let pointer = libc::dlsym(handle as *mut _, name.as_ptr());
    ensure!(
        !pointer.is_null(),
        "The update bridge is incomplete. Reinstall extend.computer."
    );
    Ok(std::mem::transmute_copy(&pointer))
}
pub fn initialize(app: &tauri::AppHandle, desktop: &Arc<Desktop>) -> Result<()> {
    let _ = DESKTOP.set(Arc::downgrade(desktop));
    #[cfg(target_os = "macos")]
    {
        let path = app.path().resource_dir()?.join("libExtendUpdater.dylib");
        if !path.is_file() {
            // Unbundled development has no updater; production must package it.
            #[cfg(debug_assertions)]
            return Ok(());
        }
        let path = std::ffi::CString::new(path.to_string_lossy().as_bytes())?;
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        ensure!(!handle.is_null(), "Could not load the update framework.");
        let _ = LIBRARY.set(handle as usize);
        let start: unsafe extern "C" fn(
            extern "C" fn() -> bool,
            extern "C" fn() -> bool,
            extern "C" fn(),
            extern "C" fn(),
        ) -> i32 = unsafe { symbol(c"extend_updates_start")? };
        ensure!(
            unsafe { start(busy, prepare, cleanup, cancel) } >= 0,
            "Could not initialize the updater."
        );
    }
    Ok(())
}
pub fn settings() -> Result<UpdateSettings> {
    let mut value = UpdateSettings {
        available: false,
        automatic_checks: false,
        automatic_downloads: false,
        channel: "stable".into(),
        version: env!("CARGO_PKG_VERSION"),
    };
    #[cfg(target_os = "macos")]
    if LIBRARY.get().is_some() {
        let status: unsafe extern "C" fn() -> i32 = unsafe { symbol(c"extend_updates_status")? };
        let channel: unsafe extern "C" fn() -> *const std::ffi::c_char =
            unsafe { symbol(c"extend_updates_channel")? };
        let flags = unsafe { status() };
        value.available = flags & 1 != 0;
        value.automatic_checks = flags & 2 != 0;
        value.automatic_downloads = flags & 4 != 0;
        value.channel = unsafe { std::ffi::CStr::from_ptr(channel()) }
            .to_string_lossy()
            .into_owned();
    }
    Ok(value)
}
pub fn install_menu() {
    #[cfg(target_os = "macos")]
    if LIBRARY.get().is_some() {
        if let Ok(install) =
            unsafe { symbol::<unsafe extern "C" fn()>(c"extend_updates_install_menu") }
        {
            unsafe { install() };
        }
    }
}
pub fn check() -> Result<()> {
    ensure!(
        settings()?.available,
        "Updates are unavailable in this development build."
    );
    ensure!(!busy(), "Finish sharing before checking for updates.");
    #[cfg(target_os = "macos")]
    {
        let check: unsafe extern "C" fn() = unsafe { symbol(c"extend_updates_check")? };
        unsafe { check() };
    }
    Ok(())
}
pub fn configure(checks: bool, downloads: bool, channel: &str) -> Result<UpdateSettings> {
    ensure!(
        matches!(channel, "stable" | "beta" | "alpha" | "canary"),
        "Unknown update channel."
    );
    ensure!(
        settings()?.available,
        "Updates are unavailable in this development build."
    );
    #[cfg(target_os = "macos")]
    {
        let configure: unsafe extern "C" fn(bool, bool, *const std::ffi::c_char) =
            unsafe { symbol(c"extend_updates_preferences")? };
        let channel = std::ffi::CString::new(channel)?;
        unsafe { configure(checks, checks && downloads, channel.as_ptr()) };
    }
    settings()
}

async fn on_main<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = send.send(action().map_err(|e| e.to_string()));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || receive.recv().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn update_settings(app: tauri::AppHandle) -> Result<UpdateSettings, String> {
    on_main(app, settings).await
}
#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<(), String> {
    on_main(app, check).await
}
#[tauri::command]
pub async fn configure_updates(
    app: tauri::AppHandle,
    checks: bool,
    downloads: bool,
    channel: String,
) -> Result<UpdateSettings, String> {
    on_main(app, move || configure(checks, downloads, &channel)).await
}
