//! Windows adapters. Wi-Fi low-jitter tuning and VPN-overlap socket scoping
//! are macOS-only, so Windows shares those fallbacks but reports Wi-Fi ready.
mod device_info;
mod identity;
mod link;
pub mod low_jitter;

pub(crate) use crate::platform::unsupported::local_network;
pub(crate) use device_info::device_kind;
pub(crate) use identity::load_identity;
pub(crate) use link::classify_link;
