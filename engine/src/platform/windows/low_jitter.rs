//! Windows has no Wi-Fi broker to prepare, so connections are always ready.
pub use crate::platform::unsupported::low_jitter::{
    configure_app_helper, require_ready_for_peer, required_for_peer, start_for_peer,
    start_required, Lease,
};

pub fn ready() -> bool {
    true
}
