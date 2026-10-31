//! The viewer's input intent, a port of the macOS viewer's model
//! (native/macos/display/ViewerInput.swift). The viewer owns only local
//! intent: the runtime's receiver owns the lease, ordering and target, and
//! nothing here posts OS input.
use crate::keys::{COMMAND, CONTROL, OPTION};
use anyhow::{ensure, Result};
use extend_computer_agent::{
    display::input::{Command, Point},
    helper::display::{InputState, NativeEvent},
    input::{InputEvent, ShortcutPolicy},
};
use std::{
    collections::{BTreeSet, VecDeque},
    time::{Duration, Instant},
};

/// Where the source's captured screen sits in the canvas, normalized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Content {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for Content {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}
impl Content {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1.0 + 1e-12
            && self.y + self.height <= 1.0 + 1e-12
    }

    pub fn contains(&self, point: Point) -> bool {
        self.valid()
            && point.x.is_finite()
            && point.y.is_finite()
            && point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

/// What the runtime's `input_state` lets the viewer offer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Capabilities {
    pub revision: u64,
    pub remote_focused: bool,
    pub available: bool,
    pub enabled: bool,
    pub text_supported: bool,
    pub shortcut_policy: Option<ShortcutPolicy>,
    pub content: Content,
}
impl Capabilities {
    pub fn new(state: &InputState) -> Result<Self> {
        let content = Content {
            x: state.content.x,
            y: state.content.y,
            width: state.content.width,
            height: state.content.height,
        };
        ensure!(
            state.revision > 0 && (!state.enabled || state.available) && content.valid(),
            "Invalid viewer input state."
        );
        Ok(Self {
            revision: state.revision,
            remote_focused: state.focused,
            available: state.available,
            enabled: state.enabled,
            text_supported: state.available && state.text_supported,
            shortcut_policy: state.shortcut_policy,
            content,
        })
    }
}

/// What the source holds because of this viewer, as the source's own input
/// state tracks it. An event that changes nothing there is not sent.
#[derive(Debug, Default)]
pub struct Held {
    pub keys: BTreeSet<u16>,
    pub buttons: BTreeSet<u8>,
    pub modifiers: u8,
}
impl Held {
    fn apply(&mut self, event: &InputEvent) -> bool {
        if event.validate().is_err() {
            return false;
        }
        match *event {
            InputEvent::MacKey { code, down, repeat } => match (down, repeat) {
                (true, true) => self.keys.contains(&code),
                (true, false) => self.keys.insert(code),
                (false, _) => self.keys.remove(&code),
            },
            InputEvent::Button { button, down, .. } => {
                if down {
                    self.buttons.insert(button)
                } else {
                    self.buttons.remove(&button)
                }
            }
            InputEvent::Scroll { .. } => true,
            InputEvent::Modifiers { mask } => std::mem::replace(&mut self.modifiers, mask) != mask,
            // The source refuses text while keys or modifiers are down.
            InputEvent::Text { .. } => self.keys.is_empty() && self.modifiers == 0,
            InputEvent::Release => {
                let held = !self.keys.is_empty() || !self.buttons.is_empty() || self.modifiers != 0;
                self.reset();
                held
            }
        }
    }

    fn reset(&mut self) {
        self.keys.clear();
        self.buttons.clear();
        self.modifiers = 0;
    }
}

/// Windows editing chords (Ctrl with A, Z, X, C, V or Y, as macOS key codes)
/// become Command chords for a source that edits with Command.
const EDITING_KEYS: [u16; 6] = [0, 6, 7, 8, 9, 16];

#[derive(Debug, Default)]
pub struct Model {
    capabilities: Capabilities,
    focused: bool,
    held: Held,
    last_point: Option<Point>,
    scroll: (f64, f64),
}
impl Model {
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn held(&self) -> &Held {
        &self.held
    }

    pub fn last_point(&self) -> Option<Point> {
        self.last_point
    }

    pub fn accepts_input(&self) -> bool {
        self.focused && self.capabilities.available
    }

    /// Applies a newer input state. Returns whether local held state and any
    /// composition must be discarded too.
    pub fn configure(&mut self, next: Capabilities) -> bool {
        if next.revision < self.capabilities.revision {
            return false;
        }
        let current = &self.capabilities;
        let invalidate = !next.available
            || (current.enabled && !next.enabled)
            || (current.enabled && next.content != current.content)
            || next.revision != current.revision
            || (current.remote_focused && !next.remote_focused);
        self.capabilities = next;
        if invalidate {
            self.reset();
            self.focused = false;
        }
        invalidate
    }

    /// The user acted on the viewer: ask for the lease unless it's held.
    pub fn intent(&mut self) -> Vec<NativeEvent> {
        if !self.capabilities.available {
            return Vec::new();
        }
        let request = !self.focused || !self.capabilities.enabled;
        self.focused = true;
        if request {
            vec![NativeEvent::Focus {
                active: true,
                revision: self.capabilities.revision,
            }]
        } else {
            Vec::new()
        }
    }

    /// Focus left the viewer: give up the lease, which releases everything
    /// the source holds.
    pub fn blur(&mut self) -> Vec<NativeEvent> {
        let active = std::mem::take(&mut self.focused);
        self.reset();
        if active {
            vec![NativeEvent::Focus {
                active: false,
                revision: self.capabilities.revision,
            }]
        } else {
            Vec::new()
        }
    }

    fn reset(&mut self) {
        self.held.reset();
        self.last_point = None;
        self.scroll = (0.0, 0.0);
    }

    fn input(&self, command: Command) -> NativeEvent {
        NativeEvent::Input {
            command,
            revision: self.capabilities.revision,
        }
    }

    fn event(&mut self, event: InputEvent, point: Option<Point>) -> Vec<NativeEvent> {
        if !self.accepts_input() || !self.held.apply(&event) {
            return Vec::new();
        }
        vec![self.input(Command::Input { point, event })]
    }

    pub fn modifiers(&mut self, mask: u8) -> Vec<NativeEvent> {
        self.event(InputEvent::Modifiers { mask }, None)
    }

    pub fn move_to(&mut self, point: Point) -> Vec<NativeEvent> {
        // Windows also reports a move when nothing moved, e.g. after a click.
        if !self.accepts_input()
            || !self.capabilities.content.contains(point)
            || self.last_point == Some(point)
        {
            return Vec::new();
        }
        self.last_point = Some(point);
        vec![self.input(Command::Move { point })]
    }

    pub fn button(
        &mut self,
        button: u8,
        down: bool,
        clicks: u8,
        point: Option<Point>,
        modifiers: u8,
    ) -> Vec<NativeEvent> {
        if !self.accepts_input() {
            return Vec::new();
        }
        let point = point.filter(|point| self.capabilities.content.contains(*point));
        // A drag may end beyond the picture. Release at the last valid
        // position; never press outside the displayed content.
        let Some(target) = point.or(if down { None } else { self.last_point }) else {
            return if down { Vec::new() } else { self.blur() };
        };
        self.last_point = Some(target);
        let mut events = self.modifiers(modifiers);
        events.extend(self.event(
            InputEvent::Button {
                button,
                down,
                clicks: clicks.clamp(1, 3),
            },
            Some(target),
        ));
        events
    }

    /// Scroll deltas in macOS pixels; fractions carry over to the next one.
    pub fn scroll(&mut self, dx: f64, dy: f64, point: Point, modifiers: u8) -> Vec<NativeEvent> {
        if !self.accepts_input()
            || !self.capabilities.content.contains(point)
            || !dx.is_finite()
            || !dy.is_finite()
        {
            return Vec::new();
        }
        self.last_point = Some(point);
        self.scroll.0 += dx.clamp(-4096.0, 4096.0);
        self.scroll.1 += dy.clamp(-4096.0, 4096.0);
        let x = self.scroll.0.clamp(-4096.0, 4096.0) as i32;
        let y = self.scroll.1.clamp(-4096.0, 4096.0) as i32;
        self.scroll.0 -= f64::from(x);
        self.scroll.1 -= f64::from(y);
        if x == 0 && y == 0 {
            return Vec::new();
        }
        let mut events = self.modifiers(modifiers);
        events.extend(self.event(InputEvent::Scroll { dx: x, dy: y }, Some(point)));
        events
    }

    /// A physical key. Key-ups go out only for keys whose press went out.
    pub fn key(&mut self, code: u16, down: bool, repeat: bool, modifiers: u8) -> Vec<NativeEvent> {
        let mut mask = modifiers;
        // Clipboard and undo chords run on the source with its own modifier.
        // AltGr (Control with Alt) stays as typed.
        if mask & (CONTROL | OPTION) == CONTROL
            && EDITING_KEYS.contains(&code)
            && self.capabilities.shortcut_policy == Some(ShortcutPolicy::Command)
        {
            mask = mask & !CONTROL | COMMAND;
        }
        if !self.accepts_input() || (!down && !self.held.keys.contains(&code)) {
            return Vec::new();
        }
        let mut events = self.modifiers(mask);
        events.extend(self.event(InputEvent::MacKey { code, down, repeat }, None));
        events
    }

    /// Committed text, sent with no modifiers held and the modifiers restored
    /// after it.
    pub fn text(&mut self, value: &str) -> Vec<NativeEvent> {
        if !self.accepts_input() || !self.capabilities.text_supported || !self.held.keys.is_empty()
        {
            return Vec::new();
        }
        let Some(chunks) = text_chunks(value) else {
            return Vec::new();
        };
        let mask = self.held.modifiers;
        let mut events = self.modifiers(0);
        for chunk in chunks {
            events.extend(self.event(InputEvent::Text { value: chunk }, None));
        }
        events.extend(self.modifiers(mask));
        events
    }
}

/// Splits a commit into protocol-sized pieces: at most 64 characters and 256
/// bytes each. A commit must fit the bounded output queue, so larger ones and
/// any with control characters are refused whole.
pub fn text_chunks(value: &str) -> Option<Vec<String>> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().count() > 2048
        || value.chars().any(char::is_control)
    {
        return None;
    }
    let mut chunks = Vec::new();
    let (mut current, mut count) = (String::new(), 0);
    for character in value.chars() {
        if count == 64 || current.len() + character.len_utf8() > 256 {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
        current.push(character);
        count += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Some(chunks)
}

/// The wire modifier mask: shift, control, option (Alt) and command
/// (Windows key), without sides.
pub fn modifier_mask(shift: bool, control: bool, alt: bool, windows: bool) -> u8 {
    u8::from(shift) | u8::from(control) << 1 | u8::from(alt) << 2 | u8::from(windows) << 3
}

/// What a key press types, from the character messages it queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Produces {
    Nothing,
    /// A printable character.
    Text,
    /// A dead key, which composes with the next one.
    Dead,
    /// Enter, Tab, Backspace, Escape and other control characters.
    Control,
}
impl Produces {
    /// Classifies a `WM_CHAR` or `WM_DEADCHAR` unit.
    pub fn of(unit: u16, dead: bool) -> Self {
        if dead {
            Self::Dead
        } else if char::from_u32(u32::from(unit)).is_some_and(char::is_control) {
            Self::Control
        } else {
            Self::Text
        }
    }
}

/// Whether a key press goes out as the text it types instead of as a
/// physical key, never both. Like the macOS viewer, text needs the source's
/// support and no Control or Command chord; AltGr (Control with Alt) types
/// text.
pub fn sends_text(text_supported: bool, mask: u8, produces: Produces) -> bool {
    let chord = mask & COMMAND != 0 || (mask & CONTROL != 0 && mask & OPTION == 0);
    text_supported && !chord && matches!(produces, Produces::Text | Produces::Dead)
}

/// Joins `WM_CHAR` units into text, skipping the characters of keys that
/// went out as physical keys.
#[derive(Debug, Default)]
pub struct Typing {
    physical: bool,
    surrogate: Option<u16>,
}
impl Typing {
    /// A key press was routed; its characters follow.
    pub fn pressed(&mut self, physical: bool) {
        self.physical = physical;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The text a character unit completes, if it goes out as text.
    pub fn character(&mut self, unit: u16) -> Option<char> {
        if self.physical {
            self.surrogate = None;
            return None;
        }
        let character = match unit {
            0xD800..=0xDBFF => {
                self.surrogate = Some(unit);
                return None;
            }
            0xDC00..=0xDFFF => char::decode_utf16([self.surrogate.take()?, unit])
                .next()?
                .ok()?,
            _ => {
                self.surrogate = None;
                char::from_u32(u32::from(unit))?
            }
        };
        (!character.is_control()).then_some(character)
    }
}

/// Click counts, as the Windows input helper counts them: a press of the
/// same button within the double-click time and distance continues the
/// series, up to a triple click.
#[derive(Debug, Default)]
pub struct Clicks {
    last: Option<(u8, Instant, (i32, i32), u8)>,
}
impl Clicks {
    pub fn press(
        &mut self,
        button: u8,
        at: Instant,
        position: (i32, i32),
        limit: Duration,
        slop: (i32, i32),
    ) -> u8 {
        let count = match self.last {
            Some((last, time, from, count))
                if last == button
                    && at.saturating_duration_since(time) <= limit
                    && (from.0 - position.0).abs() <= slop.0 / 2
                    && (from.1 - position.1).abs() <= slop.1 / 2 =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.last = Some((button, at, position, count));
        count
    }

    /// The count of the series a release ends.
    pub fn count(&self, button: u8) -> u8 {
        match self.last {
            Some((last, _, _, count)) if last == button => count,
            _ => 1,
        }
    }
}

/// Pending kind 4 payloads. Stdout may be backpressured by a stopped
/// parent, so the queue is bounded: consecutive moves coalesce, and an
/// overflow replaces everything queued with one focus loss.
#[derive(Debug, Default)]
pub struct Outbox {
    pending: VecDeque<(Vec<u8>, bool, u64)>,
}
pub const OUTBOX_LIMIT: usize = 64;
impl Outbox {
    /// Queues `events`; returns whether the queue overflowed, after which
    /// local input state must be dropped too.
    pub fn enqueue(&mut self, events: Vec<NativeEvent>) -> bool {
        for event in events {
            let revision = match &event {
                NativeEvent::Focus { revision, .. } | NativeEvent::Input { revision, .. } => {
                    *revision
                }
            };
            // The runtime ends the session on input it can't read; never
            // send any.
            let Some(bytes) = encode(&event) else {
                continue;
            };
            let movement = matches!(
                event,
                NativeEvent::Input {
                    command: Command::Move { .. },
                    ..
                }
            );
            if matches!(event, NativeEvent::Focus { active: false, .. }) {
                self.pending.clear();
            }
            if movement
                && self
                    .pending
                    .back()
                    .is_some_and(|(_, moved, last)| *moved && *last == revision)
            {
                self.pending.pop_back();
            }
            if self.pending.len() >= OUTBOX_LIMIT {
                self.pending.clear();
                let loss = NativeEvent::Focus {
                    active: false,
                    revision,
                };
                if let Some(bytes) = encode(&loss) {
                    self.pending.push_back((bytes, false, revision));
                }
                return true;
            }
            self.pending.push_back((bytes, movement, revision));
        }
        false
    }

    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.pending.pop_front().map(|(bytes, ..)| bytes)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// The kind 4 payload, if the runtime will accept it.
pub fn encode(event: &NativeEvent) -> Option<Vec<u8>> {
    let bytes = serde_json::to_vec(event).ok()?;
    (bytes.len() <= extend_computer_agent::helper::packet::MAX_VIEWER_INPUT
        && NativeEvent::parse(&bytes).is_ok())
    .then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::display::input::Rect;

    fn state(
        revision: u64,
        enabled: bool,
        focused: bool,
        policy: Option<ShortcutPolicy>,
    ) -> Capabilities {
        Capabilities::new(&InputState {
            available: true,
            enabled,
            focused,
            text_supported: true,
            shortcut_policy: policy,
            content: Rect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            revision,
        })
        .unwrap()
    }

    fn event(message: &NativeEvent) -> Option<&InputEvent> {
        match message {
            NativeEvent::Input {
                command: Command::Input { event, .. },
                ..
            } => Some(event),
            _ => None,
        }
    }

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn input_state_parses_from_framed_json_and_rejects_inconsistent_grants() {
        let bytes = br#"{"type":"input_state","available":true,"enabled":false,"focused":false,"text_supported":true,"shortcut_policy":"command","content":{"x":0,"y":0,"width":1,"height":1},"revision":1}"#;
        let extend_computer_agent::helper::display::ViewerCommand::InputState(parsed) =
            serde_json::from_slice(bytes).unwrap()
        else {
            panic!("not input state");
        };
        assert_eq!(
            Capabilities::new(&parsed).unwrap(),
            state(1, false, false, Some(ShortcutPolicy::Command))
        );
        let mut invalid = parsed.clone();
        invalid.revision = 0;
        assert!(Capabilities::new(&invalid).is_err());
        invalid = parsed.clone();
        invalid.available = false;
        invalid.enabled = true;
        assert!(Capabilities::new(&invalid).is_err());
        invalid = parsed;
        invalid.content.x = 0.5;
        assert!(Capabilities::new(&invalid).is_err());
    }

    #[test]
    fn lease_intent_buttons_and_scrolling() {
        let mut model = Model::default();
        assert!(model.intent().is_empty() && model.move_to(point(0.5, 0.5)).is_empty());
        model.configure(state(1, false, false, Some(ShortcutPolicy::Command)));
        assert!(model.move_to(point(0.5, 0.5)).is_empty());
        let focus = model.intent();
        assert_eq!(
            focus,
            vec![NativeEvent::Focus {
                active: true,
                revision: 1
            }]
        );
        let down = model.button(0, true, 2, Some(point(0.4, 0.5)), 1);
        assert_eq!(down.len(), 2);
        assert_eq!(event(&down[0]), Some(&InputEvent::Modifiers { mask: 1 }));
        assert_eq!(
            event(&down[1]),
            Some(&InputEvent::Button {
                button: 0,
                down: true,
                clicks: 2
            })
        );
        model.configure(state(1, true, true, Some(ShortcutPolicy::Command)));
        let up = model.button(0, false, 2, None, 0);
        let NativeEvent::Input {
            command: Command::Input {
                point: released, ..
            },
            ..
        } = up.last().unwrap()
        else {
            panic!("no release");
        };
        assert_eq!(*released, Some(point(0.4, 0.5)));
        assert!(model.held().buttons.is_empty());
        assert!(model.button(0, true, 1, Some(point(-1., 0.)), 0).is_empty());
        assert!(model.scroll(0.4, 0.4, point(0.5, 0.5), 0).is_empty());
        let scroll = model.scroll(0.7, 0.7, point(0.5, 0.5), 0);
        assert_eq!(
            event(scroll.last().unwrap()),
            Some(&InputEvent::Scroll { dx: 1, dy: 1 })
        );
        assert!(model.scroll(f64::INFINITY, 1., point(0., 0.), 0).is_empty());
        let bounded = model.scroll(1e300, -1e300, point(0., 0.), 0);
        assert_eq!(
            event(bounded.last().unwrap()),
            Some(&InputEvent::Scroll {
                dx: 4096,
                dy: -4095
            })
        );
    }

    #[test]
    fn keys_text_shortcuts_and_focus_loss() {
        let mut model = Model::default();
        model.configure(state(1, true, true, Some(ShortcutPolicy::Command)));
        model.intent();
        assert!(!model.key(123, true, false, 0).is_empty());
        assert!(model.text("blocked").is_empty());
        assert!(!model.key(123, true, true, 0).is_empty());
        assert!(!model.key(123, false, false, 0).is_empty());
        assert!(model.key(123, false, false, 0).is_empty());
        // A key-up for a key whose press went out as text is never sent.
        assert!(model.key(0, false, false, 0).is_empty());
        model.modifiers(1);
        let unicode = "é e\u{301} 中 👩‍💻 ".repeat(30);
        let text = model.text(&unicode);
        let commits: Vec<String> = text
            .iter()
            .filter_map(|message| match event(message) {
                Some(InputEvent::Text { value }) => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(commits.concat(), unicode);
        for commit in &commits {
            assert!(commit.len() <= 256 && commit.chars().count() <= 64);
        }
        for message in &text {
            assert!(encode(message).is_some_and(|bytes| bytes.len() <= 1024));
        }
        assert_eq!(event(&text[0]), Some(&InputEvent::Modifiers { mask: 0 }));
        assert_eq!(
            event(text.last().unwrap()),
            Some(&InputEvent::Modifiers { mask: 1 })
        );
        // AltGr text neutralizes Control and Option, then restores them.
        model.modifiers(CONTROL | OPTION);
        let altgr = model.text("€");
        assert_eq!(event(&altgr[0]), Some(&InputEvent::Modifiers { mask: 0 }));
        assert_eq!(
            event(&altgr[1]),
            Some(&InputEvent::Text {
                value: "€".into()
            })
        );
        assert_eq!(event(&altgr[2]), Some(&InputEvent::Modifiers { mask: 6 }));
        assert!(model.text("bad\ntext").is_empty());
        assert!(model.text(&"x".repeat(2049)).is_empty());
        assert!(model.text(&"👩".repeat(1025)).is_empty());

        // Ctrl+C reaches a Command source as Command+C, but AltGr+C doesn't.
        let copy = model.key(8, true, false, CONTROL);
        assert_eq!(event(&copy[0]), Some(&InputEvent::Modifiers { mask: 8 }));
        assert_eq!(model.held().modifiers, 8);
        model.key(8, false, false, CONTROL);
        let altgr = model.key(8, true, false, CONTROL | OPTION);
        assert_eq!(event(&altgr[0]), Some(&InputEvent::Modifiers { mask: 6 }));
        model.key(8, false, false, CONTROL | OPTION);
        model.configure(state(1, true, true, Some(ShortcutPolicy::Control)));
        let control = model.key(8, true, false, CONTROL);
        assert_eq!(event(&control[0]), Some(&InputEvent::Modifiers { mask: 2 }));

        let blur = model.blur();
        assert_eq!(
            blur,
            vec![NativeEvent::Focus {
                active: false,
                revision: 1
            }]
        );
        assert!(model.held().keys.is_empty() && model.held().modifiers == 0);
        assert!(model.blur().is_empty() && model.text("late").is_empty());
        model.intent();
        model.button(0, true, 1, Some(point(0., 0.)), 0);
        let mut lost = state(2, false, false, Some(ShortcutPolicy::Control));
        assert!(model.configure(lost.clone()));
        assert!(!model.accepts_input() && model.held().buttons.is_empty());
        lost.revision = 1;
        assert!(!model.configure(lost));
        assert_eq!(model.capabilities().revision, 2);
    }

    #[test]
    fn text_and_physical_keys_never_both_carry_a_key_press() {
        // Printable keys, dead keys and AltGr type text when the source
        // takes it; chords and control characters are physical keys.
        let cases = [
            (true, 0, Produces::Text, true),
            (true, 1, Produces::Text, true),
            (true, 0, Produces::Dead, true),
            (true, CONTROL | OPTION, Produces::Text, true),
            (true, OPTION, Produces::Text, true),
            (true, CONTROL, Produces::Text, false),
            (true, COMMAND, Produces::Text, false),
            (true, 0, Produces::Control, false),
            (true, 0, Produces::Nothing, false),
            (false, 0, Produces::Text, false),
            (false, 0, Produces::Dead, false),
        ];
        for (supported, mask, produces, text) in cases {
            assert_eq!(
                sends_text(supported, mask, produces),
                text,
                "{supported} {mask} {produces:?}"
            );
        }
        assert_eq!(Produces::of(u16::from(b'a'), false), Produces::Text);
        assert_eq!(Produces::of(u16::from(b'\r'), false), Produces::Control);
        assert_eq!(Produces::of(0x7f, false), Produces::Control);
        assert_eq!(Produces::of(0xb4, true), Produces::Dead);
        assert_eq!(modifier_mask(true, false, true, false), 5);
        assert_eq!(modifier_mask(false, true, false, true), 10);
    }

    #[test]
    fn typing_joins_surrogates_and_skips_physical_keys_and_dead_key_sequences() {
        let mut typing = Typing::default();
        typing.pressed(false);
        assert_eq!(typing.character(u16::from(b'a')), Some('a'));
        // A dead key queues WM_DEADCHAR, then the next key's WM_CHAR carries
        // the composed character.
        typing.pressed(false);
        typing.pressed(false);
        assert_eq!(typing.character(0xe9), Some('é'));
        // An emoji from the touch keyboard arrives as two units.
        assert_eq!(typing.character(0xd83d), None);
        assert_eq!(typing.character(0xde00), Some('😀'));
        assert_eq!(typing.character(0xde00), None);
        assert_eq!(typing.character(u16::from(b'\r')), None);
        // Enter went out as a physical key; its WM_CHAR is not text too.
        typing.pressed(true);
        assert_eq!(typing.character(u16::from(b'x')), None);
        typing.reset();
        assert_eq!(typing.character(u16::from(b'x')), Some('x'));
    }

    #[test]
    fn clicks_count_like_the_input_helper() {
        let mut clicks = Clicks::default();
        let start = Instant::now();
        let limit = Duration::from_millis(500);
        let at = |ms| start + Duration::from_millis(ms);
        assert_eq!(clicks.press(0, at(0), (100, 100), limit, (4, 4)), 1);
        assert_eq!(clicks.press(0, at(200), (101, 101), limit, (4, 4)), 2);
        assert_eq!(clicks.count(0), 2);
        assert_eq!(clicks.press(0, at(400), (100, 100), limit, (4, 4)), 3);
        assert_eq!(clicks.press(0, at(600), (100, 100), limit, (4, 4)), 1);
        assert_eq!(clicks.press(0, at(1200), (100, 100), limit, (4, 4)), 1);
        assert_eq!(clicks.press(0, at(1300), (110, 100), limit, (4, 4)), 1);
        assert_eq!(clicks.press(1, at(1400), (110, 100), limit, (4, 4)), 1);
        assert_eq!(clicks.count(0), 1);
    }

    #[test]
    fn outbox_coalesces_moves_and_overflow_leaves_one_focus_loss() {
        let mut outbox = Outbox::default();
        let movement = NativeEvent::Input {
            command: Command::Move {
                point: point(0.4, 0.5),
            },
            revision: 1,
        };
        let focus = NativeEvent::Focus {
            active: true,
            revision: 1,
        };
        assert!(!outbox.enqueue(vec![focus.clone()]));
        for _ in 0..100 {
            assert!(!outbox.enqueue(vec![movement.clone()]));
        }
        assert_eq!(outbox.pending.len(), 2);
        let mut overflowed = false;
        for _ in 0..65 {
            if outbox.enqueue(vec![focus.clone()]) {
                overflowed = true;
                break;
            }
        }
        assert!(overflowed);
        assert_eq!(
            outbox.pop().unwrap(),
            br#"{"type":"focus","active":false,"revision":1}"#
        );
        assert!(outbox.pop().is_none());
        // An invalid event is dropped instead of ending the session.
        assert!(!outbox.enqueue(vec![NativeEvent::Input {
            command: Command::Move {
                point: point(1.5, 0.5)
            },
            revision: 1
        }]));
        assert!(outbox.is_empty());
    }

    #[test]
    fn events_have_the_bytes_the_runtime_reads() {
        let bytes = encode(&NativeEvent::Input {
            command: Command::Input {
                point: Some(point(0.25, 0.5)),
                event: InputEvent::Button {
                    button: 0,
                    down: true,
                    clicks: 1,
                },
            },
            revision: 7,
        })
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"type":"input","command":{"kind":"input","point":{"x":0.25,"y":0.5},"event":{"kind":"button","button":0,"down":true,"clicks":1}},"revision":7}"#
        );
    }
}
