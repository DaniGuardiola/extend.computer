//! Compile-time platform selection. Shared engine modules depend on this boundary.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub(crate) use macos::{load_identity, low_jitter};
#[cfg(not(target_os = "macos"))]
pub(crate) use unsupported::{load_identity, low_jitter};
