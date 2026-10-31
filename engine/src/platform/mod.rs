//! Compile-time platform selection. Shared engine modules depend on this boundary.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::{classify_link, device_kind, load_identity, local_network, low_jitter};
#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) use unsupported::{
    classify_link, device_kind, load_identity, local_network, low_jitter,
};
#[cfg(windows)]
pub(crate) use windows::{classify_link, device_kind, load_identity, local_network, low_jitter};
