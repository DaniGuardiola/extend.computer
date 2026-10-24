//! Receiver-side display input. The media worker alone writes to RTC; JNI only
//! validates and queues commands for the exact rendered configuration.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::{
        input::{Binding, Command, CommandQueue, Geometry, Message, Reason, Role},
        Transport,
    },
    input::{InputEvent, ShortcutPolicy},
    session::display::Config,
};
use std::time::{Duration, Instant};

pub(super) struct ReceiverInput {
    supported: bool,
    text: bool,
    text_protocol: bool,
    shortcut_protocol: bool,
    shortcut_policy: Option<ShortcutPolicy>,
    enabled: bool,
    state: &'static str,
    reason: Option<Reason>,
    request: u64,
    request_started: Option<Instant>,
    request_sent: Option<Instant>,
    binding: Option<Binding>,
    geometry: Option<Geometry>,
    queue: Option<CommandQueue>,
    pending: Option<Message>,
    stop: Option<Message>,
    sequence: u64,
    sent_sequence: u64,
    applied: u64,
    last_applied: Option<Instant>,
    last_alive: Instant,
}
impl ReceiverInput {
    pub(super) fn with_capabilities(supported: bool, text: bool, shortcuts: bool) -> Self {
        let mut value = Self::with_text(supported, text);
        value.shortcut_protocol = shortcuts;
        value
    }
    pub(super) fn with_text(supported: bool, text: bool) -> Self {
        let mut value = Self::new(supported);
        value.text_protocol = text;
        value
    }
    pub(super) fn new(supported: bool) -> Self {
        Self {
            supported,
            text: false,
            text_protocol: false,
            shortcut_protocol: false,
            shortcut_policy: None,
            enabled: false,
            state: if supported { "disabled" } else { "unsupported" },
            reason: None,
            request: 0,
            request_started: None,
            request_sent: None,
            binding: None,
            geometry: None,
            queue: None,
            pending: None,
            stop: None,
            sequence: 0,
            sent_sequence: 0,
            applied: 0,
            last_applied: None,
            last_alive: Instant::now(),
        }
    }
    pub(super) fn snapshot(&self, revision: u64) -> serde_json::Value {
        serde_json::json!({
            "text_supported":self.text,"shortcut_policy":self.shortcut_policy,"state":self.state,"revision":revision,"binding":self.binding,
            "geometry":self.geometry,"reason":self.reason,"applied_sequence":self.applied
        })
    }
    pub(super) fn needs_poll(&self) -> bool {
        self.enabled || self.stop.is_some()
    }
    pub(super) fn enable(&mut self, enabled: bool) -> Result<()> {
        if !enabled {
            self.suspend(Reason::Stopped);
            self.state = if self.supported {
                "disabled"
            } else {
                "unsupported"
            };
            return Ok(());
        }
        ensure!(
            self.supported,
            "This display does not support touch control"
        );
        if self.enabled {
            return Ok(());
        }
        self.enabled = true;
        self.reason = None;
        self.request = self
            .request
            .checked_add(1)
            .context("Input request overflow")?;
        self.request_sent = None;
        self.request_started = Some(Instant::now());
        self.state = "requesting";
        Ok(())
    }
    pub(super) fn suspend(&mut self, reason: Reason) {
        if let Some(binding) = self.binding.take() {
            self.sequence = self.sequence.saturating_add(1);
            self.stop = Some(Message::Command {
                binding,
                sequence: self.sequence,
                command: Command::Stop,
            });
        }
        self.enabled = false;
        self.text = false;
        self.shortcut_policy = None;
        self.geometry = None;
        self.queue = None;
        self.pending = None;
        self.request_sent = None;
        self.request_started = None;
        self.last_applied = None;
        self.reason = Some(reason);
        self.state = if self.supported {
            "suspended"
        } else {
            "unsupported"
        };
    }
    pub(super) fn closed(&mut self) {
        self.suspend(Reason::Stopped);
        self.supported = false;
        self.stop = None;
        self.state = "unsupported";
    }
    pub(super) fn enqueue(
        &mut self,
        revision: u64,
        current_revision: u64,
        rendered: bool,
        command: Command,
    ) -> Result<()> {
        command.validate()?;
        ensure!(
            revision == current_revision,
            "Display configuration replaced"
        );
        if matches!(command, Command::Stop) {
            self.suspend(Reason::Stopped);
            return Ok(());
        }
        ensure!(
            rendered && self.enabled && self.state == "granted",
            "Touch control unavailable"
        );
        if matches!(
            &command,
            Command::Input {
                event: InputEvent::Text { .. },
                ..
            }
        ) {
            ensure!(self.text, "This peer does not support committed text");
        }
        let binding = self.binding.clone().context("Touch control unavailable")?;
        ensure!(
            binding.revision == revision,
            "Display input configuration replaced"
        );
        let point = match &command {
            Command::Move { point } => Some(*point),
            // command.validate requires point-less keys/modifiers and atomic
            // target points for buttons/scroll. All share this rendered lease.
            Command::Input { point, .. } => *point,
            Command::Alive => None,
            Command::Stop => unreachable!(),
        };
        if let Some(point) = point {
            let content = self
                .geometry
                .as_ref()
                .context("Missing input geometry")?
                .content;
            ensure!(
                point.x >= content.x
                    && point.y >= content.y
                    && point.x < content.x + content.width
                    && point.y < content.y + content.height,
                "Touch is outside the captured display"
            );
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("Input sequence overflow")?;
        if matches!(
            command,
            Command::Input {
                event: InputEvent::Release,
                ..
            }
        ) {
            self.pending = None;
        }
        let message = Message::Command {
            binding,
            sequence: self.sequence,
            command,
        };
        let result = self
            .queue
            .as_mut()
            .context("Touch control unavailable")?
            .push(message);
        if result.is_err() {
            self.suspend(Reason::Stopped);
        }
        result.map(|_| ())
    }
    pub(super) fn receive(
        &mut self,
        message: Message,
        config: &Config,
        revision: u64,
    ) -> Result<()> {
        message.validate_sender(Role::Source)?;
        match message {
            Message::Granted {
                request,
                binding,
                geometry,
                text_supported,
                shortcut_policy,
            } => {
                ensure!(
                    shortcut_policy.is_none() || self.shortcut_protocol,
                    "Unnegotiated shortcut grant"
                );
                ensure!(
                    !text_supported || self.text_protocol,
                    "Unnegotiated text grant"
                );
                if self.binding.as_ref() == Some(&binding) {
                    if self.enabled
                        && self.state == "granted"
                        && request == self.request
                        && self.geometry.as_ref() == Some(&geometry)
                        && self.text == text_supported
                        && self.shortcut_policy == shortcut_policy
                    {
                        return Ok(());
                    }
                    self.suspend(Reason::Changed);
                    anyhow::bail!("Display input grant changed without a new binding");
                }
                if !self.enabled
                    || self.state != "requesting"
                    || request != self.request
                    || binding.revision != revision
                {
                    // A late grant cannot revive an old native gesture or lease.
                    if self.stop.is_none() {
                        self.stop = Some(Message::Command {
                            binding,
                            sequence: 1,
                            command: Command::Stop,
                        });
                    }
                    return Ok(());
                }
                geometry.validate_for_config(config)?;
                self.text = text_supported;
                self.shortcut_policy = shortcut_policy;
                self.queue = Some(CommandQueue::new(binding.clone())?);
                self.binding = Some(binding);
                self.geometry = Some(geometry);
                self.sequence = 0;
                self.sent_sequence = 0;
                self.applied = 0;
                self.request_sent = None;
                self.request_started = None;
                self.last_applied = Some(Instant::now());
                self.last_alive = Instant::now();
                self.state = "granted";
            }
            Message::Denied { request, reason }
                if request == self.request && self.state == "requesting" =>
            {
                self.suspend(reason);
                self.state = "denied";
            }
            Message::Applied { binding, sequence } if self.binding.as_ref() == Some(&binding) => {
                ensure!(
                    sequence <= self.sent_sequence,
                    "Invalid input acknowledgement"
                );
                if sequence > self.applied {
                    self.applied = sequence;
                    self.last_applied = Some(Instant::now());
                }
            }
            Message::Revoked { binding, reason } if self.binding.as_ref() == Some(&binding) => {
                self.suspend(reason);
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn flush(&mut self, transport: &mut Transport, rendered: bool) -> Result<()> {
        // A capability can be negotiated before its RTC channel opens. Native
        // UI must still finish boundedly when opening/writing never succeeds.
        if self.state == "requesting"
            && self
                .request_started
                .is_some_and(|started| started.elapsed() >= Duration::from_secs(3))
        {
            self.suspend(Reason::Timeout);
            self.state = "denied";
        }
        if self.state == "granted"
            && self
                .last_applied
                .is_some_and(|time| time.elapsed() >= Duration::from_secs(2))
        {
            self.suspend(Reason::Timeout);
        }
        if let Some(stop) = self.stop.as_ref() {
            if !transport.write_input(stop)? {
                return Ok(());
            }
            self.stop = None;
        }
        if self.state == "requesting" && rendered {
            if let Some(sent) = self.request_sent {
                if sent.elapsed() >= Duration::from_secs(3) {
                    self.suspend(Reason::Timeout);
                    self.state = "denied";
                }
            } else if transport.write_input(&Message::Request {
                request: self.request,
            })? {
                self.request_sent = Some(Instant::now());
            }
        }
        if self.state != "granted" {
            return Ok(());
        }
        if self.last_alive.elapsed() >= Duration::from_millis(250) {
            let revision = self
                .binding
                .as_ref()
                .context("Missing input binding")?
                .revision;
            self.enqueue(revision, revision, rendered, Command::Alive)?;
            self.last_alive = Instant::now();
        }
        // Limit each pass so input cannot starve video decoding/cursor delivery.
        for _ in 0..16 {
            if self.pending.is_none() {
                self.pending = self.queue.as_mut().and_then(CommandQueue::pop);
            }
            let Some(message) = self.pending.as_ref() else {
                break;
            };
            if !transport.write_input(message)? {
                break;
            }
            if let Message::Command { sequence, .. } = message {
                self.sent_sequence = *sequence;
            }
            self.pending = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{
        display::input::{Point, Rect, Size},
        session::display::{Mode, VideoCodec},
    };
    fn config() -> Config {
        Config {
            mode: Mode::Mirror,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 8_000_000,
            retina: false,
            auto_size: false,
            separate_cursor: true,
            codec: VideoCodec::H264,
        }
    }
    fn grant(input: &mut ReceiverInput, revision: u64) -> Binding {
        input.enable(true).unwrap();
        let binding = Binding::new(1, revision);
        input
            .receive(
                Message::Granted {
                    text_supported: input.text_protocol,
                    shortcut_policy: None,
                    request: input.request,
                    binding: binding.clone(),
                    geometry: Geometry {
                        content: Rect {
                            x: 0.0,
                            y: 0.1,
                            width: 1.0,
                            height: 0.8,
                        },
                        canvas: Size {
                            width: 1280,
                            height: 800,
                        },
                        raster: Size {
                            width: 1280,
                            height: 800,
                        },
                        source_size: Size {
                            width: 1280,
                            height: 640,
                        },
                    },
                },
                &config(),
                revision,
            )
            .unwrap();
        binding
    }
    #[test]
    fn grant_cannot_revive_disabled_or_reconfigured_view() {
        let mut input = ReceiverInput::new(true);
        let binding = grant(&mut input, 2);
        input.enable(false).unwrap();
        assert!(input.binding.is_none());
        assert!(matches!(
            input.stop,
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        let geometry = Geometry {
            content: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            canvas: Size {
                width: 1280,
                height: 800,
            },
            raster: Size {
                width: 1280,
                height: 800,
            },
            source_size: Size {
                width: 1280,
                height: 800,
            },
        };
        input
            .receive(
                Message::Granted {
                    text_supported: false,
                    shortcut_policy: None,
                    request: input.request,
                    binding,
                    geometry,
                },
                &config(),
                3,
            )
            .unwrap();
        assert!(input.binding.is_none());
        assert_eq!(input.state, "disabled");
    }
    #[test]
    fn committed_text_requires_capability_exact_rendered_lease_and_no_point() {
        let text = Command::Input {
            point: None,
            event: InputEvent::Text {
                value: "é🙂中文".into(),
            },
        };
        let mut old = ReceiverInput::new(true);
        grant(&mut old, 3);
        assert!(old.enqueue(3, 3, true, text.clone()).is_err());
        assert_eq!(old.sequence, 0);
        let mut input = ReceiverInput::with_text(true, true);
        assert!(input.enqueue(3, 3, true, text.clone()).is_err());
        grant(&mut input, 3);
        assert!(input.enqueue(2, 3, true, text.clone()).is_err());
        assert!(input.enqueue(3, 3, false, text.clone()).is_err());
        assert!(input
            .enqueue(
                3,
                3,
                true,
                Command::Input {
                    point: Some(Point { x: 0.5, y: 0.5 }),
                    event: InputEvent::Text { value: "é".into() }
                }
            )
            .is_err());
        assert_eq!(input.sequence, 0);
        input.enqueue(3, 3, true, text).unwrap();
        assert_eq!(input.sequence, 1);
        input.enable(false).unwrap();
        assert!(input
            .enqueue(
                3,
                3,
                true,
                Command::Input {
                    point: None,
                    event: InputEvent::Text {
                        value: "late".into()
                    }
                }
            )
            .is_err());
    }
    #[test]
    fn native_commands_require_rendered_revision_and_reject_bars() {
        let mut input = ReceiverInput::new(true);
        grant(&mut input, 2);
        let movement = Command::Move {
            point: Point { x: 0.5, y: 0.5 },
        };
        assert!(input.enqueue(1, 2, true, movement.clone()).is_err());
        assert!(input.enqueue(1, 2, true, Command::Stop).is_err());
        assert_eq!(input.state, "granted");
        assert!(input.enqueue(2, 2, false, movement.clone()).is_err());
        assert!(input
            .enqueue(
                2,
                2,
                true,
                Command::Move {
                    point: Point { x: 0.5, y: 0.05 }
                }
            )
            .is_err());
        assert!(input
            .enqueue(
                2,
                2,
                true,
                Command::Input {
                    point: Some(Point { x: 0.5, y: 0.5 }),
                    event: InputEvent::MacKey {
                        code: 0,
                        down: true,
                        repeat: false
                    }
                }
            )
            .is_err());
        input.enqueue(2, 2, true, movement).unwrap();
        assert_eq!(input.queue.as_ref().unwrap().len(), 1);
        input.suspend(Reason::Changed);
        assert!(input.enqueue(2, 2, true, Command::Alive).is_err());
    }
    #[test]
    fn keyboard_commands_require_granted_rendered_lease_and_preserve_order_without_points() {
        let events = [
            InputEvent::Modifiers { mask: 1 },
            InputEvent::MacKey {
                code: 0,
                down: true,
                repeat: false,
            },
            InputEvent::MacKey {
                code: 0,
                down: false,
                repeat: false,
            },
            InputEvent::Modifiers { mask: 0 },
        ];
        let key = Command::Input {
            point: None,
            event: events[1].clone(),
        };
        let mut input = ReceiverInput::new(true);
        assert!(input.enqueue(3, 3, true, key.clone()).is_err());
        let binding = grant(&mut input, 3);
        assert!(input.enqueue(2, 3, true, key.clone()).is_err());
        assert!(input.enqueue(3, 3, false, key.clone()).is_err());
        for event in &events {
            assert!(input
                .enqueue(
                    3,
                    3,
                    true,
                    Command::Input {
                        point: Some(Point { x: 0.5, y: 0.5 }),
                        event: event.clone(),
                    }
                )
                .is_err());
            input
                .enqueue(
                    3,
                    3,
                    true,
                    Command::Input {
                        point: None,
                        event: event.clone(),
                    },
                )
                .unwrap();
        }
        for (index, event) in events.iter().enumerate() {
            assert_eq!(
                input.queue.as_mut().unwrap().pop(),
                Some(Message::Command {
                    binding: binding.clone(),
                    sequence: index as u64 + 1,
                    command: Command::Input {
                        point: None,
                        event: event.clone()
                    },
                })
            );
        }
        input.enable(false).unwrap();
        assert!(input.queue.is_none() && input.binding.is_none());
        assert!(matches!(
            input.stop,
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        assert!(input.enqueue(3, 3, true, key).is_err());
    }

    #[test]
    fn keyboard_queue_overload_stops_lease_instead_of_leaving_held_keys() {
        let mut input = ReceiverInput::new(true);
        grant(&mut input, 0);
        let key = Command::Input {
            point: None,
            event: InputEvent::MacKey {
                code: 0,
                down: true,
                repeat: false,
            },
        };
        let mut overloaded = false;
        for _ in 0..4096 {
            if input.enqueue(0, 0, true, key.clone()).is_err() {
                overloaded = true;
                break;
            }
        }
        assert!(overloaded);
        assert_eq!(input.state, "suspended");
        assert!(input.binding.is_none() && input.queue.is_none() && input.pending.is_none());
        assert!(matches!(
            input.stop,
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
    }

    #[test]
    fn release_survives_flood_and_ack_cannot_claim_unsent_sequence() {
        let mut input = ReceiverInput::new(true);
        let binding = grant(&mut input, 0);
        for _ in 0..10_000 {
            input
                .enqueue(
                    0,
                    0,
                    true,
                    Command::Move {
                        point: Point { x: 0.5, y: 0.5 },
                    },
                )
                .unwrap();
        }
        assert_eq!(input.queue.as_ref().unwrap().len(), 1);
        input
            .enqueue(
                0,
                0,
                true,
                Command::Input {
                    point: None,
                    event: InputEvent::Release,
                },
            )
            .unwrap();
        assert_eq!(input.queue.as_ref().unwrap().len(), 1);
        // Allocated sequence is not an acknowledgement until RTC accepted it.
        assert!(input
            .receive(
                Message::Applied {
                    binding: binding.clone(),
                    sequence: input.sequence
                },
                &config(),
                0
            )
            .is_err());
        assert!(input
            .receive(
                Message::Applied {
                    binding,
                    sequence: 10_002
                },
                &config(),
                0
            )
            .is_err());
        input.closed();
        assert!(!input.supported && input.queue.is_none());
    }
    #[test]
    fn duplicate_grant_is_idempotent_and_cannot_stop_live_lease() {
        let mut input = ReceiverInput::new(true);
        let binding = grant(&mut input, 0);
        let geometry = input.geometry.clone().unwrap();
        let duplicate = Message::Granted {
            text_supported: false,
            shortcut_policy: None,
            request: input.request,
            binding: binding.clone(),
            geometry: geometry.clone(),
        };
        input.receive(duplicate.clone(), &config(), 0).unwrap();
        assert!(input.stop.is_none());
        input
            .enqueue(
                0,
                0,
                true,
                Command::Move {
                    point: Point { x: 0.5, y: 0.5 },
                },
            )
            .unwrap();
        input.receive(duplicate, &config(), 0).unwrap();
        assert_eq!(input.sequence, 1);
        assert_eq!(input.queue.as_ref().unwrap().len(), 1);
        let mut altered = geometry;
        altered.content.y = 0.0;
        assert!(input
            .receive(
                Message::Granted {
                    text_supported: false,
                    shortcut_policy: None,
                    request: input.request,
                    binding,
                    geometry: altered
                },
                &config(),
                0
            )
            .is_err());
        assert!(input.binding.is_none());
        assert!(input.stop.is_some());
    }
    #[test]
    fn unopened_channel_or_backpressured_stop_cannot_leave_request_pending_forever() {
        let mut input = ReceiverInput::new(true);
        let binding = grant(&mut input, 0);
        input.suspend(Reason::Changed);
        input.enable(true).unwrap();
        input.request_started = Some(Instant::now() - Duration::from_secs(4));
        let mut transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        // No channel exists and the old lease stop cannot be written; timeout
        // is evaluated before that failure and never resurrects its grant.
        assert!(input.flush(&mut transport, true).is_err());
        assert_eq!(input.state, "denied");
        assert!(!input.enabled && input.binding.is_none());
        assert!(matches!(input.stop,Some(Message::Command{binding:ref old,..}) if *old == binding));
    }
    #[test]
    fn old_grant_cannot_downgrade_release_or_delay_current_ack_timeout() {
        let mut input = ReceiverInput::new(true);
        let binding = grant(&mut input, 0);
        let geometry = input.geometry.clone().unwrap();
        input.sequence = 12;
        input.sent_sequence = 12;
        input.enable(false).unwrap();
        let old = Message::Granted {
            text_supported: false,
            shortcut_policy: None,
            request: input.request,
            binding: binding.clone(),
            geometry: geometry.clone(),
        };
        input.receive(old, &config(), 0).unwrap();
        assert!(matches!(
            input.stop,
            Some(Message::Command { sequence: 13, .. })
        ));
        input.stop = None;
        let active = grant(&mut input, 0);
        input
            .receive(
                Message::Granted {
                    text_supported: false,
                    shortcut_policy: None,
                    request: 0,
                    binding,
                    geometry,
                },
                &config(),
                0,
            )
            .unwrap();
        input.last_applied = Some(Instant::now() - Duration::from_secs(3));
        let mut transport = Transport::new("127.0.0.1".parse().unwrap()).unwrap();
        assert!(input.flush(&mut transport, true).is_err());
        assert_eq!(input.state, "suspended");
        assert!(matches!(input.stop,Some(Message::Command{binding:ref old,..}) if *old==active));
    }
}
