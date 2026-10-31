//! Windows adapters. Wi-Fi low-jitter tuning is macOS-only, so Windows
//! shares the no-op adapter but reports it ready.
mod identity;
pub mod low_jitter;

pub(crate) use identity::load_identity;
