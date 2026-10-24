//! Ordered input events. mac_key is explicitly platform-specific until adapters
//! translate physical keys; mouse and modifier fields are platform-neutral.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// Editing shortcut modifier proven by the active receiver adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutPolicy {
    Command,
    Control,
}
impl ShortcutPolicy {
    pub(crate) fn validate_grant(policy: Option<Self>, negotiated: bool) -> Result<Option<Self>> {
        ensure!(
            policy.is_none() || negotiated,
            "Shortcut policy was not negotiated"
        );
        Ok(policy)
    }
    pub fn modifier_mask(self) -> u8 {
        match self {
            Self::Command => 8,
            Self::Control => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEvent {
    Button {
        button: u8,
        down: bool,
        clicks: u8,
    },
    Scroll {
        dx: i32,
        dy: i32,
    },
    MacKey {
        code: u16,
        down: bool,
        repeat: bool,
    },
    Modifiers {
        mask: u8,
    },
    /// Final committed text, independent of the receiver keyboard layout.
    Text {
        value: String,
    },
    Release,
}
impl InputEvent {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Button { button, clicks, .. } => {
                ensure!(*button <= 2 && (1..=3).contains(clicks), "invalid button")
            }
            Self::Scroll { dx, dy } => ensure!(
                dx.unsigned_abs() <= 4096 && dy.unsigned_abs() <= 4096,
                "invalid scroll"
            ),
            Self::MacKey { code, down, repeat } => {
                ensure!(
                    *code < 128 && ![54, 55, 56, 57, 58, 59, 60, 61, 62, 63].contains(code),
                    "invalid nonmodifier key"
                );
                ensure!(!repeat || *down, "invalid key repeat");
            }
            Self::Modifiers { mask } => ensure!(*mask < 16, "invalid modifier mask"),
            Self::Text { value } => ensure!(
                !value.is_empty()
                    && value.len() <= 256
                    && value.chars().count() <= 64
                    && !value.chars().any(|c| c.is_control()),
                "invalid committed text"
            ),
            Self::Release => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    #[test]
    fn shortcut_policy_is_typed_and_requires_negotiation() {
        for (policy, name, mask) in [
            (ShortcutPolicy::Command, "command", 8),
            (ShortcutPolicy::Control, "control", 2),
        ] {
            assert_eq!(serde_json::to_value(policy).unwrap(), name);
            assert_eq!(policy.modifier_mask(), mask);
            assert_eq!(
                ShortcutPolicy::validate_grant(Some(policy), true).unwrap(),
                Some(policy)
            );
            assert!(ShortcutPolicy::validate_grant(Some(policy), false).is_err());
        }
        assert_eq!(ShortcutPolicy::validate_grant(None, false).unwrap(), None);
        for value in ["windows", "meta", "", "COMMAND"] {
            assert!(serde_json::from_value::<ShortcutPolicy>(serde_json::json!(value)).is_err());
        }
    }
}
