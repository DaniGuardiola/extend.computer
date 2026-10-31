//! What this device can do in display and input sessions. Each OS starts from
//! the defaults its build supports, so nothing changes while the display
//! helper is being asked or if it can't answer; its `capabilities` answer then
//! refines them. Peers see the result as Hello offers and the UI as
//! `Capabilities`, so both follow this one description.
use super::*;
use extend_computer_agent::{
    helper::display::Abilities as HelperAbilities,
    protocol::{offer, Offers},
    session::display::{Config, DeclineReason, Mode, VideoCodec},
};
use std::{io::Read, path::Path, process::Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Abilities {
    pub source_mirror: bool,
    pub source_extend: bool,
    /// A source applies a viewer's input to the captured display.
    pub display_control: bool,
    /// Presents a peer's mirrored or extended display, in H.264 at least.
    pub viewer: bool,
    /// The viewer sends the user's input to a source that allows control.
    pub viewer_input: bool,
    /// Every source encodes H.264.
    pub encode_hevc: bool,
    pub decode_hevc: bool,
    /// A source can send the pointer beside the video.
    pub cursor_send: bool,
    /// A viewer can draw a pointer sent beside the video.
    pub cursor_overlay: bool,
    pub input_receive: bool,
    pub input_send: bool,
}

impl Abilities {
    /// This build's defaults, from the platform adapter.
    pub fn defaults() -> Self {
        crate::platform::ABILITIES
    }

    /// The display helper speaks for what it implements; input stays as the
    /// platform defines it.
    pub fn refined(self, helper: &HelperAbilities) -> Self {
        Self {
            source_mirror: helper.sources(Mode::Mirror),
            source_extend: helper.sources(Mode::Extend),
            display_control: self.display_control && helper.target,
            viewer: helper.viewer && helper.decodes(VideoCodec::H264),
            viewer_input: helper.viewer && helper.viewer_input,
            encode_hevc: helper.encodes(VideoCodec::Hevc),
            decode_hevc: helper.decodes(VideoCodec::Hevc),
            cursor_send: helper.cursor_send,
            cursor_overlay: helper.viewer && helper.cursor_overlay,
            ..self
        }
    }

    /// The Hello `offers` tokens.
    pub fn offers(&self) -> Vec<String> {
        [
            (self.source_mirror, offer::DISPLAY_SOURCE_MIRROR),
            (self.source_extend, offer::DISPLAY_SOURCE_EXTEND),
            (self.display_control, offer::DISPLAY_CONTROL),
            (self.viewer, offer::DISPLAY_VIEWER),
            (
                self.viewer && self.viewer_input,
                offer::DISPLAY_VIEWER_INPUT,
            ),
            (self.viewer && self.decode_hevc, offer::DISPLAY_DECODE_HEVC),
            (
                self.viewer && self.cursor_overlay,
                offer::DISPLAY_CURSOR_OVERLAY,
            ),
            (self.input_receive, offer::INPUT_RECEIVE),
            (self.input_send, offer::INPUT_SEND),
        ]
        .into_iter()
        .filter(|(able, _)| *able)
        .map(|(_, token)| token.into())
        .collect()
    }

    /// What the UI offers on this device.
    pub fn capabilities(&self) -> crate::platform::Capabilities {
        crate::platform::Capabilities {
            extend_display: self.source_extend,
            mirror_display: self.source_mirror,
            share_input: self.input_send,
            remote_desktop: self.viewer && self.viewer_input,
            input_permissions: crate::platform::INPUT_PERMISSIONS,
            wifi_optimization: crate::platform::WIFI_OPTIMIZATION,
        }
    }

    pub fn sources(&self, mode: Mode) -> bool {
        match mode {
            Mode::Mirror => self.source_mirror,
            Mode::Extend => self.source_extend,
        }
    }

    /// A source's codec and pointer for one session, from the roles the peer
    /// offered in that session's own handshake. A peer that didn't say keeps
    /// the user's preference, as before offers existed.
    pub fn choose(&self, config: &mut Config, peer: &Offers) {
        if config.codec == VideoCodec::Hevc
            && !(self.encode_hevc && peer.allows(offer::DISPLAY_DECODE_HEVC))
        {
            config.codec = VideoCodec::H264;
        }
        config.separate_cursor &= self.cursor_send && peer.allows(offer::DISPLAY_CURSOR_OVERLAY);
    }

    /// A receiver's check of an incoming offer, before any helper starts. A
    /// pointer this viewer can't draw is drawn into the video instead, a
    /// downgrade the source accepts.
    pub fn admit(&self, config: &mut Config) -> std::result::Result<(), DeclineReason> {
        if !self.viewer || (config.codec == VideoCodec::Hevc && !self.decode_hevc) {
            return Err(DeclineReason::Unsupported);
        }
        config.separate_cursor &= self.cursor_overlay;
        Ok(())
    }
}

/// Runs the display helper's `capabilities` command.
fn probe(helper: &Path) -> Result<HelperAbilities> {
    let mut child = extend_computer_agent::cursor::helper_command(helper)
        .arg("capabilities")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "The display helper has no abilities report"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("The display helper abilities report timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .context("Missing display helper output")?
        .take(4096)
        .read_to_string(&mut output)?;
    HelperAbilities::parse(&output)
}

impl Desktop {
    pub(crate) fn abilities(&self) -> Abilities {
        *self.abilities.lock().unwrap()
    }

    pub(super) fn set_abilities(&self, abilities: Abilities) {
        *self.abilities.lock().unwrap() = abilities;
        extend_computer_agent::protocol::set_local_offers(abilities.offers());
    }

    /// Asks the display helper off the UI thread. Until it answers, and if it
    /// can't, this build's defaults stay in place.
    pub fn start_abilities_probe(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let Some(app) = weak.upgrade() else { return };
            let result = app.display_helper().and_then(|path| probe(&path));
            match result {
                Ok(helper) => {
                    let abilities = Abilities::defaults().refined(&helper);
                    eprintln!("Display helper abilities: {:?}", abilities.offers());
                    app.set_abilities(abilities);
                }
                Err(error) => eprintln!("Display helper abilities unavailable: {error:#}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::protocol::offer::*;

    fn config(codec: VideoCodec, separate_cursor: bool) -> Config {
        Config {
            mode: Mode::Mirror,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 20_000_000,
            retina: false,
            auto_size: false,
            codec,
            separate_cursor,
        }
    }
    fn mac() -> Abilities {
        Abilities {
            source_mirror: true,
            source_extend: true,
            display_control: true,
            viewer: true,
            viewer_input: true,
            encode_hevc: true,
            decode_hevc: true,
            cursor_send: true,
            cursor_overlay: true,
            input_receive: true,
            input_send: true,
        }
    }
    fn windows() -> Abilities {
        Abilities {
            source_extend: false,
            display_control: false,
            encode_hevc: false,
            decode_hevc: false,
            cursor_send: false,
            cursor_overlay: false,
            ..mac()
        }
    }
    fn offers(abilities: Abilities) -> Offers {
        Offers::known(abilities.offers()).unwrap()
    }

    #[test]
    fn source_choices_follow_the_peer_offers_from_this_session() {
        let viewer_only = Offers::known(vec![DISPLAY_VIEWER.into()]).unwrap();
        // (local, peer, preferred) -> (codec, separate pointer)
        for (local, peer, preferred, expected) in [
            // Peers without offers keep the preference, as before.
            (
                mac(),
                Offers::unknown(),
                (VideoCodec::Hevc, true),
                (VideoCodec::Hevc, true),
            ),
            (
                mac(),
                Offers::unknown(),
                (VideoCodec::H264, false),
                (VideoCodec::H264, false),
            ),
            (
                mac(),
                offers(mac()),
                (VideoCodec::Hevc, true),
                (VideoCodec::Hevc, true),
            ),
            (
                mac(),
                offers(windows()),
                (VideoCodec::Hevc, true),
                (VideoCodec::H264, false),
            ),
            (
                mac(),
                viewer_only.clone(),
                (VideoCodec::Hevc, true),
                (VideoCodec::H264, false),
            ),
            (
                mac(),
                offers(mac()),
                (VideoCodec::H264, true),
                (VideoCodec::H264, true),
            ),
            // A Windows source encodes H.264 and draws the pointer, whoever views.
            (
                windows(),
                Offers::unknown(),
                (VideoCodec::Hevc, true),
                (VideoCodec::H264, false),
            ),
            (
                windows(),
                offers(mac()),
                (VideoCodec::Hevc, true),
                (VideoCodec::H264, false),
            ),
            (
                windows(),
                offers(windows()),
                (VideoCodec::H264, false),
                (VideoCodec::H264, false),
            ),
        ] {
            let mut chosen = config(preferred.0, preferred.1);
            local.choose(&mut chosen, &peer);
            assert_eq!(
                (chosen.codec, chosen.separate_cursor),
                expected,
                "{local:?} {peer:?} {preferred:?}"
            );
        }
    }

    #[test]
    fn receiver_declines_what_it_cannot_show_and_draws_the_pointer_itself() {
        let mut offer = config(VideoCodec::Hevc, true);
        assert_eq!(mac().admit(&mut offer), Ok(()));
        assert!(offer.separate_cursor);
        let mut offer = config(VideoCodec::Hevc, false);
        assert_eq!(windows().admit(&mut offer), Err(DeclineReason::Unsupported));
        let mut offer = config(VideoCodec::H264, true);
        assert_eq!(windows().admit(&mut offer), Ok(()));
        assert!(!offer.separate_cursor);
        let no_viewer = Abilities {
            viewer: false,
            ..windows()
        };
        for mode in [Mode::Extend, Mode::Mirror] {
            let mut offer = config(VideoCodec::H264, false);
            offer.mode = mode;
            assert_eq!(no_viewer.admit(&mut offer), Err(DeclineReason::Unsupported));
        }
    }

    #[test]
    fn defaults_match_this_build_until_the_helper_answers() {
        let defaults = Abilities::defaults();
        #[cfg(target_os = "macos")]
        assert_eq!(defaults, mac());
        #[cfg(not(target_os = "macos"))]
        assert_eq!(defaults, windows());
        let capabilities = defaults.capabilities();
        assert!(capabilities.mirror_display && capabilities.share_input);
        assert_eq!(capabilities.extend_display, cfg!(target_os = "macos"));
        assert!(Offers::known(defaults.offers()).is_some());
    }

    #[test]
    fn platform_defaults_match_what_each_helper_reports() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../native/shared/fixtures/display-helper.json"
        ))
        .unwrap();
        let report = |from: &str| {
            let vector = fixture["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["channel"] == "capabilities" && v["from"] == from)
                .unwrap();
            HelperAbilities::parse(vector["json"].as_str().unwrap()).unwrap()
        };
        assert_eq!(mac().refined(&report("macos")), mac());
        assert_eq!(windows().refined(&report("windows")), windows());
    }

    #[test]
    fn helper_report_refines_display_roles_only() {
        let report = HelperAbilities::parse(
            r#"{"version":1,"source":["mirror"],"target":false,"viewer":true,"viewer_input":true,"encode":["h264"],"decode":["h264"]}"#,
        )
        .unwrap();
        let refined = mac().refined(&report);
        assert!(refined.source_mirror && !refined.source_extend);
        assert!(!refined.display_control && !refined.encode_hevc && !refined.decode_hevc);
        assert!(refined.viewer && refined.viewer_input && !refined.cursor_overlay);
        assert!(refined.input_receive && refined.input_send);
        assert_eq!(
            refined.offers(),
            [
                DISPLAY_SOURCE_MIRROR,
                DISPLAY_VIEWER,
                DISPLAY_VIEWER_INPUT,
                INPUT_RECEIVE,
                INPUT_SEND
            ]
        );
        // A build that doesn't know input injection can't gain it from a helper.
        let full = HelperAbilities::parse(
            r#"{"version":1,"source":["mirror","extend"],"target":true,"viewer":true,"viewer_input":true,"encode":["hevc","h264"],"decode":["hevc","h264"],"cursor_send":true,"cursor_overlay":true}"#,
        )
        .unwrap();
        assert_eq!(mac().refined(&full), mac());
        assert!(!windows().refined(&full).display_control);
    }

    #[test]
    #[cfg(unix)]
    fn failed_or_missing_probe_keeps_the_defaults() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        for (name, script) in [
            (
                "old",
                "#!/bin/sh\necho 'Choose extend, mirror or viewer.' >&2\nexit 1\n",
            ),
            ("garbage", "#!/bin/sh\necho 'capture=true virtual=true'\n"),
        ] {
            let helper = temp.path().join(name);
            std::fs::write(&helper, script).unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(probe(&helper).is_err());
        }
        assert!(probe(&temp.path().join("missing")).is_err());
        let helper = temp.path().join("current");
        std::fs::write(
            &helper,
            "#!/bin/sh\n[ \"$1\" = capabilities ] && echo '{\"version\":1,\"source\":[\"mirror\"],\"viewer\":true,\"decode\":[\"h264\"]}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let report = probe(&helper).unwrap();
        assert!(report.viewer && report.sources(Mode::Mirror));

        // A desktop whose probe never ran, failed or is still pending answers
        // peers and its own receive gate with the build's defaults.
        let app = Desktop::new(temp.path().join("app"), temp.path().join("old")).unwrap();
        assert_eq!(app.abilities(), Abilities::defaults());
        let mut offer = config(VideoCodec::Hevc, true);
        assert_eq!(
            app.abilities().admit(&mut offer).is_ok(),
            cfg!(target_os = "macos")
        );
    }
}
