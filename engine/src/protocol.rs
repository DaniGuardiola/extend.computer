//! Stable encrypted compatibility envelope, independent of application SemVer.
use crate::{error::EngineError, wire::Channel};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const MIN_VERSION: u16 = 1;
pub const MAX_VERSION: u16 = 1;
pub const INPUT: &str = "input-v1";
/// Encoding support only. OS adapter support is authenticated in the active input grant.
pub const INPUT_TEXT: &str = "input-text-v1";
pub const INPUT_SHORTCUTS: &str = "input-shortcuts-v1";
pub const CURSOR: &str = "cursor-v1";
pub const CONTROL: &str = "control-v1";
pub const DISPLAY_INPUT: &str = "display-input-v1";
pub const RECEIVER_SETTINGS: &str = "receiver-settings-v1";

// Keep this envelope additive and bounded. Unknown fields are deliberately
// accepted here; ordinary control messages remain strict and versioned.
#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    min_protocol: u16,
    max_protocol: u16,
    app_version: String,
    capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_info: Option<crate::device_info::DeviceInfo>,
}
fn negotiate(local: &Hello, remote: &Hello) -> Result<(u16, Vec<String>)> {
    if let Some(info) = &remote.device_info {
        ensure!(info.validate().is_ok(), EngineError::ProtocolIncompatible);
    }
    ensure!(
        remote.min_protocol > 0
            && remote.min_protocol <= remote.max_protocol
            && remote.app_version.len() <= 64
            && remote.capabilities.len() <= 32
            && remote.capabilities.iter().all(|s| s.len() <= 64),
        EngineError::ProtocolIncompatible
    );
    let version = local.max_protocol.min(remote.max_protocol);
    ensure!(
        version >= local.min_protocol.max(remote.min_protocol),
        EngineError::ProtocolIncompatible
    );
    let capabilities = local
        .capabilities
        .iter()
        .filter(|c| remote.capabilities.contains(c))
        .cloned()
        .collect();
    Ok((version, capabilities))
}
pub(crate) fn exchange(channel: &mut Channel) -> Result<()> {
    let local = Hello {
        min_protocol: MIN_VERSION,
        max_protocol: MAX_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: [
            INPUT,
            CURSOR,
            CONTROL,
            DISPLAY_INPUT,
            INPUT_TEXT,
            INPUT_SHORTCUTS,
            RECEIVER_SETTINGS,
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        device_info: Some(crate::device_info::local_info()),
    };
    channel.send(&local)?;
    let remote: Hello = channel.receive()?;
    let (version, capabilities) = negotiate(&local, &remote)?;
    channel.set_protocol(version, capabilities);
    channel.set_remote_device_info(remote.device_info);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hello(min: u16, max: u16, app: &str, capabilities: &[&str]) -> Hello {
        Hello {
            min_protocol: min,
            max_protocol: max,
            app_version: app.into(),
            capabilities: capabilities.iter().map(|c| (*c).into()).collect(),
            device_info: None,
        }
    }
    #[test]
    fn shortcut_policy_capability_is_mutual_and_old_peers_keep_input() {
        let current = hello(1, 1, "current", &[INPUT, DISPLAY_INPUT, INPUT_SHORTCUTS]);
        let legacy = hello(1, 1, "old", &[INPUT, DISPLAY_INPUT]);
        let capabilities = negotiate(&current, &legacy).unwrap().1;
        assert!(capabilities.contains(&INPUT.into()));
        assert!(!capabilities.contains(&INPUT_SHORTCUTS.into()));
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .contains(&INPUT_SHORTCUTS.into()));
    }
    #[test]
    fn text_is_opt_in_and_old_peers_keep_physical_input() {
        let current = hello(1, 1, "current", &[INPUT, DISPLAY_INPUT, INPUT_TEXT]);
        let old = hello(1, 1, "old", &[INPUT, DISPLAY_INPUT]);
        let (_, capabilities) = negotiate(&current, &old).unwrap();
        assert!(capabilities.contains(&INPUT.into()));
        assert!(!capabilities.contains(&INPUT_TEXT.into()));
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .contains(&INPUT_TEXT.into()));
    }
    #[test]
    fn app_versions_do_not_need_to_match() {
        let a = hello(1, 3, "2.0.0", &[INPUT, CONTROL]);
        let b = hello(1, 2, "0.9.0", &[INPUT]);
        assert_eq!(negotiate(&a, &b).unwrap(), (2, vec![INPUT.into()]));
    }
    #[test]
    fn disjoint_or_invalid_ranges_rejected() {
        let a = hello(2, 3, "1.0.0", &[]);
        for b in [
            hello(1, 1, "1.0.0", &[]),
            hello(3, 2, "1.0.0", &[]),
            hello(0, 3, "1.0.0", &[]),
        ] {
            assert!(negotiate(&a, &b).unwrap_err().is::<EngineError>());
        }
    }
    #[test]
    fn additive_envelope_and_bounded_capabilities() {
        let remote: Hello = serde_json::from_str(r#"{"min_protocol":1,"max_protocol":1,"app_version":"1.0","capabilities":[],"future":true}"#).unwrap();
        assert!(negotiate(&remote, &remote).is_ok());
        let huge = hello(1, 1, "1", &[&"x".repeat(65)]);
        assert!(negotiate(&remote, &huge).is_err());
    }
    #[test]
    fn hardware_metadata_is_additive_and_legacy_peers_remain_compatible() {
        let legacy = hello(1, 1, "legacy", &[INPUT]);
        let mut current = hello(1, 1, "current", &[INPUT]);
        current.device_info = Some(crate::device_info::DeviceInfo {
            platform: "macos".into(),
            kind: crate::device_info::DeviceKind::Desktop,
        });
        assert_eq!(
            negotiate(&current, &legacy).unwrap(),
            (1, vec![INPUT.into()])
        );
        #[derive(Deserialize)]
        struct LegacyHello {
            min_protocol: u16,
            max_protocol: u16,
            app_version: String,
            capabilities: Vec<String>,
        }
        let old: LegacyHello =
            serde_json::from_slice(&serde_json::to_vec(&current).unwrap()).unwrap();
        assert_eq!((old.min_protocol, old.max_protocol), (1, 1));
        assert_eq!(old.app_version, "current");
        assert_eq!(old.capabilities, vec![INPUT]);
        current.device_info.as_mut().unwrap().platform = "x".repeat(33);
        assert!(negotiate(&legacy, &current).is_err());
    }
    #[test]
    fn display_input_requires_both_peers_without_a_version_upgrade() {
        let current = hello(1, 1, "1.0", &[INPUT, CONTROL, DISPLAY_INPUT]);
        let legacy = hello(1, 1, "0.9", &[INPUT, CONTROL]);
        let (version, capabilities) = negotiate(&current, &legacy).unwrap();
        assert_eq!(version, 1);
        assert!(!capabilities.iter().any(|c| c == DISPLAY_INPUT));
        assert_eq!(capabilities, vec![INPUT, CONTROL]);
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .iter()
            .any(|c| c == DISPLAY_INPUT));
    }
}
