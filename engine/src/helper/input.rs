//! Input helper (`ExtendComputerCursor`) lines.
use super::display::{Bounds, CaptureTarget};
use crate::{
    display::input::{Binding, Point, Size},
    input::{InputEvent, ShortcutPolicy},
    protocol,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// `status`: permission to capture (`listen`) and to post (`post`) input.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub listen: bool,
    pub post: bool,
}
impl Status {
    /// Skips fields it does not know, so helpers can add them.
    pub fn parse(line: &str) -> Self {
        Self {
            listen: line.contains("listen=true"),
            post: line.contains("post=true"),
        }
    }
}
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "listen={} post={}", self.listen, self.post)
    }
}

/// What `inject-control` can apply beyond pointer, button, scroll and key events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    InputText,
    ShortcutsCommand,
    ShortcutsControl,
}
impl Token {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InputText => protocol::INPUT_TEXT,
            Self::ShortcutsCommand => protocol::INPUT_SHORTCUTS_COMMAND,
            Self::ShortcutsControl => protocol::INPUT_SHORTCUTS_CONTROL,
        }
    }

    fn parse(token: &str) -> Option<Self> {
        [
            Self::InputText,
            Self::ShortcutsCommand,
            Self::ShortcutsControl,
        ]
        .into_iter()
        .find(|known| known.as_str() == token)
    }

    pub fn shortcut_policy(self) -> Option<ShortcutPolicy> {
        match self {
            Self::InputText => None,
            Self::ShortcutsCommand => Some(ShortcutPolicy::Command),
            Self::ShortcutsControl => Some(ShortcutPolicy::Control),
        }
    }
}

/// `READY <w> <h> [tokens]` from `inject-control` and `capture-control-*`: the
/// main display in logical points. Only `inject-control` sends tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlReady {
    pub width: f64,
    pub height: f64,
    pub tokens: Vec<Token>,
}
impl ControlReady {
    pub fn parse(line: &str) -> Result<Self> {
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(
            (3..=5).contains(&fields.len()) && fields[0] == "READY",
            "Invalid helper startup line"
        );
        let mut tokens: Vec<Token> = Vec::new();
        for field in &fields[3..] {
            let token = Token::parse(field).context("Unknown helper token")?;
            // At most one text token and one shortcut policy.
            ensure!(
                !tokens
                    .iter()
                    .any(|known| known.shortcut_policy().is_some()
                        == token.shortcut_policy().is_some()),
                "Repeated helper token"
            );
            tokens.push(token);
        }
        Ok(Self {
            width: fields[1].parse()?,
            height: fields[2].parse()?,
            tokens,
        })
    }

    pub fn text(&self) -> bool {
        self.tokens.contains(&Token::InputText)
    }

    pub fn shortcut_policy(&self) -> Option<ShortcutPolicy> {
        self.tokens.iter().find_map(|token| token.shortcut_policy())
    }
}
impl fmt::Display for ControlReady {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "READY {:.1} {:.1}", self.width, self.height)?;
        for token in &self.tokens {
            write!(f, " {}", token.as_str())?;
        }
        Ok(())
    }
}

/// `inject-display <target>`: the only display this helper may touch, bound to
/// one display input lease.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayTarget {
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
    pub display_id: u32,
    pub bounds: Bounds,
    pub canvas: Size,
    pub content: Bounds,
}
impl DisplayTarget {
    pub fn new(binding: &Binding, target: &CaptureTarget) -> Self {
        Self {
            session: binding.session.clone(),
            epoch: binding.epoch,
            revision: binding.revision,
            display_id: target.display_id,
            bounds: target.bounds.clone(),
            canvas: target.canvas,
            content: target.content.clone(),
        }
    }

    pub fn to_arg(&self) -> serde_json::Result<String> {
        Ok(String::from_utf8(super::sorted(self)?).expect("JSON is UTF-8"))
    }
}

/// `READY <json>` from `inject-display`: the target echoed back, plus what the
/// helper supports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayReady {
    #[serde(flatten)]
    pub target: DisplayTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_policy: Option<ShortcutPolicy>,
}
impl DisplayReady {
    const FIELDS: [&str; 9] = [
        "session",
        "epoch",
        "revision",
        "display_id",
        "bounds",
        "canvas",
        "content",
        "text_supported",
        "shortcut_policy",
    ];

    pub fn parse_line(line: &str) -> Result<Self> {
        let json = line
            .strip_prefix("READY ")
            .context("Cursor helper did not become ready")?;
        // A flattened target ignores unknown fields, so name them all first.
        let fields: std::collections::BTreeMap<String, serde::de::IgnoredAny> =
            serde_json::from_str(json)?;
        ensure!(
            fields
                .keys()
                .all(|key| Self::FIELDS.contains(&key.as_str())),
            "Cursor helper accepted a different capture target"
        );
        Ok(serde_json::from_str(json)?)
    }
}

/// One `inject-display` stdin line, answered by [`Applied`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayEnvelope {
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
    pub sequence: u64,
    pub event: DisplayEvent,
}
impl DisplayEnvelope {
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        super::sorted(self)
    }
}

/// Pointer positions are normalized to the encoded canvas.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayEvent {
    #[serde(flatten)]
    pub action: DisplayAction,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub point: Option<Point>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayAction {
    Move,
    Alive,
    /// Releases everything held and ends the lease.
    Release,
    #[serde(untagged)]
    Input(InputEvent),
}

/// `APPLIED <json>`: the helper applied this line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
    pub sequence: u64,
}
impl Applied {
    pub fn new(binding: &Binding, sequence: u64) -> Self {
        Self {
            session: binding.session.clone(),
            epoch: binding.epoch,
            revision: binding.revision,
            sequence,
        }
    }

    pub fn parse_line(line: &str) -> Result<Self> {
        let json = line
            .strip_prefix("APPLIED ")
            .context("Cursor helper rejected input")?;
        Ok(serde_json::from_str(json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_ready_allows_one_text_token_and_one_shortcut_policy() {
        let ready =
            ControlReady::parse("READY 1728.0 1117.0 input-text-v1 input-shortcuts-control-v1\n")
                .unwrap();
        assert!(ready.text());
        assert_eq!(ready.shortcut_policy(), Some(ShortcutPolicy::Control));
        assert_eq!(
            ready.to_string(),
            "READY 1728.0 1117.0 input-text-v1 input-shortcuts-control-v1"
        );
        let plain = ControlReady::parse("READY 1512 982").unwrap();
        assert!(!plain.text() && plain.shortcut_policy().is_none());
        assert_eq!(plain.to_string(), "READY 1512.0 982.0");
        for invalid in [
            "READY 1920",
            "READ 1920 1080",
            "READY wide 1080",
            "READY 1920 1080 input-shortcuts-windows-v1",
            "READY 1920 1080 input-text-v1 input-text-v1",
            "READY 1920 1080 input-shortcuts-command-v1 input-shortcuts-control-v1",
            "READY 1920 1080 input-text-v1 input-shortcuts-command-v1 extra",
        ] {
            assert!(ControlReady::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn status_reads_known_fields_and_formats_both() {
        let status = Status::parse("listen=true post=false wifi=true\n");
        assert_eq!(
            status,
            Status {
                listen: true,
                post: false
            }
        );
        assert_eq!(
            Status {
                listen: true,
                post: true
            }
            .to_string(),
            "listen=true post=true"
        );
    }
}
