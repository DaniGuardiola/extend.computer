//! Stable encrypted compatibility envelope, independent of application SemVer.
use crate::{error::EngineError, wire::Channel};
use anyhow::{ensure, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::Mutex;

pub const MIN_VERSION: u16 = 1;
pub const MAX_VERSION: u16 = 1;
/// Encoding support only. OS adapter support is authenticated in the active input grant.
pub const INPUT_TEXT: &str = "input-text-v1";
pub const INPUT_SHORTCUTS: &str = "input-shortcuts-v1";
/// Input helper READY tokens; the helper's shortcut policy, not peer capabilities.
pub const INPUT_SHORTCUTS_COMMAND: &str = "input-shortcuts-command-v1";
pub const INPUT_SHORTCUTS_CONTROL: &str = "input-shortcuts-control-v1";
pub const CONTROL: &str = "control-v1";
pub const DISPLAY_INPUT: &str = "display-input-v1";
pub const RECEIVER_SETTINGS: &str = "receiver-settings-v1";
/// Modifier masks may mark right-hand keys (see input::RIGHT_MODIFIERS).
pub const INPUT_SIDED_MODIFIERS: &str = "input-sided-modifiers-v1";
/// A display receiver may answer an offer with `Declined` and a reason.
pub const DISPLAY_DECLINE: &str = "display-decline-v1";

/// Role tokens a device lists in its Hello `offers`: what it can do in a
/// session, as opposed to `capabilities`, the wire features both ends must
/// share. Names and limits are frozen once released.
pub mod offer {
    pub const DISPLAY_SOURCE_MIRROR: &str = "display-source-mirror";
    pub const DISPLAY_SOURCE_EXTEND: &str = "display-source-extend";
    /// A source that applies a viewer's input to the captured display.
    pub const DISPLAY_CONTROL: &str = "display-control";
    /// Presents another device's mirrored or extended display, H.264 at least.
    pub const DISPLAY_VIEWER: &str = "display-viewer";
    /// A viewer that sends the user's input to a source that allows control.
    pub const DISPLAY_VIEWER_INPUT: &str = "display-viewer-input";
    pub const DISPLAY_DECODE_HEVC: &str = "display-decode-hevc";
    /// A viewer that draws the pointer sent beside the video.
    pub const DISPLAY_CURSOR_OVERLAY: &str = "display-cursor-overlay";
    pub const INPUT_RECEIVE: &str = "input-receive";
    pub const INPUT_SEND: &str = "input-send";
    pub const MAX_TOKENS: usize = 32;
    pub const MAX_TOKEN_LEN: usize = 48;

    /// At most 32 tokens of 1 to 48 lowercase letters, digits and hyphens.
    pub fn valid(tokens: &[String]) -> bool {
        tokens.len() <= MAX_TOKENS
            && tokens.iter().all(|token| {
                (1..=MAX_TOKEN_LEN).contains(&token.len())
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    }
}

/// The roles a peer offered in its Hello. Unknown when it sent none (older
/// builds) or a list this build can't read; callers then keep the behaviour
/// that predates offers rather than guess from the platform name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offers(Option<Vec<String>>);
impl Offers {
    pub fn unknown() -> Self {
        Self(None)
    }
    /// None when the list breaks the grammar in `offer::valid`.
    pub fn known(tokens: Vec<String>) -> Option<Self> {
        offer::valid(&tokens).then_some(Self(Some(tokens)))
    }
    pub fn tokens(&self) -> Option<&[String]> {
        self.0.as_deref()
    }
    /// None when the peer did not say.
    pub fn has(&self, token: &str) -> Option<bool> {
        self.0
            .as_ref()
            .map(|tokens| tokens.iter().any(|t| t == token))
    }
    /// Unknown peers are allowed, as before offers existed.
    pub fn allows(&self, token: &str) -> bool {
        self.has(token) != Some(false)
    }
}

static LOCAL_OFFERS: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// What this device offers in every later Hello. Without a call, or with an
/// invalid list, the Hello carries no offers and peers treat it as unknown.
pub fn set_local_offers(tokens: Vec<String>) {
    *LOCAL_OFFERS.lock().unwrap() = offer::valid(&tokens).then_some(tokens);
}
fn local_offers() -> Option<Vec<String>> {
    LOCAL_OFFERS.lock().unwrap().clone()
}

/// A peer's offers are advisory, so a list this build can't read is unknown
/// rather than an incompatible Hello.
fn lenient_offers<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<Vec<String>>(value)
        .ok()
        .filter(|tokens| offer::valid(tokens)))
}

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
    /// The sender's physical link for this connection, for explanations only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<crate::link::Link>,
    /// Role tokens (`offer`). Older peers ignore the field.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_offers"
    )]
    offers: Option<Vec<String>>,
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
fn local_hello(link: Option<crate::link::Link>) -> Hello {
    Hello {
        min_protocol: MIN_VERSION,
        max_protocol: MAX_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: [
            CONTROL,
            DISPLAY_INPUT,
            INPUT_TEXT,
            INPUT_SHORTCUTS,
            RECEIVER_SETTINGS,
            INPUT_SIDED_MODIFIERS,
            DISPLAY_DECLINE,
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        device_info: Some(crate::device_info::local_info()),
        link,
        offers: local_offers(),
    }
}
pub(crate) fn exchange(channel: &mut Channel) -> Result<()> {
    let local = local_hello(channel.local_link());
    channel.send(&local)?;
    let remote: Hello = channel.receive()?;
    let (version, capabilities) = negotiate(&local, &remote)?;
    channel.set_protocol(version, capabilities);
    channel.set_remote_device_info(remote.device_info);
    channel.set_remote_link(remote.link);
    channel.set_remote_offers(remote.offers.and_then(Offers::known).unwrap_or_default());
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
            link: None,
            offers: None,
        }
    }
    #[test]
    fn shortcut_policy_capability_is_mutual_and_old_peers_keep_input() {
        let current = hello(1, 1, "current", &[CONTROL, DISPLAY_INPUT, INPUT_SHORTCUTS]);
        let legacy = hello(1, 1, "old", &[CONTROL, DISPLAY_INPUT]);
        let capabilities = negotiate(&current, &legacy).unwrap().1;
        assert!(capabilities.contains(&CONTROL.into()));
        assert!(!capabilities.contains(&INPUT_SHORTCUTS.into()));
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .contains(&INPUT_SHORTCUTS.into()));
    }
    #[test]
    fn text_is_opt_in_and_old_peers_keep_physical_input() {
        let current = hello(1, 1, "current", &[CONTROL, DISPLAY_INPUT, INPUT_TEXT]);
        let old = hello(1, 1, "old", &[CONTROL, DISPLAY_INPUT]);
        let (_, capabilities) = negotiate(&current, &old).unwrap();
        assert!(capabilities.contains(&CONTROL.into()));
        assert!(!capabilities.contains(&INPUT_TEXT.into()));
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .contains(&INPUT_TEXT.into()));
    }
    #[test]
    fn app_versions_do_not_need_to_match() {
        let a = hello(1, 3, "2.0.0", &[CONTROL, DISPLAY_INPUT]);
        let b = hello(1, 2, "0.9.0", &[CONTROL]);
        assert_eq!(negotiate(&a, &b).unwrap(), (2, vec![CONTROL.into()]));
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
        let legacy = hello(1, 1, "legacy", &[CONTROL]);
        let mut current = hello(1, 1, "current", &[CONTROL]);
        current.device_info = Some(crate::device_info::DeviceInfo {
            platform: "macos".into(),
            kind: crate::device_info::DeviceKind::Desktop,
        });
        assert_eq!(
            negotiate(&current, &legacy).unwrap(),
            (1, vec![CONTROL.into()])
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
        assert_eq!(old.capabilities, vec![CONTROL]);
        current.device_info.as_mut().unwrap().platform = "x".repeat(33);
        assert!(negotiate(&legacy, &current).is_err());
    }
    /// What a v0.3.1 Mac sent, from `git show v0.3.1:engine/src/protocol.rs`.
    const V031_HELLO: &str = r#"{"min_protocol":1,"max_protocol":1,"app_version":"0.3.1","capabilities":["input-v1","cursor-v1","control-v1","display-input-v1","input-text-v1","input-shortcuts-v1","receiver-settings-v1"],"device_info":{"platform":"macos","kind":"laptop"}}"#;
    fn offers_of(json: &str) -> Option<Vec<String>> {
        serde_json::from_str::<Hello>(json).unwrap().offers
    }
    fn with_offers(offers: &str) -> String {
        format!(
            r#"{{"min_protocol":1,"max_protocol":1,"app_version":"1","capabilities":[],"offers":{offers}}}"#
        )
    }
    #[test]
    fn offers_are_lenient_bounded_and_keep_unknown_tokens() {
        assert_eq!(
            offers_of(&with_offers(r#"["display-viewer","future-role-v9"]"#)),
            Some(vec!["display-viewer".into(), "future-role-v9".into()])
        );
        assert_eq!(offers_of(&with_offers("[]")), Some(vec![]));
        let max = format!(r#"["{}"]"#, "a".repeat(offer::MAX_TOKEN_LEN));
        assert!(offers_of(&with_offers(&max)).is_some());
        let many = |n: usize| {
            format!(
                "[{}]",
                (0..n)
                    .map(|i| format!(r#""t{i}""#))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        assert!(offers_of(&with_offers(&many(offer::MAX_TOKENS))).is_some());
        for invalid in [
            "null".to_string(),
            "5".into(),
            r#""display-viewer""#.into(),
            r#"{"display-viewer":true}"#.into(),
            r#"["display-viewer",7]"#.into(),
            r#"[""]"#.into(),
            r#"["Display-Viewer"]"#.into(),
            r#"["display viewer"]"#.into(),
            r#"["display_viewer"]"#.into(),
            format!(r#"["{}"]"#, "a".repeat(offer::MAX_TOKEN_LEN + 1)),
            many(offer::MAX_TOKENS + 1),
        ] {
            let json = with_offers(&invalid);
            let hello: Hello = serde_json::from_str(&json).unwrap();
            assert_eq!(hello.offers, None, "{invalid}");
            // Advisory metadata never makes a peer incompatible.
            assert!(negotiate(&hello, &hello).is_ok());
        }
        assert_eq!(Offers::known(vec!["Bad".into()]), None);
        let unknown = Offers::unknown();
        assert_eq!(unknown.has(offer::DISPLAY_VIEWER), None);
        assert!(unknown.allows(offer::DISPLAY_VIEWER));
        let windows = Offers::known(vec![offer::DISPLAY_VIEWER.into()]).unwrap();
        assert_eq!(windows.has(offer::DISPLAY_VIEWER), Some(true));
        assert!(!windows.allows(offer::DISPLAY_DECODE_HEVC));
    }
    #[test]
    fn recorded_v031_hello_still_negotiates_both_ways() {
        let old: Hello = serde_json::from_str(V031_HELLO).unwrap();
        assert_eq!(old.offers, None);
        let current = local_hello(Some(crate::link::Link::Wifi));
        let (version, capabilities) = negotiate(&current, &old).unwrap();
        assert_eq!(version, 1);
        assert_eq!(
            capabilities,
            vec![
                CONTROL,
                DISPLAY_INPUT,
                INPUT_TEXT,
                INPUT_SHORTCUTS,
                RECEIVER_SETTINGS
            ]
        );
        assert!(!capabilities.iter().any(|c| c == DISPLAY_DECLINE));
        // The v0.3.1 struct, which ignores unknown fields, reads a Hello that
        // carries the most offers and metadata this build can send.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct V031Hello {
            min_protocol: u16,
            max_protocol: u16,
            app_version: String,
            capabilities: Vec<String>,
            #[serde(default)]
            device_info: Option<crate::device_info::DeviceInfo>,
        }
        let mut largest = current;
        largest.offers = Some(
            (0..offer::MAX_TOKENS)
                .map(|i| format!("{i:0>width$}", width = offer::MAX_TOKEN_LEN))
                .collect(),
        );
        largest.device_info = Some(crate::device_info::DeviceInfo {
            platform: "x".repeat(32),
            kind: crate::device_info::DeviceKind::Laptop,
        });
        largest.app_version = "9".repeat(64);
        let bytes = serde_json::to_vec(&largest).unwrap();
        // Hello travels before display signaling raises the frame limit.
        assert!(
            bytes.len() + 16 <= crate::wire::MAX_FRAME,
            "{}",
            bytes.len()
        );
        let read: V031Hello = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(read.capabilities, largest.capabilities);
        assert!(read.capabilities.len() <= 32);
        let old = hello(1, 1, "0.3.1", &[CONTROL]);
        assert!(negotiate(&old, &serde_json::from_slice(&bytes).unwrap()).is_ok());
    }
    #[test]
    fn display_input_requires_both_peers_without_a_version_upgrade() {
        let current = hello(1, 1, "1.0", &[CONTROL, DISPLAY_INPUT]);
        let legacy = hello(1, 1, "0.9", &[CONTROL]);
        let (version, capabilities) = negotiate(&current, &legacy).unwrap();
        assert_eq!(version, 1);
        assert!(!capabilities.iter().any(|c| c == DISPLAY_INPUT));
        assert_eq!(capabilities, vec![CONTROL]);
        assert!(negotiate(&current, &current)
            .unwrap()
            .1
            .iter()
            .any(|c| c == DISPLAY_INPUT));
    }
}
