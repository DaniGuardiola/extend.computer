//! What the runtime sends a viewer on its framed stdin, and the replies.
use anyhow::{bail, Context, Result};
use extend_computer_agent::{
    display::Frame,
    helper::{
        display::{Configuration, HelperMessage, Screen, ViewerCommand},
        packet::{frame, Kind},
    },
    session::display::CONFIG_LIMITS,
};

pub enum Inbound {
    Frame(Frame),
    /// A frame packet that can't be read; the viewer asks for a keyframe.
    BadFrame,
    Command(ViewerCommand),
    /// Pointer packets. The pointer is drawn into the video for this viewer.
    Ignored,
}

/// Classifies one stdin packet. Like the macOS viewer, a control message it
/// can't read or a packet kind meant for the app ends the viewer.
pub fn inbound(kind: Kind, payload: &[u8]) -> Result<Inbound> {
    Ok(match kind {
        Kind::Frame => frame::decode(payload).map_or(Inbound::BadFrame, Inbound::Frame),
        Kind::Cursor => Inbound::Ignored,
        Kind::Control => {
            Inbound::Command(serde_json::from_slice(payload).context("Invalid viewer control.")?)
        }
        Kind::Startup | Kind::ViewerInput => bail!("Invalid viewer packet."),
    })
}

/// Whether a viewer can apply new settings: the canvas it maps input to and
/// the frame rate, within the limits a launch accepts.
pub fn configuration_valid(configuration: &Configuration) -> bool {
    CONFIG_LIMITS.width.contains(&configuration.width)
        && CONFIG_LIMITS.height.contains(&configuration.height)
        && CONFIG_LIMITS.fps.contains(&configuration.fps)
}

/// The pixels per point at `dpi`, as Windows' display scale: 96 DPI is
/// 100%. A missing answer counts as 100%.
pub fn dpi_scale(dpi: u32) -> f64 {
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

/// A monitor of `width` x `height` physical pixels at `dpi`, as the macOS
/// helper reports a screen: its size in points (Windows' scaled size) and the
/// pixels per point. The runtime streams a high-density monitor in Retina and
/// another one at its physical pixels.
pub fn screen(width: u32, height: u32, dpi: u32) -> Screen {
    let scale = dpi_scale(dpi);
    let points = |pixels: u32| ((f64::from(pixels) / scale).round() as u32).max(1);
    Screen {
        width: points(width),
        height: points(height),
        scale,
        usable_width: None,
        usable_height: None,
        viewport_cg: None,
        window_number: None,
    }
}

/// The viewport reply: the window's physical size at its monitor's `dpi`,
/// in the `screen` command's terms.
pub fn viewport(id: u64, width: u32, height: u32, dpi: u32, fullscreen: bool) -> HelperMessage {
    HelperMessage::Viewport {
        id,
        fullscreen,
        screen: screen(width, height, dpi),
    }
}

/// Counts fresh frames shown; reports the first and every 60th.
#[derive(Debug, Default)]
pub struct Presented(u64);
impl Presented {
    pub fn shown(&mut self) -> Option<HelperMessage> {
        self.0 += 1;
        (self.0 == 1 || self.0.is_multiple_of(60))
            .then_some(HelperMessage::Presented { frames: self.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::helper::packet::{read_packet, write_packet};

    #[test]
    fn framed_stdin_is_classified_like_the_macos_viewer() {
        let mut stream = Vec::new();
        let picture = Frame {
            timestamp: 5,
            keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 2],
        };
        write_packet(&mut stream, Kind::Frame, &frame::encode(&picture)).unwrap();
        write_packet(&mut stream, Kind::Frame, &[0; 4]).unwrap();
        write_packet(&mut stream, Kind::Cursor, br#"{"revision":1}"#).unwrap();
        for command in [
            ViewerCommand::Viewport { id: 3 },
            ViewerCommand::Configuration(Configuration {
                id: 4,
                width: 1920,
                height: 1080,
                retina: false,
                fps: 60,
                bitrate: 12_000_000,
            }),
            ViewerCommand::Stop,
        ] {
            write_packet(&mut stream, Kind::Control, &command.to_bytes().unwrap()).unwrap();
        }
        let mut input = stream.as_slice();
        let mut read = Vec::new();
        while let Some((kind, payload)) = read_packet(&mut input).unwrap() {
            read.push(inbound(kind, &payload).unwrap());
        }
        assert!(matches!(
            &read[0],
            Inbound::Frame(Frame { timestamp: 5, keyframe: true, bytes }) if *bytes == picture.bytes
        ));
        assert!(matches!(read[1], Inbound::BadFrame));
        assert!(matches!(read[2], Inbound::Ignored));
        assert!(matches!(
            read[3],
            Inbound::Command(ViewerCommand::Viewport { id: 3 })
        ));
        assert!(matches!(
            &read[4],
            Inbound::Command(ViewerCommand::Configuration(c)) if configuration_valid(c)
        ));
        assert!(matches!(read[5], Inbound::Command(ViewerCommand::Stop)));

        assert!(inbound(Kind::Control, br#"{"type":"teleport"}"#).is_err());
        assert!(inbound(Kind::Control, br#"{"type":"configuration","id":1}"#).is_err());
        assert!(inbound(Kind::Control, b"").is_err());
        assert!(inbound(Kind::Startup, br#"{"ready":true}"#).is_err());
        assert!(inbound(Kind::ViewerInput, br#"{}"#).is_err());
        // A truncated stream is an error, not a clean end.
        assert!(read_packet(&mut &stream[..7]).is_err());
    }

    #[test]
    fn screens_report_points_and_the_display_scale() {
        let size = |screen: Screen| (screen.width, screen.height, screen.scale);
        // 100% keeps physical pixels at scale 1, the bytes older builds sent.
        assert_eq!(size(screen(1920, 1080, 96)), (1920, 1080, 1.0));
        assert_eq!(
            String::from_utf8(screen(1920, 1080, 96).to_bytes().unwrap()).unwrap(),
            r#"{"height":1080,"scale":1.0,"width":1920}"#
        );
        assert_eq!(size(screen(1920, 1080, 120)), (1536, 864, 1.25));
        assert_eq!(size(screen(2560, 1600, 144)), (1707, 1067, 1.5));
        assert_eq!(size(screen(2880, 1800, 192)), (1440, 900, 2.0));
        assert_eq!(size(screen(3840, 2160, 168)), (2194, 1234, 1.75));
        // No answer from Windows counts as 100%.
        assert_eq!(size(screen(1366, 768, 0)), (1366, 768, 1.0));
    }

    #[test]
    fn configurations_outside_launch_limits_are_refused() {
        let valid = Configuration {
            id: 1,
            width: 640,
            height: 480,
            retina: true,
            fps: 120,
            bitrate: 1_000_000,
        };
        assert!(configuration_valid(&valid));
        for invalid in [
            Configuration {
                width: 639,
                ..valid.clone()
            },
            Configuration {
                height: 4097,
                ..valid.clone()
            },
            Configuration {
                fps: 29,
                ..valid.clone()
            },
        ] {
            assert!(!configuration_valid(&invalid));
        }
    }

    #[test]
    fn replies_have_the_bytes_the_runtime_reads() {
        let reply = viewport(3, 1920, 1080, 96, true);
        assert_eq!(
            String::from_utf8(reply.to_bytes().unwrap()).unwrap(),
            r#"{"fullscreen":true,"height":1080,"id":3,"scale":1.0,"type":"viewport","width":1920}"#
        );
        assert_eq!(
            serde_json::from_slice::<HelperMessage>(&reply.to_bytes().unwrap()).unwrap(),
            reply
        );
        assert_eq!(
            String::from_utf8(viewport(4, 2880, 1800, 192, true).to_bytes().unwrap()).unwrap(),
            r#"{"fullscreen":true,"height":900,"id":4,"scale":2.0,"type":"viewport","width":1440}"#
        );
        let mut presented = Presented::default();
        let reports: Vec<u64> = (0..180)
            .filter_map(|_| match presented.shown() {
                Some(HelperMessage::Presented { frames }) => Some(frames),
                _ => None,
            })
            .collect();
        assert_eq!(reports, vec![1, 60, 120, 180]);
        for (message, json) in [
            (
                HelperMessage::RenderReady(extend_computer_agent::helper::display::RenderReady {
                    ready: true,
                }),
                r#"{"ready":true,"type":"render_ready"}"#,
            ),
            (HelperMessage::Keyframe, r#"{"type":"keyframe"}"#),
            (HelperMessage::Closed, r#"{"type":"closed"}"#),
            (
                HelperMessage::Configured { id: 2, ok: false },
                r#"{"id":2,"ok":false,"type":"configured"}"#,
            ),
        ] {
            let bytes = message.to_bytes().unwrap();
            assert_eq!(String::from_utf8(bytes.clone()).unwrap(), json);
            assert_eq!(
                serde_json::from_slice::<HelperMessage>(&bytes).unwrap(),
                message
            );
        }
    }
}
