//! Ordered input events. mac_key is explicitly platform-specific until adapters
//! translate physical keys; mouse and modifier fields are platform-neutral.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEvent {
    Button { button: u8, down: bool, clicks: u8 },
    Scroll { dx: i32, dy: i32 },
    MacKey { code: u16, down: bool, repeat: bool },
    Modifiers { mask: u8 },
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
            Self::Release => {}
        }
        Ok(())
    }
}
