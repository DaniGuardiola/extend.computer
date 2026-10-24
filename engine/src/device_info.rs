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
    let kind = LOCAL_KIND
        .lock()
        .unwrap()
        .unwrap_or_else(|| *DETECTED_KIND.get_or_init(detect_kind));
    DeviceInfo {
        platform: std::env::consts::OS.into(),
        kind,
    }
}

fn detect_kind() -> DeviceKind {
    #[cfg(target_os = "macos")]
    return macos::kind().unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    DeviceKind::Unknown
}

#[cfg(target_os = "macos")]
mod macos {
    use super::DeviceKind;
    use std::{ffi::c_void, ptr};

    type CFTypeRef = *const c_void;
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        fn IOPSCopyPowerSourcesList(info: CFTypeRef) -> CFTypeRef;
        fn IOPSGetPowerSourceDescription(info: CFTypeRef, source: CFTypeRef) -> CFTypeRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
        fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            value: *const u8,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFEqual(left: CFTypeRef, right: CFTypeRef) -> u8;
        fn CFRelease(value: CFTypeRef);
    }
    struct Owned(CFTypeRef);
    impl Owned {
        fn new(value: CFTypeRef) -> Option<Self> {
            (!value.is_null()).then_some(Self(value))
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    pub(super) fn kind() -> Option<DeviceKind> {
        // Internal batteries distinguish MacBooks even when powered by AC.
        // External UPS devices have a different transport and do not count.
        // Model identifiers on recent Macs do not encode their hardware kind.
        if let Some(kind) = hardware_model().as_deref().and_then(model_kind) {
            return Some(kind);
        }
        unsafe {
            let info = Owned::new(IOPSCopyPowerSourcesInfo())?;
            let sources = Owned::new(IOPSCopyPowerSourcesList(info.0))?;
            let transport = Owned::new(CFStringCreateWithCString(
                ptr::null(),
                b"Transport Type\0".as_ptr(),
                0x08000100,
            ))?;
            let internal = Owned::new(CFStringCreateWithCString(
                ptr::null(),
                b"Internal\0".as_ptr(),
                0x08000100,
            ))?;
            let count = CFArrayGetCount(sources.0);
            if !(0..=64).contains(&count) {
                return None;
            }
            for index in 0..count {
                let description =
                    IOPSGetPowerSourceDescription(info.0, CFArrayGetValueAtIndex(sources.0, index));
                if description.is_null() {
                    return None;
                }
                let value = CFDictionaryGetValue(description, transport.0);
                if value.is_null() {
                    return None;
                }
                if CFEqual(value, internal.0) != 0 {
                    return Some(DeviceKind::Laptop);
                }
            }
            Some(DeviceKind::Desktop)
        }
    }

    fn hardware_model() -> Option<String> {
        let mut buffer = [0u8; 128];
        let mut length = buffer.len();
        let result = unsafe {
            libc::sysctlbyname(
                b"hw.model\0".as_ptr().cast(),
                buffer.as_mut_ptr().cast(),
                &mut length,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 || length == 0 || length > buffer.len() {
            return None;
        }
        std::str::from_utf8(&buffer[..length])
            .ok()
            .map(|value| value.trim_end_matches('\0').to_owned())
    }

    fn model_kind(model: &str) -> Option<DeviceKind> {
        if model.starts_with("MacBook") {
            Some(DeviceKind::Laptop)
        } else if ["iMac", "Macmini", "MacPro", "MacStudio"]
            .iter()
            .any(|prefix| model.starts_with(prefix))
        {
            Some(DeviceKind::Desktop)
        } else {
            None
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn modern_generic_model_ids_require_hardware_fallback() {
            assert_eq!(model_kind("MacBookPro18,1"), Some(DeviceKind::Laptop));
            assert_eq!(model_kind("MacBookAir10,1"), Some(DeviceKind::Laptop));
            for model in ["iMac20,1", "Macmini9,1", "MacPro7,1", "MacStudio1,1"] {
                assert_eq!(model_kind(model), Some(DeviceKind::Desktop));
            }
            for model in ["Mac14,2", "Mac15,12", "Mac16,1", ""] {
                assert_eq!(model_kind(model), None);
            }
        }
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
