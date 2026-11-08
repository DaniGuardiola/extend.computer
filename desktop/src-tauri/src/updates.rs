//! Sparkle owns downloading, signature verification, installation, and native UI.
//! Keep its bridge loaded for the application lifetime, on the macOS main thread.
use crate::runtime::Desktop;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::sync::{Arc, OnceLock, Weak};

static DESKTOP: OnceLock<Weak<Desktop>> = OnceLock::new();
static LIBRARY: OnceLock<usize> = OnceLock::new();

/// Every channel the updater knows, in the order Settings lists them.
const CHANNELS: [&str; 4] = ["stable", "beta", "alpha", "canary"];
/// Channels with a published feed on the `updates` branch. Settings offers no
/// other channel, so an update check never asks for a feed that doesn't
/// exist. Add a preview channel here once its first release is published.
const PUBLISHED_CHANNELS: &[&str] = &["stable"];

#[derive(Serialize)]
pub struct UpdateSettings {
    available: bool,
    /// Why automatic updates are off, when they are.
    unavailable: Option<&'static str>,
    automatic_checks: bool,
    automatic_downloads: bool,
    channel: String,
    /// The channels Settings offers. See `offered_channels`.
    channels: Vec<&'static str>,
    /// Whether the chosen channel has a feed; a choice from an older version
    /// may not, and then needs a way back to one that does.
    channel_published: bool,
    version: &'static str,
}
/// The channel a version was released on: the start of its prerelease
/// suffix, as the release script infers it (`0.4.0-beta.1` is beta).
fn release_channel(version: &str) -> &'static str {
    let suffix = version
        .split_once('-')
        .map_or("", |(_, suffix)| suffix.split('.').next().unwrap_or(""));
    CHANNELS
        .into_iter()
        .find(|channel| *channel != "stable" && *channel == suffix)
        .unwrap_or("stable")
}
/// Channels with a feed, plus this build's own channel, whose feed is
/// published together with the build.
fn published(channel: &str, version: &str) -> bool {
    PUBLISHED_CHANNELS.contains(&channel) || channel == release_channel(version)
}
/// The published channels and the current choice, so a choice without a feed
/// stays visible until the user picks another channel.
fn offered_channels(current: &str, version: &str) -> Vec<&'static str> {
    CHANNELS
        .into_iter()
        .filter(|channel| published(channel, version) || *channel == current)
        .collect()
}
/// Why this build can't update itself.
fn unavailable_reason() -> &'static str {
    if cfg!(target_os = "macos") {
        "Automatic updates are unavailable in this development build."
    } else {
        "Automatic updates aren’t available on Windows yet."
    }
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
#[cfg(not(target_os = "macos"))]
unsafe fn symbol<T>(_: &std::ffi::CStr) -> Result<T> {
    anyhow::bail!("Updates are unavailable in this build.")
}
pub fn initialize(app: &tauri::AppHandle, desktop: &Arc<Desktop>) -> Result<()> {
    let _ = DESKTOP.set(Arc::downgrade(desktop));
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
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
    let version = env!("CARGO_PKG_VERSION");
    let mut value = UpdateSettings {
        available: false,
        unavailable: Some(unavailable_reason()),
        automatic_checks: false,
        automatic_downloads: false,
        channel: "stable".into(),
        channels: Vec::new(),
        channel_published: true,
        version,
    };
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
    if value.available {
        value.unavailable = None;
        value.channels = offered_channels(&value.channel, version);
        value.channel_published = published(&value.channel, version);
    }
    Ok(value)
}
pub fn install_menu() {
    if LIBRARY.get().is_some() {
        if let Ok(install) =
            unsafe { symbol::<unsafe extern "C" fn()>(c"extend_updates_install_menu") }
        {
            unsafe { install() };
        }
    }
}
pub fn check() -> Result<()> {
    ensure!(settings()?.available, unavailable_reason());
    ensure!(!busy(), "Finish sharing before checking for updates.");
    let check: unsafe extern "C" fn() = unsafe { symbol(c"extend_updates_check")? };
    unsafe { check() };
    Ok(())
}
pub fn configure(checks: bool, downloads: bool, channel: &str) -> Result<UpdateSettings> {
    ensure!(CHANNELS.contains(&channel), "Unknown update channel.");
    let current = settings()?;
    ensure!(current.available, unavailable_reason());
    ensure!(
        current.channels.contains(&channel),
        "This update channel has no releases yet."
    );
    let configure: unsafe extern "C" fn(bool, bool, *const std::ffi::c_char) =
        unsafe { symbol(c"extend_updates_preferences")? };
    let channel = std::ffi::CString::new(channel)?;
    unsafe { configure(checks, checks && downloads, channel.as_ptr()) };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_name_the_channel_they_were_released_on() {
        for (version, channel) in [
            ("0.3.1", "stable"),
            ("0.4.0-beta.0", "beta"),
            ("0.4.0-alpha.3", "alpha"),
            ("0.4.0-canary.12", "canary"),
            ("0.4.0-rc.1", "stable"),
            ("0.4.0-betamax.1", "stable"),
        ] {
            assert_eq!(release_channel(version), channel, "{version}");
        }
    }

    #[test]
    fn settings_offer_only_channels_with_a_feed() {
        // A stable build offers Stable alone until a preview feed exists.
        assert_eq!(offered_channels("stable", "0.3.1"), ["stable"]);
        // A preview build offers its own channel, published with it.
        assert_eq!(offered_channels("beta", "0.4.0-beta.0"), ["stable", "beta"]);
        assert_eq!(
            offered_channels("stable", "0.4.0-beta.0"),
            ["stable", "beta"]
        );
        // A preview choice from an older version stays visible, flagged, so
        // the user can leave it.
        assert_eq!(offered_channels("alpha", "0.3.1"), ["stable", "alpha"]);
        assert!(!published("alpha", "0.3.1"));
        assert!(published("stable", "0.3.1") && published("alpha", "0.4.0-alpha.1"));
    }

    #[test]
    fn every_published_channel_is_a_known_channel() {
        assert!(PUBLISHED_CHANNELS.contains(&"stable"));
        assert!(PUBLISHED_CHANNELS
            .iter()
            .all(|channel| CHANNELS.contains(channel)));
    }

    #[test]
    fn unavailable_updates_say_why_for_this_platform() {
        let settings = settings().unwrap();
        assert!(!settings.available && settings.channels.is_empty());
        let reason = settings.unavailable.unwrap();
        if cfg!(target_os = "macos") {
            assert!(reason.contains("development build"));
        } else {
            assert!(reason.contains("Windows") && !reason.contains("development"));
        }
        assert_eq!(check().unwrap_err().to_string(), reason);
        assert_eq!(
            configure(true, true, "stable").err().unwrap().to_string(),
            reason
        );
        assert_eq!(
            configure(true, true, "nightly").err().unwrap().to_string(),
            "Unknown update channel."
        );
    }
}
