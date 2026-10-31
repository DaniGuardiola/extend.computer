//! The app's contract with its native helpers, described in
//! docs/architecture/native-helpers.md. The Swift helpers implement it by
//! hand; native/shared/fixtures pins both sides to the same bytes.
pub mod display;
pub mod input;
pub mod packet;

/// The `json!` messages these types replaced had sorted keys. A `Value` round
/// trip keeps the wire byte for byte.
fn sorted(value: &impl serde::Serialize) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::to_value(value)?)
}

#[cfg(test)]
mod fixture_tests;
