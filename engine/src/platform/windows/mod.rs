//! Windows adapters. Wi-Fi low-jitter tuning is macOS-only, so Windows shares the no-op adapter.
mod identity;

pub(crate) use super::unsupported::low_jitter;
pub(crate) use identity::load_identity;
