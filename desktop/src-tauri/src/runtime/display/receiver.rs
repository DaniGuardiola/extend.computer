//! Receiver-side input leases. Native viewer intent never authorizes OS input:
//! only the authenticated source's current grant installs a binding.
use anyhow::{ensure, Result};
use extend_computer_agent::{
    display::input::{Binding, Command, Geometry, Message, Rect},
    input::{InputEvent, ShortcutPolicy},
    session::display::Config,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) const TIMEOUT: Duration = Duration::from_secs(2);
const ALIVE_INTERVAL: Duration = Duration::from_millis(500);
const WINDOW: usize = 4;
const QUEUE: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum NativeEvent {
    Focus { active: bool, revision: u64 },
    Input { command: Command, revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct State {
    #[serde(rename = "type")]
    kind: &'static str,
    pub available: bool,
    pub enabled: bool,
    pub focused: bool,
    pub text_supported: bool,
    pub shortcut_policy: Option<ShortcutPolicy>,
    pub content: Rect,
    pub revision: u64,
}

struct Lease {
    binding: Binding,
    geometry: Geometry,
    text_supported: bool,
    shortcut_policy: Option<ShortcutPolicy>,
    sequence: u64,
    submitted: u64,
    acknowledged: u64,
    pending: VecDeque<(u64, Instant)>,
    last_sent: Instant,
}

pub(super) struct Receiver {
    available: bool,
    revision: u64,
    text_negotiated: bool,
    shortcuts_negotiated: bool,
    focused: bool,
    request_sequence: u64,
    request: Option<(u64, Instant)>,
    lease: Option<Lease>,
    commands: VecDeque<(Command, Instant)>,
    control: VecDeque<Message>,
    unsent: Option<(Message, Instant)>,
}

impl Receiver {
    pub fn new(available: bool, text_negotiated: bool, shortcuts_negotiated: bool) -> Self {
        Self {
            available,
            revision: 1,
            text_negotiated,
            shortcuts_negotiated,
            focused: false,
            request_sequence: 0,
            request: None,
            lease: None,
            commands: VecDeque::new(),
            control: VecDeque::new(),
            unsent: None,
        }
    }

    pub fn state(&self) -> State {
        State {
            kind: "input_state",
            available: self.available,
            enabled: self.focused && self.lease.is_some(),
            focused: self.focused,
            text_supported: self.available
                && self.text_negotiated
                && self.lease.as_ref().is_none_or(|lease| lease.text_supported),
            shortcut_policy: self.lease.as_ref().and_then(|lease| lease.shortcut_policy),
            content: self.lease.as_ref().map_or(
                Rect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                |lease| lease.geometry.content,
            ),
            revision: self.revision,
        }
    }

    pub fn reconfigure(&mut self, now: Instant) -> Result<()> {
        let previous = self.revision;
        self.suspend(now);
        if previous == self.revision {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Display viewer input revision exhausted"))?;
        }
        Ok(())
    }

    pub fn close(&mut self, now: Instant) {
        self.suspend(now);
        self.available = false;
        self.unsent = None;
        self.control.clear();
    }

    fn stop_message(lease: &Lease) -> Option<Message> {
        lease
            .sequence
            .checked_add(1)
            .map(|sequence| Message::Command {
                binding: lease.binding.clone(),
                sequence,
                command: Command::Stop,
            })
    }

    /// Focus/resize/overflow cancels queued intent before releasing the lease.
    /// Stop supersedes outstanding commands even when their ACK window is full.
    pub fn suspend(&mut self, now: Instant) {
        let had_intent = self.focused || self.request.is_some() || self.lease.is_some();
        // A request and denial can occur before the next native-state snapshot.
        // The revision still proves that pending native ownership must reset.
        if had_intent {
            if let Some(revision) = self.revision.checked_add(1) {
                self.revision = revision;
            } else {
                self.available = false;
            }
        }
        self.focused = false;
        self.request = None;
        self.commands.clear();
        // Repeated focus-loss notifications must not erase the pending release.
        self.unsent = self.unsent.take().filter(|(message, at)| {
            matches!(
                message,
                Message::Command {
                    command: Command::Stop,
                    ..
                }
            ) && now.duration_since(*at) < TIMEOUT
        });
        self.control.retain(|message| {
            matches!(
                message,
                Message::Command {
                    command: Command::Stop,
                    ..
                }
            )
        });
        if let Some(lease) = self.lease.take() {
            if let Some(message) = Self::stop_message(&lease) {
                self.control.push_front(message);
            }
        }
    }

    fn focus(&mut self, active: bool, now: Instant) -> Result<()> {
        if !active {
            self.suspend(now);
            return Ok(());
        }
        if !self.available {
            return Ok(());
        }
        self.focused = true;
        if self.lease.is_none() && self.request.is_none() {
            self.request_sequence = self
                .request_sequence
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Display input request counter exhausted"))?;
            self.request = Some((self.request_sequence, now));
            ensure!(
                self.control.len() < QUEUE,
                "Display input control queue overflow"
            );
            self.control.push_back(Message::Request {
                request: self.request_sequence,
            });
        }
        Ok(())
    }

    fn validate_command(&self, command: &Command) -> Result<()> {
        command.validate()?;
        if let Command::Input {
            event: InputEvent::Text { .. },
            ..
        } = command
        {
            ensure!(
                self.state().text_supported,
                "Display text input is unavailable"
            );
        }
        if let Some(lease) = &self.lease {
            let point = match command {
                Command::Move { point }
                | Command::Input {
                    point: Some(point), ..
                } => Some(point),
                _ => None,
            };
            if let Some(point) = point {
                let rect = lease.geometry.content;
                ensure!(
                    point.x >= rect.x
                        && point.y >= rect.y
                        && point.x <= rect.x + rect.width
                        && point.y <= rect.y + rect.height,
                    "Display input point is outside the captured content"
                );
            }
        }
        Ok(())
    }

    pub fn native(&mut self, event: NativeEvent, now: Instant) -> Result<()> {
        match event {
            NativeEvent::Focus { active, revision } => {
                if revision != self.revision {
                    return Ok(());
                }
                self.focus(active, now)
            }
            NativeEvent::Input { command, revision } => {
                if revision != self.revision {
                    return Ok(());
                }
                if !self.focused || !self.available {
                    return Ok(());
                }
                if let Err(error) = self.validate_command(&command) {
                    self.suspend(now);
                    return Err(error);
                }
                if matches!(
                    command,
                    Command::Stop
                        | Command::Input {
                            event: InputEvent::Release,
                            ..
                        }
                ) {
                    self.suspend(now);
                    return Ok(());
                }
                if matches!(command, Command::Move { .. })
                    && self
                        .commands
                        .back()
                        .is_some_and(|(previous, _)| matches!(previous, Command::Move { .. }))
                {
                    self.commands.pop_back();
                }
                if self.commands.len() >= QUEUE {
                    self.suspend(now);
                    anyhow::bail!("Display viewer input queue overflow");
                }
                self.commands.push_back((command, now));
                Ok(())
            }
        }
    }

    pub fn remote(&mut self, message: Message, config: &Config, now: Instant) -> Result<()> {
        message.validate_sender(extend_computer_agent::display::input::Role::Source)?;
        match message {
            Message::Granted {
                request,
                binding,
                geometry,
                text_supported,
                shortcut_policy,
            } => {
                let current = self.focused
                    && self.request.is_some_and(|(id, started)| {
                        id == request && now.duration_since(started) < TIMEOUT
                    });
                // A grant may race focus loss or a request timeout. Explicitly
                // retire that authenticated source binding, never install it.
                if !current {
                    if self
                        .lease
                        .as_ref()
                        .is_none_or(|lease| lease.binding != binding)
                    {
                        if self.control.len() >= QUEUE {
                            self.control.clear();
                            self.suspend(now);
                        }
                        self.control.push_back(Message::Command {
                            binding,
                            sequence: 1,
                            command: Command::Stop,
                        });
                    }
                    return Ok(());
                }
                let valid = geometry.validate_for_config(config).and_then(|()| {
                    ensure!(
                        !text_supported || self.text_negotiated,
                        "Unnegotiated display text grant"
                    );
                    ensure!(
                        shortcut_policy.is_none() || self.shortcuts_negotiated,
                        "Unnegotiated display shortcut grant"
                    );
                    Ok(())
                });
                if let Err(error) = valid {
                    self.suspend(now);
                    self.control.push_back(Message::Command {
                        binding,
                        sequence: 1,
                        command: Command::Stop,
                    });
                    return Err(error);
                }
                self.request = None;
                self.lease = Some(Lease {
                    binding,
                    geometry,
                    text_supported,
                    shortcut_policy,
                    sequence: 0,
                    submitted: 0,
                    acknowledged: 0,
                    pending: VecDeque::new(),
                    last_sent: now,
                });
                // Pre-grant intent is still checked against the granted content
                // rectangle before any command reaches the source adapter.
                if self.commands.iter().any(|(command, received)| {
                    now.duration_since(*received) >= TIMEOUT
                        || self.validate_command(command).is_err()
                }) {
                    self.suspend(now);
                }
            }
            Message::Denied { request, .. } => {
                if self.request.is_some_and(|(id, _)| id == request) {
                    self.suspend(now);
                }
            }
            Message::Revoked { binding, .. } => {
                if self
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.binding == binding)
                {
                    self.suspend(now);
                }
            }
            Message::Applied { binding, sequence } => {
                if let Some(lease) = self.lease.as_mut().filter(|lease| lease.binding == binding) {
                    if sequence > lease.submitted {
                        self.suspend(now);
                        anyhow::bail!("Display input acknowledgement exceeds submitted sequence");
                    }
                    if sequence > lease.acknowledged {
                        lease.acknowledged = sequence;
                        while lease
                            .pending
                            .front()
                            .is_some_and(|(old, _)| *old <= sequence)
                        {
                            lease.pending.pop_front();
                        }
                    }
                }
            }
            _ => unreachable!("Source message validation excludes receiver messages"),
        }
        Ok(())
    }

    pub fn message(&mut self, now: Instant) -> Option<&Message> {
        let expired = self
            .request
            .is_some_and(|(_, at)| now.duration_since(at) >= TIMEOUT)
            || self
                .commands
                .front()
                .is_some_and(|(_, at)| now.duration_since(*at) >= TIMEOUT)
            || self.lease.as_ref().is_some_and(|lease| {
                lease
                    .pending
                    .front()
                    .is_some_and(|(_, at)| now.duration_since(*at) >= TIMEOUT)
            });
        if expired {
            self.suspend(now);
        }
        if self
            .unsent
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) >= TIMEOUT)
        {
            self.suspend(now);
        }
        if self.unsent.is_none() {
            if let Some(message) = self.control.pop_front() {
                self.unsent = Some((message, now));
            } else if let Some(lease) = self
                .lease
                .as_mut()
                .filter(|lease| lease.pending.len() < WINDOW)
            {
                let command = self
                    .commands
                    .pop_front()
                    .map(|(command, _)| command)
                    .or_else(|| {
                        (now.duration_since(lease.last_sent) >= ALIVE_INTERVAL)
                            .then_some(Command::Alive)
                    });
                if let Some(command) = command {
                    if let Some(sequence) = lease.sequence.checked_add(1) {
                        lease.sequence = sequence;
                        self.unsent = Some((
                            Message::Command {
                                binding: lease.binding.clone(),
                                sequence,
                                command,
                            },
                            now,
                        ));
                    } else {
                        self.suspend(now);
                    }
                }
            }
        }
        self.unsent.as_ref().map(|(message, _)| message)
    }

    pub fn accepted(&mut self, now: Instant) {
        if let Some((
            Message::Command {
                binding, sequence, ..
            },
            _,
        )) = self.unsent.take()
        {
            if let Some(lease) = self.lease.as_mut().filter(|lease| lease.binding == binding) {
                lease.pending.push_back((sequence, now));
                lease.submitted = sequence;
                lease.last_sent = now;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_pre_recovery_grant_at_two_second_boundary_never_replays_queued_click() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        let request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        receiver.accepted(now);
        input(&mut receiver, button(true), now).unwrap();
        // Even if this endpoint's media loop did not tick during the outage,
        // its original request clock is retained. Strict equality expires it.
        let later = now + Duration::from_secs(2);
        let binding = Binding::new(1, 0);
        receiver
            .remote(
                Message::Granted {
                    request,
                    binding: binding.clone(),
                    geometry: geometry(&config()),
                    text_supported: true,
                    shortcut_policy: Some(ShortcutPolicy::Command),
                },
                &config(),
                later,
            )
            .unwrap();
        assert!(receiver.lease.is_none());
        assert!(!receiver.state().enabled);
        assert!(
            matches!(receiver.message(later),Some(Message::Command {binding:retired,command:Command::Stop,..}) if retired==&binding)
        );
        receiver.accepted(later);
        assert!(receiver.commands.is_empty());
        assert!(receiver.message(later).is_none());
    }

    #[test]
    fn old_grant_after_cooldown_cannot_authorize_new_request_or_old_pregrant_commands() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        let old_request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        receiver.accepted(now);
        input(&mut receiver, button(true), now).unwrap();
        let later = now + Duration::from_secs(3);
        assert!(receiver.message(later).is_none());
        focus(&mut receiver, later);
        let current_request = match receiver.message(later).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        assert!(current_request > old_request);
        receiver.accepted(later);
        let old_binding = Binding::new(1, 0);
        receiver
            .remote(
                Message::Granted {
                    request: old_request,
                    binding: old_binding.clone(),
                    geometry: geometry(&config()),
                    text_supported: true,
                    shortcut_policy: Some(ShortcutPolicy::Command),
                },
                &config(),
                later,
            )
            .unwrap();
        assert!(receiver.lease.is_none());
        assert!(receiver.commands.is_empty());
        assert!(
            matches!(receiver.message(later),Some(Message::Command {binding,command:Command::Stop,..}) if binding==&old_binding)
        );
        receiver.accepted(later);
        assert!(receiver.message(later).is_none());
        receiver
            .remote(
                Message::Granted {
                    request: current_request,
                    binding: Binding::new(2, 0),
                    geometry: geometry(&config()),
                    text_supported: true,
                    shortcut_policy: Some(ShortcutPolicy::Command),
                },
                &config(),
                later,
            )
            .unwrap();
        assert!(receiver.state().enabled);
        assert!(receiver.message(later).is_none());
        input(&mut receiver, button(true), later).unwrap();
        assert!(
            matches!(receiver.message(later),Some(Message::Command {command,..}) if command==&button(true))
        );
    }

    use extend_computer_agent::display::input::{Point, Size};

    fn config() -> Config {
        Config {
            mode: extend_computer_agent::session::display::Mode::Extend,
            width: 1728,
            height: 1085,
            fps: 60,
            bitrate: 20_000_000,
            retina: true,
            auto_size: true,
            codec: extend_computer_agent::session::display::VideoCodec::Hevc,
            separate_cursor: true,
        }
    }
    fn geometry(config: &Config) -> Geometry {
        let scale = if config.retina { 2 } else { 1 };
        Geometry {
            content: Rect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            canvas: Size {
                width: config.width,
                height: config.height,
            },
            raster: Size {
                width: config.width * scale,
                height: config.height * scale,
            },
            source_size: Size {
                width: config.width,
                height: config.height,
            },
        }
    }
    fn focus(receiver: &mut Receiver, now: Instant) {
        receiver
            .native(
                NativeEvent::Focus {
                    active: true,
                    revision: receiver.revision,
                },
                now,
            )
            .unwrap();
    }
    fn input(receiver: &mut Receiver, command: Command, now: Instant) -> Result<()> {
        receiver.native(
            NativeEvent::Input {
                command,
                revision: receiver.revision,
            },
            now,
        )
    }
    fn grant(receiver: &mut Receiver, now: Instant) -> Binding {
        let request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => panic!("Expected source authorization request"),
        };
        receiver.accepted(now);
        let binding = Binding::new(1, 0);
        receiver
            .remote(
                Message::Granted {
                    request,
                    binding: binding.clone(),
                    geometry: geometry(&config()),
                    text_supported: true,
                    shortcut_policy: Some(ShortcutPolicy::Command),
                },
                &config(),
                now,
            )
            .unwrap();
        binding
    }
    fn movement(x: f64) -> Command {
        Command::Move {
            point: Point { x, y: 0.5 },
        }
    }
    fn button(down: bool) -> Command {
        Command::Input {
            point: Some(Point { x: 0.5, y: 0.5 }),
            event: InputEvent::Button {
                button: 0,
                down,
                clicks: 1,
            },
        }
    }

    #[test]
    fn viewer_intent_requires_negotiation_and_a_current_source_grant() {
        let now = Instant::now();
        let mut absent = Receiver::new(false, false, false);
        focus(&mut absent, now);
        input(&mut absent, button(true), now).unwrap();
        assert!(absent.message(now).is_none());
        assert!(!absent.state().available && !absent.state().enabled);

        let mut receiver = Receiver::new(true, true, true);
        input(&mut receiver, button(true), now).unwrap();
        assert!(receiver.message(now).is_none());
        focus(&mut receiver, now);
        input(&mut receiver, button(true), now).unwrap();
        input(&mut receiver, button(false), now).unwrap();
        assert!(!receiver.state().enabled);
        grant(&mut receiver, now);
        assert!(receiver.state().enabled);
        assert!(
            matches!(receiver.message(now), Some(Message::Command { command, sequence: 1, .. }) if command == &button(true))
        );
        receiver.accepted(now);
        assert!(
            matches!(receiver.message(now), Some(Message::Command { command, sequence: 2, .. }) if command == &button(false))
        );
    }

    #[test]
    fn late_grant_after_focus_loss_is_stopped_without_replaying_intent() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        let request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        receiver.accepted(now);
        input(&mut receiver, button(true), now).unwrap();
        receiver.suspend(now);
        let binding = Binding::new(1, 0);
        receiver
            .remote(
                Message::Granted {
                    request,
                    binding: binding.clone(),
                    geometry: geometry(&config()),
                    text_supported: true,
                    shortcut_policy: None,
                },
                &config(),
                now,
            )
            .unwrap();
        assert!(!receiver.state().enabled && receiver.commands.is_empty());
        assert!(
            matches!(receiver.message(now), Some(Message::Command { binding: retired, command: Command::Stop, .. }) if retired == &binding)
        );
    }

    #[test]
    fn denial_before_any_state_snapshot_still_resets_native_pending_ownership() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        let initial = receiver.state();
        focus(&mut receiver, now);
        let request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        receiver.accepted(now);
        receiver
            .remote(
                Message::Denied {
                    request,
                    reason: extend_computer_agent::display::input::Reason::Permission,
                },
                &config(),
                now,
            )
            .unwrap();
        assert_ne!(receiver.state(), initial);
        assert!(receiver.state().revision > initial.revision);
        assert!(!receiver.state().focused);
    }

    #[test]
    fn bounded_ack_window_coalesces_only_pointer_and_times_out_to_stop() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        let binding = grant(&mut receiver, now);
        for _ in 0..4 {
            input(&mut receiver, button(true), now).unwrap();
            assert!(receiver.message(now).is_some());
            receiver.accepted(now);
        }
        for x in [0.1, 0.2, 0.3] {
            input(&mut receiver, movement(x), now).unwrap();
        }
        assert_eq!(receiver.commands.len(), 1);
        assert!(receiver.message(now).is_none());
        receiver
            .remote(
                Message::Applied {
                    binding: binding.clone(),
                    sequence: 2,
                },
                &config(),
                now,
            )
            .unwrap();
        assert!(
            matches!(receiver.message(now), Some(Message::Command { command, sequence: 5, .. }) if command == &movement(0.3))
        );
        receiver.accepted(now);
        let later = now + TIMEOUT;
        assert!(matches!(
            receiver.message(later),
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        assert!(!receiver.state().enabled);
    }

    #[test]
    fn repeated_focus_loss_keeps_release_and_resize_rejects_old_layout_events() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        grant(&mut receiver, now);
        input(&mut receiver, button(true), now).unwrap();
        receiver.message(now).unwrap();
        receiver.accepted(now);
        receiver.suspend(now);
        receiver.suspend(now);
        assert!(matches!(
            receiver.message(now),
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        receiver.accepted(now);
        let old_revision = receiver.revision;
        receiver.reconfigure(now).unwrap();
        receiver
            .native(
                NativeEvent::Focus {
                    active: true,
                    revision: old_revision,
                },
                now,
            )
            .unwrap();
        receiver
            .native(
                NativeEvent::Input {
                    command: button(true),
                    revision: old_revision,
                },
                now,
            )
            .unwrap();
        assert!(receiver.message(now).is_none());
        focus(&mut receiver, now);
        assert!(matches!(
            receiver.message(now),
            Some(Message::Request { .. })
        ));
    }

    #[test]
    fn grant_geometry_and_capture_bars_reject_stale_or_outside_commands() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        grant(&mut receiver, now);
        receiver.lease.as_mut().unwrap().geometry.content = Rect {
            x: 0.25,
            y: 0.,
            width: 0.5,
            height: 1.,
        };
        assert!(input(&mut receiver, movement(0.1), now).is_err());
        assert!(matches!(
            receiver.message(now),
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        receiver.accepted(now);
        focus(&mut receiver, now);
        let request = match receiver.message(now).unwrap() {
            Message::Request { request } => *request,
            _ => unreachable!(),
        };
        receiver.accepted(now);
        let mut wrong = geometry(&config());
        wrong.canvas.width += 2;
        assert!(receiver
            .remote(
                Message::Granted {
                    request,
                    binding: Binding::new(2, 0),
                    geometry: wrong,
                    text_supported: true,
                    shortcut_policy: None
                },
                &config(),
                now
            )
            .is_err());
        assert!(!receiver.state().enabled);
    }

    #[test]
    fn unicode_requires_negotiation_and_queue_overflow_releases_the_lease() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        grant(&mut receiver, now);
        let text = Command::Input {
            point: None,
            event: InputEvent::Text {
                value: "USB input: é ñ 文 😀".into(),
            },
        };
        input(&mut receiver, text.clone(), now).unwrap();
        assert!(
            matches!(receiver.message(now), Some(Message::Command { command, .. }) if command == &text)
        );
        receiver.accepted(now);
        for _ in 0..QUEUE {
            input(&mut receiver, button(true), now).unwrap();
        }
        assert!(input(&mut receiver, button(false), now).is_err());
        assert!(matches!(
            receiver.message(now),
            Some(Message::Command {
                command: Command::Stop,
                ..
            })
        ));
        assert!(!receiver.state().enabled);
        let mut legacy = Receiver::new(true, false, false);
        focus(&mut legacy, now);
        assert!(input(&mut legacy, text, now).is_err());
    }

    #[test]
    fn stale_binding_ack_is_ignored_but_unsent_future_ack_retires_input() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        let binding = grant(&mut receiver, now);
        receiver
            .remote(
                Message::Applied {
                    binding: Binding::new(9, 9),
                    sequence: 99,
                },
                &config(),
                now,
            )
            .unwrap();
        assert!(receiver.state().enabled);
        input(&mut receiver, button(true), now).unwrap();
        receiver.message(now).unwrap();
        assert!(receiver
            .remote(
                Message::Applied {
                    binding,
                    sequence: 1
                },
                &config(),
                now
            )
            .is_err());
        assert!(!receiver.state().enabled);
    }

    #[test]
    fn idle_lease_has_bounded_keepalive_and_channel_close_disables_input() {
        let now = Instant::now();
        let mut receiver = Receiver::new(true, true, true);
        focus(&mut receiver, now);
        grant(&mut receiver, now);
        assert!(receiver.message(now).is_none());
        assert!(matches!(
            receiver.message(now + ALIVE_INTERVAL),
            Some(Message::Command {
                command: Command::Alive,
                ..
            })
        ));
        receiver.accepted(now + ALIVE_INTERVAL);
        receiver.close(now + ALIVE_INTERVAL);
        assert!(!receiver.state().available && !receiver.state().enabled);
        assert!(receiver.message(now + ALIVE_INTERVAL).is_none());
    }

    #[test]
    fn native_protocol_requires_layout_revision_and_rejects_remote_binding_fields() {
        for value in [
            r#"{"type":"focus","active":true}"#,
            r#"{"type":"focus","active":true,"revision":1,"binding":{}}"#,
            r#"{"type":"granted","revision":1}"#,
            r#"{"type":"input","revision":1,"command":{"kind":"move","point":{"x":0.5,"y":0.5},"epoch":7}}"#,
        ] {
            assert!(serde_json::from_str::<NativeEvent>(value).is_err());
        }
        let event: NativeEvent =
            serde_json::from_str(r#"{"type":"focus","active":true,"revision":1}"#).unwrap();
        let mut receiver = Receiver::new(true, true, true);
        receiver.native(event, Instant::now()).unwrap();
        assert!(receiver.request.is_some());
    }
}
