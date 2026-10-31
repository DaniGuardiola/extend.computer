//! Compile-time platform selection. Shared desktop modules depend on this boundary.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(not(target_os = "macos"))]
pub use windows::*;

/// What this device can start and which OS grants it uses, so the UI offers
/// only what works here. Each platform exports its set as `CAPABILITIES`.
#[derive(Clone, Copy, serde::Serialize)]
pub struct Capabilities {
    pub extend_display: bool,
    pub mirror_display: bool,
    pub share_input: bool,
    /// Show and control another device's screen here.
    pub remote_desktop: bool,
    /// Input capture and injection need the user's approval (Accessibility).
    pub input_permissions: bool,
    pub wifi_optimization: bool,
}
