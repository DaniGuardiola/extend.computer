//! Hardware presentation metadata. It never grants access or identifies a peer.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Desktop,
    Laptop,
    Phone,
    Tablet,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub platform: String,
    #[serde(default)]
    pub kind: DeviceKind,
}

impl DeviceInfo {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.platform.is_empty()
                && self.platform.len() <= 32
                && self.platform.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'_'
                    || b == b'-'),
            "Invalid device platform"
        );
        Ok(())
    }
}

static LOCAL_KIND: Mutex<Option<DeviceKind>> = Mutex::new(None);
static DETECTED_KIND: OnceLock<DeviceKind> = OnceLock::new();

/// A platform adapter can supply a hardware kind detected through its OS APIs.
pub fn set_local_kind(kind: DeviceKind) {
    *LOCAL_KIND.lock().unwrap() = Some(kind);
}

pub fn local_info() -> DeviceInfo {
    let kind = LOCAL_KIND.lock().unwrap().unwrap_or_else(|| {
        *DETECTED_KIND.get_or_init(|| crate::platform::device_kind().unwrap_or_default())
    });
    DeviceInfo {
        platform: std::env::consts::OS.into(),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_bounded_and_future_kinds_are_neutral() {
        for platform in ["", "bad\nplatform", &"x".repeat(33)] {
            assert!(DeviceInfo {
                platform: platform.into(),
                kind: DeviceKind::Unknown
            }
            .validate()
            .is_err());
        }
        let info: DeviceInfo =
            serde_json::from_str(r#"{"platform":"macos","kind":"future_device"}"#).unwrap();
        assert_eq!(info.kind, DeviceKind::Unknown);
        assert!(info.validate().is_ok());
    }
    #[test]
    fn local_metadata_uses_operating_system_not_device_name() {
        let info = local_info();
        assert_eq!(info.platform, std::env::consts::OS);
        assert!(info.validate().is_ok());
    }
}
