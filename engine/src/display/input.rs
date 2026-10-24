//! Bounded, display-scoped input. Negotiated transport support never grants OS
//! input access: adapters authorize a lease and release held state on teardown.
use crate::{input::InputEvent, session::display::Config};
use anyhow::{ensure, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_MESSAGE_BYTES: usize = 1024;
pub const MAX_PENDING_MESSAGES: usize = 64;
pub const MAX_BUFFERED_BYTES: usize = MAX_MESSAGE_BYTES * MAX_PENDING_MESSAGES;

/// The source creates this binding. Neither the session nonce nor a revision
/// authorizes access to a display chosen by a remote peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
}
impl Binding {
    pub fn new(epoch: u64, revision: u64) -> Self {
        let mut nonce = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        Self {
            session: hex::encode(nonce),
            epoch,
            revision,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.session.len() == 32
                && self
                    .session
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "Invalid display input session"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub fn validate(self) -> Result<()> {
        ensure!(
            [self.x, self.y]
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
            "Invalid display input point"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn validate(self) -> Result<()> {
        Point {
            x: self.x,
            y: self.y,
        }
        .validate()?;
        ensure!(
            [self.width, self.height]
                .into_iter()
                .all(|v| v.is_finite() && v > 0.0 && v <= 1.0)
                && self.x + self.width <= 1.0 + f64::EPSILON * 4.0
                && self.y + self.height <= 1.0 + f64::EPSILON * 4.0,
            "Invalid display input content rectangle"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}
impl Size {
    fn validate(self) -> Result<()> {
        ensure!(
            (1..=32768).contains(&self.width) && (1..=32768).contains(&self.height),
            "Invalid display input dimensions"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub content: Rect,
    pub canvas: Size,
    pub raster: Size,
    pub source_size: Size,
}
impl Geometry {
    pub fn validate(&self) -> Result<()> {
        self.content.validate()?;
        self.canvas.validate()?;
        self.raster.validate()?;
        self.source_size.validate()?;
        ensure!(
            (640..=4096).contains(&self.canvas.width)
                && (480..=4096).contains(&self.canvas.height)
                && self.raster.width <= 4096
                && self.raster.height <= 4096
                && [1, 2]
                    .into_iter()
                    .any(|s| self.raster.width == self.canvas.width * s
                        && self.raster.height == self.canvas.height * s),
            "Invalid display input raster"
        );
        Ok(())
    }
    pub fn validate_for_config(&self, config: &Config) -> Result<()> {
        self.validate()?;
        config.validate()?;
        let scale = if config.retina { 2 } else { 1 };
        ensure!(
            self.canvas
                == Size {
                    width: config.width,
                    height: config.height
                }
                && self.raster
                    == Size {
                        width: config.width * scale,
                        height: config.height * scale
                    },
            "Display input geometry is stale"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Move {
        point: Point,
    },
    /// Button/scroll include their target point atomically. Keyboard/release
    /// retain the existing typed input schema; adapters may decline keyboard.
    Input {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point: Option<Point>,
        event: InputEvent,
    },
    Alive,
    Stop,
}
impl Command {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Move { point } => point.validate()?,
            Self::Input { point, event } => {
                event.validate()?;
                if let Some(point) = point {
                    point.validate()?;
                }
                ensure!(
                    point.is_some()
                        == matches!(event, InputEvent::Button { .. } | InputEvent::Scroll { .. }),
                    "Display button/scroll requires a point; other input must omit it"
                );
            }
            Self::Alive | Self::Stop => {}
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ReceivingOff,
    Permission,
    Unsupported,
    NotReady,
    Stopped,
    Timeout,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Request {
        request: u64,
    },
    Granted {
        request: u64,
        binding: Binding,
        geometry: Geometry,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        text_supported: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shortcut_policy: Option<crate::input::ShortcutPolicy>,
    },
    Denied {
        request: u64,
        reason: Reason,
    },
    Command {
        binding: Binding,
        sequence: u64,
        command: Command,
    },
    Applied {
        binding: Binding,
        sequence: u64,
    },
    Revoked {
        binding: Binding,
        reason: Reason,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Source,
    Receiver,
}
impl Message {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Granted {
                binding, geometry, ..
            } => {
                binding.validate()?;
                geometry.validate()?;
            }
            Self::Command {
                binding,
                sequence,
                command,
            } => {
                binding.validate()?;
                ensure!(*sequence > 0, "Invalid display input sequence");
                command.validate()?;
            }
            Self::Applied { binding, sequence } => {
                binding.validate()?;
                ensure!(*sequence > 0, "Invalid display input sequence");
            }
            Self::Revoked { binding, .. } => binding.validate()?,
            Self::Request { .. } | Self::Denied { .. } => {}
        }
        Ok(())
    }
    pub fn validate_sender(&self, role: Role) -> Result<()> {
        self.validate()?;
        let receiver = matches!(self, Self::Request { .. } | Self::Command { .. });
        ensure!(
            receiver == (role == Role::Receiver),
            "Wrong display input direction"
        );
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_MESSAGE_BYTES,
            "Display input message too large"
        );
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_MESSAGE_BYTES,
            "Invalid display input length"
        );
        let message: Self = serde_json::from_slice(bytes)?;
        message.validate()?;
        Ok(message)
    }
}

/// Native adapters install only an authorized binding. Stale packets cannot
/// advance its sequence watermark, including after the consumer drains it.
/// Caller must release owned OS input before changing/removing a binding.
pub struct CommandQueue {
    binding: Binding,
    last_sequence: u64,
    queue: VecDeque<(u64, Command)>,
    stopped: bool,
}
impl CommandQueue {
    pub fn new(binding: Binding) -> Result<Self> {
        binding.validate()?;
        Ok(Self {
            binding,
            last_sequence: 0,
            queue: VecDeque::new(),
            stopped: false,
        })
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn push(&mut self, message: Message) -> Result<bool> {
        message.validate_sender(Role::Receiver)?;
        let Message::Command {
            binding,
            sequence,
            command,
        } = message
        else {
            anyhow::bail!("Expected display input command")
        };
        if self.stopped || binding != self.binding || sequence <= self.last_sequence {
            return Ok(false);
        }
        // Verify the serialized envelope bound even when called without RTC.
        Message::Command {
            binding,
            sequence,
            command: command.clone(),
        }
        .encode()?;
        if matches!(
            command,
            Command::Stop
                | Command::Input {
                    event: InputEvent::Release,
                    ..
                }
        ) {
            // Cancellation supersedes unapplied input and must remain admissible
            // even with a full queue. Native owned state still releases exactly once.
            self.queue.clear();
            self.stopped = matches!(command, Command::Stop);
        } else if matches!(command, Command::Move { .. })
            && self
                .queue
                .back()
                .is_some_and(|(_, c)| matches!(c, Command::Move { .. }))
        {
            self.queue.pop_back();
        } else {
            if self.queue.len() >= MAX_PENDING_MESSAGES {
                // Do not replay queued touches if a caller catches the failure.
                // The adapter must release OS state and authorize a fresh binding.
                self.queue.clear();
                self.stopped = true;
                anyhow::bail!("Display input queue is full; release the input lease");
            }
        }
        self.queue.push_back((sequence, command));
        self.last_sequence = sequence;
        Ok(true)
    }
    pub fn pop(&mut self) -> Option<Message> {
        self.queue
            .pop_front()
            .map(|(sequence, command)| Message::Command {
                binding: self.binding.clone(),
                sequence,
                command,
            })
    }
    /// Discard pending commands while retaining stale-packet protection.
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            session: "a".repeat(32),
            epoch: 2,
            revision: 3,
        }
    }
    fn message(sequence: u64, command: Command) -> Message {
        Message::Command {
            binding: binding(),
            sequence,
            command,
        }
    }
    fn motion(x: f64) -> Command {
        Command::Move {
            point: Point { x, y: 0.5 },
        }
    }
    #[test]
    fn legacy_display_grant_defaults_to_no_text_or_shortcuts_and_omits_new_fields() {
        let geometry = Geometry {
            content: Rect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            canvas: Size {
                width: 1000,
                height: 800,
            },
            raster: Size {
                width: 1000,
                height: 800,
            },
            source_size: Size {
                width: 1000,
                height: 800,
            },
        };
        let grant = Message::Granted {
            request: 1,
            binding: binding(),
            geometry,
            text_supported: false,
            shortcut_policy: None,
        };
        let value = serde_json::to_value(&grant).unwrap();
        assert!(value.get("text_supported").is_none());
        assert!(value.get("shortcut_policy").is_none());
        assert_eq!(
            serde_json::from_value::<Message>(value.clone()).unwrap(),
            grant
        );
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Legacy {
            #[serde(rename = "type")]
            kind: String,
            request: u64,
            binding: Binding,
            geometry: Geometry,
        }
        let legacy = serde_json::from_value::<Legacy>(value).unwrap();
        assert_eq!(legacy.kind, "granted");
        assert_eq!(legacy.request, 1);
        legacy.binding.validate().unwrap();
        legacy.geometry.validate().unwrap();
    }
    #[test]
    fn display_input_strict_schema_and_existing_event_validation() {
        let valid = message(
            1,
            Command::Input {
                point: Some(Point { x: 0.5, y: 0.2 }),
                event: InputEvent::Button {
                    button: 0,
                    down: true,
                    clicks: 1,
                },
            },
        );
        assert_eq!(Message::decode(&valid.encode().unwrap()).unwrap(), valid);
        for bytes in [
            br#"{"type":"unknown"}"#.as_slice(),
            br#"{"type":"request","request":1,"display_id":1}"#,
            br#"{"type":"request","request":1,"request":2}"#,
        ] {
            assert!(Message::decode(bytes).is_err());
        }
        for command in [
            motion(f64::NAN),
            motion(1.1),
            Command::Input {
                point: None,
                event: InputEvent::Button {
                    button: 0,
                    down: true,
                    clicks: 1,
                },
            },
            Command::Input {
                point: Some(Point { x: 0.0, y: 0.0 }),
                event: InputEvent::Scroll {
                    dx: i32::MIN,
                    dy: 0,
                },
            },
            Command::Input {
                point: None,
                event: InputEvent::MacKey {
                    code: 54,
                    down: true,
                    repeat: false,
                },
            },
        ] {
            assert!(message(1, command).encode().is_err());
        }
        assert!(message(0, motion(0.5)).encode().is_err());
        assert!(Message::decode(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
        assert!(valid.validate_sender(Role::Source).is_err());
        assert!(Message::Applied {
            binding: binding(),
            sequence: 1
        }
        .validate_sender(Role::Receiver)
        .is_err());
    }
    #[test]
    fn display_input_geometry_matches_current_logical_and_retina_config() {
        let config = Config {
            mode: crate::session::display::Mode::Mirror,
            width: 1000,
            height: 800,
            retina: true,
            fps: 60,
            bitrate: 8_000_000,
            auto_size: false,
            codec: crate::session::display::VideoCodec::H264,
            separate_cursor: true,
        };
        let mut geometry = Geometry {
            content: Rect {
                x: 0.0,
                y: 0.1875,
                width: 1.0,
                height: 0.625,
            },
            canvas: Size {
                width: 1000,
                height: 800,
            },
            raster: Size {
                width: 2000,
                height: 1600,
            },
            source_size: Size {
                width: 2000,
                height: 1000,
            },
        };
        geometry.validate_for_config(&config).unwrap();
        geometry.raster.width = 1000;
        assert!(geometry.validate().is_err());
        geometry.raster = geometry.canvas;
        assert!(geometry.validate_for_config(&config).is_err());
        geometry.content.height = 1.0;
        assert!(geometry.validate().is_err());
    }
    #[test]
    fn display_input_queue_preserves_transition_order_and_stale_watermarks() {
        let mut queue = CommandQueue::new(binding()).unwrap();
        queue.push(message(1, motion(0.1))).unwrap();
        queue.push(message(2, motion(0.2))).unwrap();
        let down = Command::Input {
            point: Some(Point { x: 0.2, y: 0.5 }),
            event: InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            },
        };
        queue.push(message(3, down.clone())).unwrap();
        queue.push(message(4, motion(0.3))).unwrap();
        assert_eq!(queue.pop(), Some(message(2, motion(0.2))));
        assert_eq!(queue.pop(), Some(message(3, down)));
        assert_eq!(queue.pop(), Some(message(4, motion(0.3))));
        assert!(!queue.push(message(3, motion(0.9))).unwrap());
        let mut stale = binding();
        stale.revision += 1;
        assert!(!queue
            .push(Message::Command {
                binding: stale,
                sequence: 100,
                command: motion(0.9)
            })
            .unwrap());
        let mut stale = binding();
        stale.epoch += 1;
        assert!(!queue
            .push(Message::Command {
                binding: stale,
                sequence: 100,
                command: motion(0.9)
            })
            .unwrap());
        let mut stale = binding();
        stale.session = "b".repeat(32);
        assert!(!queue
            .push(Message::Command {
                binding: stale,
                sequence: 100,
                command: motion(0.9)
            })
            .unwrap());
        assert!(queue
            .push(message(
                5,
                Command::Input {
                    point: None,
                    event: InputEvent::Release
                }
            ))
            .unwrap());
        queue.clear();
        assert!(!queue.push(message(5, motion(0.9))).unwrap());
        assert!(queue.is_empty());
    }
    #[test]
    fn display_input_queue_is_bounded_without_dropping_releases() {
        let mut queue = CommandQueue::new(binding()).unwrap();
        for sequence in 1..=MAX_PENDING_MESSAGES as u64 {
            queue.push(message(sequence, Command::Alive)).unwrap();
        }
        let release = message(
            65,
            Command::Input {
                point: None,
                event: InputEvent::Release,
            },
        );
        assert!(queue.push(release.clone()).unwrap());
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.queue.back().unwrap().1,
            Command::Input {
                point: None,
                event: InputEvent::Release
            }
        );
        assert!(!queue.push(release).unwrap());
        queue.push(message(66, Command::Stop)).unwrap();
        assert!(!queue.push(message(67, motion(0.5))).unwrap());
        let mut queue = CommandQueue::new(binding()).unwrap();
        for sequence in 1..=MAX_PENDING_MESSAGES as u64 {
            queue.push(message(sequence, Command::Alive)).unwrap();
        }
        assert!(queue.push(message(65, Command::Alive)).is_err());
        assert!(queue.is_empty());
        assert!(!queue.push(message(66, motion(0.5))).unwrap());
    }
}
