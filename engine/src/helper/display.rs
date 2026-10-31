//! Display helper (`ExtendComputerDisplay`) arguments and messages.
use super::input::{DisplayAction, DisplayEnvelope, DisplayEvent};
use crate::{
    display::input::{Binding, Command, Geometry, Rect, Size},
    input::ShortcutPolicy,
    session::display::{Config, VideoCodec, CONFIG_LIMITS},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Mirror,
    Extend,
    Viewer,
}
impl LaunchMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mirror => "mirror",
            Self::Extend => "extend",
            Self::Viewer => "viewer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    /// Pointer packets travel beside the video (kind 3).
    Separate,
    /// The pointer is drawn into the video.
    Embedded,
}

/// `mirror|extend|viewer W H FPS BITRATE retina|standard DISPLAY CODEC cursor|embedded`
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub mode: LaunchMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub retina: bool,
    /// 0 is the main display.
    pub display: u32,
    pub codec: VideoCodec,
    pub cursor: CursorMode,
}
impl Launch {
    pub fn new(mode: LaunchMode, config: &Config) -> Self {
        Self {
            mode,
            width: config.width,
            height: config.height,
            fps: config.fps,
            bitrate: config.bitrate,
            retina: config.retina,
            display: 0,
            codec: config.codec,
            cursor: if config.separate_cursor {
                CursorMode::Separate
            } else {
                CursorMode::Embedded
            },
        }
    }

    pub fn to_args(&self) -> Vec<String> {
        vec![
            self.mode.name().into(),
            self.width.to_string(),
            self.height.to_string(),
            self.fps.to_string(),
            self.bitrate.to_string(),
            if self.retina { "retina" } else { "standard" }.into(),
            self.display.to_string(),
            self.codec.name().into(),
            match self.cursor {
                CursorMode::Separate => "cursor",
                CursorMode::Embedded => "embedded",
            }
            .into(),
        ]
    }

    /// Reads helper arguments as the macOS helper does: the numbers must be in
    /// range, later arguments may be missing.
    pub fn parse(args: &[String]) -> Result<Self> {
        let mode = match args.first().map(String::as_str) {
            Some("mirror") => LaunchMode::Mirror,
            Some("extend") => LaunchMode::Extend,
            Some("viewer") => LaunchMode::Viewer,
            _ => bail!("Choose extend, mirror or viewer."),
        };
        let number = |index: usize, range: &std::ops::RangeInclusive<u32>| {
            args.get(index)
                .and_then(|value| value.parse().ok())
                .filter(|value| range.contains(value))
                .context("Invalid display configuration.")
        };
        let limits = &CONFIG_LIMITS;
        let (width, height) = (number(1, &limits.width)?, number(2, &limits.height)?);
        let (fps, bitrate) = (number(3, &limits.fps)?, number(4, &limits.bitrate)?);
        let codec = match args.get(7).map(String::as_str) {
            None | Some("h264") => VideoCodec::H264,
            Some("hevc") => VideoCodec::Hevc,
            Some(_) => bail!("Invalid video codec."),
        };
        Ok(Self {
            mode,
            width,
            height,
            fps,
            bitrate,
            retina: args.get(5).is_some_and(|value| value == "retina"),
            display: args
                .get(6)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            codec,
            cursor: if args.get(8).is_some_and(|value| value == "cursor") {
                CursorMode::Separate
            } else {
                CursorMode::Embedded
            },
        })
    }
}

/// `screen` output, also the body of a viewer's `viewport` replies. Lenient:
/// helpers add fields freely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Without the notch and other unsafe areas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usable_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usable_height: Option<u32>,
    /// A viewer's video view in CoreGraphics display coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_cg: Option<ViewportBounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_number: Option<u32>,
}
impl Screen {
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        super::sorted(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Kind 0, sent once a helper runs. A source adds its size; the macOS source
/// also adds its capture target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Startup {
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retina: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<CaptureTarget>,
}
impl Startup {
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        super::sorted(self)
    }
}

/// Kind 2 from a helper. Types this app does not know are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperMessage {
    /// Answers a configuration with the same id.
    Configured { id: u64, ok: bool },
    /// Asks for a keyframe, as a viewer does after a decode error.
    Keyframe,
    /// A source's capture target after a configuration.
    Target { target: CaptureTarget },
    /// A viewer rendered its first frame on the GPU.
    RenderReady(RenderReady),
    /// A viewer's presented frame count, sent for the first and every 60th.
    Presented { frames: u64 },
    /// Answers a viewer's viewport request with the same id.
    Viewport {
        id: u64,
        fullscreen: bool,
        #[serde(flatten)]
        screen: Screen,
    },
    /// A viewer's fullscreen geometry, sent with `EXTEND_DISPLAY_METRICS`.
    ViewportEvidence {
        #[serde(default)]
        fullscreen: bool,
        #[serde(flatten)]
        screen: Screen,
    },
    /// The viewer window closed.
    Closed,
    #[serde(other)]
    Unknown,
}
impl HelperMessage {
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        super::sorted(self)
    }
}

/// Exact, unlike the other messages: readiness gates a viewer's startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderReady {
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Configuration {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub retina: bool,
    pub fps: u32,
    pub bitrate: u32,
}
impl Configuration {
    pub fn new(id: u64, config: &Config) -> Self {
        Self {
            id,
            width: config.width,
            height: config.height,
            retina: config.retina,
            fps: config.fps,
            bitrate: config.bitrate,
        }
    }
}

/// A source's stdin, one JSON object per line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceCommand {
    /// Answered by [`HelperMessage::Configured`], after a new target.
    Configuration(Configuration),
    Pause {
        value: bool,
    },
    Keyframe,
    Bitrate {
        value: u32,
    },
    Stop,
}
impl SourceCommand {
    pub fn to_line(&self) -> serde_json::Result<Vec<u8>> {
        let mut line = super::sorted(self)?;
        line.push(b'\n');
        Ok(line)
    }

    /// Helpers skip lines they cannot read.
    pub fn parse_line(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
}

/// Kind 2 to a viewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerCommand {
    Configuration(Configuration),
    /// Answered by [`HelperMessage::Viewport`].
    Viewport {
        id: u64,
    },
    InputState(InputState),
    Stop,
}
impl ViewerCommand {
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        match self {
            // Input state was always written in field order.
            Self::InputState(_) => serde_json::to_vec(self),
            _ => super::sorted(self),
        }
    }
}

/// What a viewer may offer its user. Only the authenticated source grants input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputState {
    pub available: bool,
    pub enabled: bool,
    pub focused: bool,
    pub text_supported: bool,
    pub shortcut_policy: Option<ShortcutPolicy>,
    pub content: Rect,
    pub revision: u64,
}

/// Kind 4: a viewer's local input intent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeEvent {
    Focus { active: bool, revision: u64 },
    Input { command: Command, revision: u64 },
}
impl NativeEvent {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let event: Self = serde_json::from_slice(bytes)?;
        if let Self::Input { command, .. } = &event {
            command.validate()?;
        }
        Ok(event)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What a source captures, in the source's display coordinates (`bounds`) and
/// the encoded canvas (`content` inside `canvas`, before Retina doubles it to
/// `raster`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTarget {
    pub configuration_id: u64,
    pub display_id: u32,
    pub bounds: Bounds,
    pub canvas: Size,
    pub content: Bounds,
    pub raster: Size,
}
impl CaptureTarget {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.display_id != 0, "Capture target has no display");
        for r in [&self.bounds, &self.content] {
            ensure!(
                [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite())
                    && r.width >= 1.0
                    && r.height >= 1.0,
                "Invalid capture target rectangle"
            );
        }
        self.geometry()?.validate()
    }

    pub fn geometry(&self) -> Result<Geometry> {
        ensure!(
            self.bounds.width <= 32768.0 && self.bounds.height <= 32768.0,
            "Capture source too large"
        );
        Ok(Geometry {
            content: Rect {
                x: self.content.x / self.canvas.width as f64,
                y: self.content.y / self.canvas.height as f64,
                width: self.content.width / self.canvas.width as f64,
                height: self.content.height / self.canvas.height as f64,
            },
            canvas: self.canvas,
            raster: self.raster,
            source_size: Size {
                width: self.bounds.width as u32,
                height: self.bounds.height as u32,
            },
        })
    }

    /// The `inject-display` line for a granted command.
    pub fn event(
        &self,
        binding: &Binding,
        sequence: u64,
        command: &Command,
    ) -> Result<DisplayEnvelope> {
        command.validate()?;
        let (action, point) = match command {
            Command::Move { point } => (DisplayAction::Move, Some(*point)),
            Command::Alive => (DisplayAction::Alive, None),
            Command::Stop => (DisplayAction::Release, None),
            Command::Input { event, point } => (DisplayAction::Input(event.clone()), *point),
        };
        if let Some(point) = point {
            point.validate()?;
            let x = point.x * self.canvas.width as f64;
            let y = point.y * self.canvas.height as f64;
            ensure!(
                x >= self.content.x
                    && y >= self.content.y
                    && x <= self.content.x + self.content.width
                    && y <= self.content.y + self.content.height,
                "Display input point is outside captured content"
            );
        }
        Ok(DisplayEnvelope {
            session: binding.session.clone(),
            epoch: binding.epoch,
            revision: binding.revision,
            sequence,
            event: DisplayEvent { action, point },
        })
    }
}

pub const MAX_CURSOR_BYTES: usize = 128 * 1024;

/// Kind 3, and the cursor channels between peers, which carry the same bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorPacket {
    Shape(CursorShape),
    Position(CursorPosition),
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CursorShape {
    pub revision: u64,
    /// Base64 PNG.
    pub png: String,
    pub width: f64,
    pub height: f64,
    pub hotspot_x: f64,
    pub hotspot_y: f64,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CursorPosition {
    pub revision: u64,
    pub sequence: u64,
    /// Normalized to the encoded canvas.
    pub x: f64,
    pub y: f64,
    pub visible: bool,
}
impl CursorPacket {
    /// Shapes carry `png`; anything else is a position.
    pub fn is_shape(bytes: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .is_some_and(|value| value.get("png").is_some())
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_CURSOR_BYTES, "Cursor message too large");
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        if value.get("png").is_some() {
            let shape: CursorShape = serde_json::from_value(value)?;
            ensure!(shape.png.len() <= 120 * 1024, "Invalid cursor image");
            ensure!(
                [shape.width, shape.height]
                    .iter()
                    .all(|n| *n > 0.0 && *n <= 512.0),
                "Invalid cursor size"
            );
            ensure!(
                [shape.hotspot_x, shape.hotspot_y]
                    .iter()
                    .all(|n| (0.0..=512.0).contains(n)),
                "Invalid cursor hotspot"
            );
            Ok(Self::Shape(shape))
        } else {
            let position: CursorPosition = serde_json::from_value(value)?;
            ensure!(
                [position.x, position.y]
                    .iter()
                    .all(|n| (0.0..=1.0).contains(n)),
                "Invalid cursor coordinates"
            );
            Ok(Self::Position(position))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{display::input::Point, input::InputEvent};

    fn config(retina: bool, separate_cursor: bool) -> Config {
        Config {
            mode: crate::session::display::Mode::Mirror,
            width: 1728,
            height: 1117,
            fps: 60,
            bitrate: 20_000_000,
            retina,
            auto_size: false,
            codec: VideoCodec::Hevc,
            separate_cursor,
        }
    }

    #[test]
    fn launch_arguments_match_the_helper_command_line() {
        for (mode, retina, separate, expected) in [
            (
                LaunchMode::Mirror,
                true,
                true,
                "mirror 1728 1117 60 20000000 retina 0 hevc cursor",
            ),
            (
                LaunchMode::Extend,
                false,
                false,
                "extend 1728 1117 60 20000000 standard 0 hevc embedded",
            ),
            (
                LaunchMode::Viewer,
                true,
                false,
                "viewer 1728 1117 60 20000000 retina 0 hevc embedded",
            ),
        ] {
            let launch = Launch::new(mode, &config(retina, separate));
            let args = launch.to_args();
            assert_eq!(args.join(" "), expected);
            assert_eq!(Launch::parse(&args).unwrap(), launch);
        }
    }

    #[test]
    fn launch_parse_defaults_later_arguments_and_bounds_numbers() {
        let args = |line: &str| line.split(' ').map(String::from).collect::<Vec<_>>();
        let short = Launch::parse(&args("mirror 1920 1080 60 12000000")).unwrap();
        assert_eq!(
            (short.retina, short.display, short.codec, short.cursor),
            (false, 0, VideoCodec::H264, CursorMode::Embedded)
        );
        for invalid in [
            "share 1920 1080 60 12000000",
            "mirror 639 1080 60 12000000",
            "mirror 1920 4097 60 12000000",
            "mirror 1920 1080 121 12000000",
            "mirror 1920 1080 60 999999",
            "mirror 1920 1080 60 1.5e7",
            "mirror 1920 1080",
            "mirror 1920 1080 60 12000000 standard 0 vp9",
        ] {
            assert!(Launch::parse(&args(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn unknown_control_types_are_ignored_but_render_readiness_is_exact() {
        for bytes in [
            br#"{"type":"diagnostic","detail":{"nested":[1,2]}}"#.as_slice(),
            br#"{"type":"keyframe","reason":"decode"}"#,
        ] {
            assert!(serde_json::from_slice::<HelperMessage>(bytes).is_ok());
        }
        assert_eq!(
            serde_json::from_slice::<HelperMessage>(br#"{"type":"later","x":1}"#).unwrap(),
            HelperMessage::Unknown
        );
        assert!(serde_json::from_slice::<HelperMessage>(
            br#"{"type":"render_ready","ready":true,"frames":1}"#
        )
        .is_err());
    }

    #[test]
    fn input_state_keeps_field_order_while_other_commands_stay_sorted() {
        let state = ViewerCommand::InputState(InputState {
            available: true,
            enabled: false,
            focused: false,
            text_supported: true,
            shortcut_policy: None,
            content: Rect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            revision: 1,
        });
        assert_eq!(
            String::from_utf8(state.to_bytes().unwrap()).unwrap(),
            r#"{"type":"input_state","available":true,"enabled":false,"focused":false,"text_supported":true,"shortcut_policy":null,"content":{"x":0.0,"y":0.0,"width":1.0,"height":1.0},"revision":1}"#
        );
        assert_eq!(
            ViewerCommand::Viewport { id: 3 }.to_bytes().unwrap(),
            br#"{"id":3,"type":"viewport"}"#
        );
    }

    #[test]
    fn cursor_packets_keep_their_limits() {
        assert!(matches!(
            CursorPacket::parse(br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#),
            Ok(CursorPacket::Position(_))
        ));
        for invalid in [
            br#"{"revision":1,"sequence":1,"x":1.1,"y":0.2,"visible":true}"#.as_slice(),
            br#"{"revision":1,"sequence":1,"x":0.5,"y":0.2}"#,
            br#"{"revision":-1,"sequence":1,"x":0.5,"y":0.2,"visible":true}"#,
            br#"{"revision":1,"png":"","width":10000,"height":1,"hotspot_x":0,"hotspot_y":0}"#,
            br#"{"revision":1,"png":"","width":10,"height":10,"hotspot_x":513,"hotspot_y":0}"#,
            br#"{"revision":1,"png":null,"width":10,"height":10,"hotspot_x":0,"hotspot_y":0}"#,
        ] {
            assert!(
                CursorPacket::parse(invalid).is_err(),
                "{}",
                String::from_utf8_lossy(invalid)
            );
        }
        assert!(CursorPacket::parse(&vec![b' '; MAX_CURSOR_BYTES + 1]).is_err());
        assert!(CursorPacket::is_shape(br#"{"png":null}"#));
        assert!(!CursorPacket::is_shape(br#"{"revision":1}"#));
    }

    fn target() -> CaptureTarget {
        CaptureTarget {
            configuration_id: 3,
            display_id: 42,
            bounds: Bounds {
                x: -2000.0,
                y: -100.0,
                width: 2000.0,
                height: 1000.0,
            },
            canvas: Size {
                width: 1000,
                height: 800,
            },
            raster: Size {
                width: 2000,
                height: 1600,
            },
            content: Bounds {
                x: 0.0,
                y: 150.0,
                width: 1000.0,
                height: 500.0,
            },
        }
    }

    fn event(binding: &Binding, sequence: u64, command: &Command) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(
            target().event(binding, sequence, command)?,
        )?)
    }

    #[test]
    fn capture_geometry_retina_negative_origin_and_bar_rejection() {
        let target = target();
        target.validate().unwrap();
        let geometry = target.geometry().unwrap();
        assert_eq!(
            geometry.content,
            Rect {
                x: 0.0,
                y: 0.1875,
                width: 1.0,
                height: 0.625
            }
        );
        assert_eq!(
            geometry.source_size,
            Size {
                width: 2000,
                height: 1000
            }
        );
        let binding = Binding::new(2, 3);
        let good = event(
            &binding,
            1,
            &Command::Move {
                point: Point {
                    x: 0.25,
                    y: 0.65625,
                },
            },
        )
        .unwrap();
        assert_eq!(good["event"]["x"], 0.25);
        assert_eq!(good["revision"], 3);
        assert!(event(
            &binding,
            2,
            &Command::Move {
                point: Point { x: 0.25, y: 0.1 }
            }
        )
        .is_err());
        assert!(event(
            &binding,
            2,
            &Command::Input {
                point: Some(Point { x: 0.5, y: 0.5 }),
                event: InputEvent::MacKey {
                    code: 0,
                    down: true,
                    repeat: false
                }
            }
        )
        .is_err());
        let mut invalid = target.clone();
        invalid.content.height = 801.0;
        assert!(invalid.validate().is_err());
        invalid = target.clone();
        invalid.bounds.x = f64::NAN;
        assert!(invalid.validate().is_err());
        invalid = target;
        invalid.display_id = 0;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn local_button_and_scroll_envelopes_carry_their_own_point() {
        let binding = Binding::new(2, 3);
        for input in [
            InputEvent::Button {
                button: 0,
                down: true,
                clicks: 1,
            },
            InputEvent::Scroll { dx: 2, dy: -7 },
        ] {
            let value = event(
                &binding,
                12,
                &Command::Input {
                    point: Some(Point {
                        x: 0.25,
                        y: 0.65625,
                    }),
                    event: input,
                },
            )
            .unwrap();
            assert_eq!(value["event"]["x"], 0.25);
            assert_eq!(value["event"]["y"], 0.65625);
            assert_eq!(value["sequence"], 12);
        }
    }

    #[test]
    fn keyboard_and_modifiers_use_exact_binding_without_pointer_coordinates() {
        let binding = Binding::new(2, 3);
        for (index, input) in [
            InputEvent::Modifiers { mask: 9 },
            InputEvent::MacKey {
                code: 0,
                down: true,
                repeat: false,
            },
            InputEvent::MacKey {
                code: 0,
                down: true,
                repeat: true,
            },
            InputEvent::MacKey {
                code: 0,
                down: false,
                repeat: false,
            },
            InputEvent::Release,
        ]
        .into_iter()
        .enumerate()
        {
            let expected = serde_json::to_value(&input).unwrap();
            let value = event(
                &binding,
                index as u64 + 1,
                &Command::Input {
                    point: None,
                    event: input,
                },
            )
            .unwrap();
            assert_eq!(value["event"], expected);
            assert_eq!(value["session"], binding.session);
            assert_eq!(value["epoch"], 2);
            assert_eq!(value["revision"], 3);
            assert_eq!(value["sequence"], index as u64 + 1);
            assert!(value["event"].get("x").is_none());
            assert!(value["event"].get("y").is_none());
        }
        assert_eq!(
            event(&binding, 6, &Command::Stop).unwrap()["event"],
            serde_json::json!({"kind":"release"})
        );
        for input in [
            InputEvent::Modifiers { mask: 16 },
            InputEvent::MacKey {
                code: 128,
                down: true,
                repeat: false,
            },
            InputEvent::MacKey {
                code: 56,
                down: true,
                repeat: false,
            },
            InputEvent::MacKey {
                code: 0,
                down: false,
                repeat: true,
            },
        ] {
            assert!(event(
                &binding,
                7,
                &Command::Input {
                    point: None,
                    event: input
                }
            )
            .is_err());
        }
    }
}
