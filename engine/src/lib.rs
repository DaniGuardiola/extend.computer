#[cfg(all(feature = "dev-identity", not(debug_assertions)))]
compile_error!("dev-identity is forbidden in release builds");

pub mod account_trust;
pub mod cursor;
pub mod discovery;
pub mod identity;
pub mod low_jitter;
pub mod pairing;
pub mod session;
pub mod timing;
pub mod trust;
pub mod wire;

pub mod control;
pub mod input;

pub mod reconnect;

pub mod error;

mod verification;

pub mod protocol;
