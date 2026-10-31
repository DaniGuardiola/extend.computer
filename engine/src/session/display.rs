//! Display negotiation is encrypted and pinned before any media resources start.
use super::*;
pub mod settings;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Extend,
    Mirror,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
}
impl VideoCodec {
    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub retina: bool,
    #[serde(default)]
    pub auto_size: bool,
    #[serde(default)]
    pub codec: VideoCodec,
    #[serde(default)]
    pub separate_cursor: bool,
}
/// The settings the session and every display helper accept.
pub struct ConfigLimits {
    pub width: std::ops::RangeInclusive<u32>,
    pub height: std::ops::RangeInclusive<u32>,
    pub fps: std::ops::RangeInclusive<u32>,
    pub bitrate: std::ops::RangeInclusive<u32>,
    /// Largest encoded side, after Retina doubles the size.
    pub encoded: u32,
}
pub const CONFIG_LIMITS: ConfigLimits = ConfigLimits {
    width: 640..=4096,
    height: 480..=4096,
    fps: 30..=120,
    bitrate: 1_000_000..=80_000_000,
    encoded: 4096,
};
impl Config {
    pub fn validate(&self) -> Result<()> {
        let limits = &CONFIG_LIMITS;
        let scale = if self.retina { 2 } else { 1 };
        ensure!(
            limits.width.contains(&self.width)
                && limits.height.contains(&self.height)
                && self.width * scale <= limits.encoded
                && self.height * scale <= limits.encoded
                && limits.fps.contains(&self.fps)
                && limits.bitrate.contains(&self.bitrate),
            "Invalid display configuration"
        );
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Signal {
    Offer {
        config: Config,
        sdp: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        receiver_settings: bool,
    },
    Answer {
        config: Config,
        sdp: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        receiver_settings: bool,
    },
    Ping {
        sequence: u64,
    },
    Pong {
        sequence: u64,
    },
    Configure {
        config: Config,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    Configured {
        config: Option<Config>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<settings::Outcome>,
    },
    SettingsPong {
        sequence: u64,
        request: Option<settings::Request>,
    },
    SettingsResult {
        result: settings::Completion,
    },
    Close,
}
pub struct DisplaySignal {
    channel: Channel,
    last_heartbeat: Option<Instant>,
    receiver_settings: bool,
    revision: u64,
    settings_request: u64,
    config: Config,
}
/// Negotiated wire support, not authorization to inject input. The enabled value
/// is only produced after the pinned encrypted compatibility exchange.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DisplayFeatures {
    input: bool,
    text: bool,
    shortcuts: bool,
}
impl DisplayFeatures {
    fn negotiated(channel: &Channel) -> Self {
        Self {
            input: channel.supports_capability(crate::protocol::DISPLAY_INPUT),
            text: channel.supports_capability(crate::protocol::INPUT_TEXT),
            shortcuts: channel.supports_capability(crate::protocol::INPUT_SHORTCUTS),
        }
    }
    pub fn shortcuts_enabled(self) -> bool {
        self.input && self.shortcuts
    }
    pub fn text_enabled(self) -> bool {
        self.input && self.text
    }
    pub fn input_enabled(self) -> bool {
        self.input
    }
    /// A platform without an input adapter can always withhold the channel.
    pub fn without_input(self) -> Self {
        Self::default()
    }
    #[cfg(test)]
    pub(crate) fn shortcut_input_for_test() -> Self {
        Self {
            input: true,
            text: false,
            shortcuts: true,
        }
    }
    #[cfg(test)]
    pub(crate) fn input_for_test() -> Self {
        Self {
            input: true,
            text: false,
            shortcuts: false,
        }
    }
}
impl DisplaySignal {
    pub fn connect(
        stream: TcpStream,
        identity: &Identity,
        peer: &str,
        config: &Config,
        offer: &str,
    ) -> Result<(Self, Config, String)> {
        let (signal, selected, sdp, _) =
            Self::connect_with_offer(stream, identity, peer, config, |_| Ok(offer.into()))?;
        Ok((signal, selected, sdp))
    }
    /// Build SDP only after the receiver's identity and capabilities are pinned.
    /// Existing callers using connect() continue to offer the legacy channels.
    pub fn connect_with_offer(
        stream: TcpStream,
        identity: &Identity,
        peer: &str,
        config: &Config,
        make_offer: impl FnOnce(DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, Config, String, DisplayFeatures)> {
        Self::connect_with_settings_offer(stream, identity, peer, config, false, make_offer)
    }
    pub fn connect_with_settings_offer(
        mut stream: TcpStream,
        identity: &Identity,
        peer: &str,
        config: &Config,
        enable_settings: bool,
        make_offer: impl FnOnce(DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, Config, String, DisplayFeatures)> {
        config.validate()?;
        wire::configure(&stream)?;
        stream.write_all(MAGIC)?;
        stream.write_all(b"D")?;
        let (mut channel, _, _) = handshake(
            stream,
            identity,
            None,
            true,
            Instant::now() + Duration::from_secs(5),
            b"extend.computer/v1/display",
            Some(peer),
        )?;
        let features = DisplayFeatures::negotiated(&channel);
        let offer = make_offer(features)?;
        channel.display_signaling();
        let offered_settings =
            enable_settings && channel.supports_capability(crate::protocol::RECEIVER_SETTINGS);
        channel.send(&Signal::Offer {
            config: config.clone(),
            sdp: offer,
            receiver_settings: offered_settings,
        })?;
        let Signal::Answer {
            config: selected,
            sdp,
            receiver_settings,
        } = channel.receive()?
        else {
            bail!(EngineError::RequestRejected)
        };
        selected.validate()?;
        ensure!(
            selected.codec == config.codec
                && (!selected.separate_cursor || config.separate_cursor)
                && selected.mode == config.mode
                && selected.fps <= config.fps
                && selected.bitrate <= config.bitrate,
            "Invalid display answer"
        );
        ensure!(
            config.auto_size
                || (selected.width == config.width
                    && selected.height == config.height
                    && selected.retina == config.retina),
            "Display size changed without negotiation"
        );
        ensure!(
            !receiver_settings || offered_settings,
            "Unexpected receiver settings capability"
        );
        Ok((
            Self {
                channel,
                last_heartbeat: None,
                receiver_settings,
                revision: 0,
                settings_request: 0,
                config: selected.clone(),
            },
            selected,
            sdp,
            features,
        ))
    }
    pub fn accept(
        stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        answer: impl FnOnce(&str, &mut Config, &str) -> Result<String>,
    ) -> Result<(Self, String)> {
        let (signal, peer, _) =
            Self::accept_with_features(stream, identity, store, |peer, config, sdp, _| {
                answer(peer, config, sdp)
            })?;
        Ok((signal, peer))
    }
    pub fn accept_with_features(
        stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        answer: impl FnOnce(&str, &mut Config, &str, DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, String, DisplayFeatures)> {
        Self::accept_with_settings_features(stream, identity, store, false, answer)
    }
    pub fn accept_with_settings_features(
        stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        enable_settings: bool,
        answer: impl FnOnce(&str, &mut Config, &str, DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, String, DisplayFeatures)> {
        Self::accept_with_settings_features_inner(
            stream,
            identity,
            store,
            None,
            enable_settings,
            answer,
        )
    }
    /// A private carrier selects one peer. Authenticate that exact identity
    /// before exchanging capabilities or creating decoder/media resources.
    pub fn accept_with_settings_features_pinned(
        stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        expected_peer: &str,
        enable_settings: bool,
        answer: impl FnOnce(&str, &mut Config, &str, DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, String, DisplayFeatures)> {
        Self::accept_with_settings_features_inner(
            stream,
            identity,
            store,
            Some(expected_peer),
            enable_settings,
            answer,
        )
    }
    fn accept_with_settings_features_inner(
        mut stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        expected_peer: Option<&str>,
        enable_settings: bool,
        answer: impl FnOnce(&str, &mut Config, &str, DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, String, DisplayFeatures)> {
        wire::configure(&stream)?;
        let mut hello = [0; 9];
        wire::read_exact_until(
            &mut stream,
            &mut hello,
            Instant::now() + Duration::from_secs(5),
        )?;
        ensure!(
            &hello[..8] == MAGIC && hello[8] == b'D',
            EngineError::ProtocolIncompatible
        );
        let (mut channel, peer, _) = handshake(
            stream,
            identity,
            None,
            false,
            Instant::now() + Duration::from_secs(5),
            b"extend.computer/v1/display",
            expected_peer,
        )?;
        ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
        let features = DisplayFeatures::negotiated(&channel);
        channel.display_signaling();
        let Signal::Offer {
            mut config,
            sdp,
            receiver_settings,
        } = channel.receive()?
        else {
            bail!(EngineError::RequestRejected)
        };
        config.validate()?;
        let sdp = answer(&peer, &mut config, &sdp, features)?;
        config.validate()?;
        ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
        let receiver_settings = receiver_settings
            && enable_settings
            && channel.supports_capability(crate::protocol::RECEIVER_SETTINGS);
        channel.send(&Signal::Answer {
            config: config.clone(),
            sdp,
            receiver_settings,
        })?;
        Ok((
            Self {
                channel,
                last_heartbeat: None,
                receiver_settings,
                revision: 0,
                settings_request: 0,
                config,
            },
            peer,
            features,
        ))
    }
    pub fn pulse(&mut self, sequence: u64) -> Result<()> {
        self.channel.send(&Signal::Ping { sequence })?;
        let started = Instant::now();
        let reply = self.channel.receive().with_context(|| {
            format!(
                "Display heartbeat response after {} ms",
                started.elapsed().as_millis()
            )
        })?;
        ensure!(
            matches!(reply, Signal::Pong { sequence: n } if n == sequence),
            "Display connection ended"
        );
        self.last_heartbeat = Some(Instant::now());
        Ok(())
    }
    /// Freshness of a verified heartbeat on this pinned encrypted channel.
    /// Configuration traffic and a live thread do not renew this timestamp.
    pub fn last_heartbeat(&self) -> Option<Instant> {
        self.last_heartbeat
    }
    pub fn respond(&mut self) -> Result<bool> {
        self.respond_with_config(|_| bail!(EngineError::RequestRejected))
    }
    pub fn receiver_settings_supported(&self) -> bool {
        self.receiver_settings
    }
    /// The closure must return only after the source capture barrier completes.
    /// Applied is source confirmation, not proof that the receiver rendered a frame.
    pub fn pulse_with_settings(
        &mut self,
        sequence: u64,
        apply: impl FnOnce(&mut Self, Config) -> Result<Config>,
    ) -> Result<()> {
        if !self.receiver_settings {
            return self.pulse(sequence);
        }
        self.channel.send(&Signal::Ping { sequence })?;
        let Signal::SettingsPong {
            sequence: received,
            request,
        } = self.channel.receive()?
        else {
            bail!(EngineError::RequestRejected)
        };
        ensure!(received == sequence, "Invalid display heartbeat");
        self.last_heartbeat = Some(Instant::now());
        if let Some(request) = request {
            ensure!(
                request.id > self.settings_request,
                "Invalid settings request"
            );
            self.settings_request = request.id;
            let outcome = if request.expected_revision != self.revision {
                settings::Outcome::Stale
            } else if let Ok(config) = request.settings.apply(&self.config) {
                let previous_revision = self.revision;
                match apply(self, config) {
                    Ok(applied) => {
                        ensure!(
                            self.revision == previous_revision + 1 && applied == self.config,
                            "Source settings barrier did not configure"
                        );
                        settings::Outcome::Applied
                    }
                    Err(error) if error.is::<settings::Rejected>() => settings::Outcome::Rejected,
                    Err(error) => {
                        self.channel.send(&Signal::SettingsResult {
                            result: settings::Completion {
                                id: request.id,
                                revision: self.revision,
                                outcome: settings::Outcome::Failed,
                            },
                        })?;
                        return Err(error);
                    }
                }
            } else {
                settings::Outcome::Rejected
            };
            self.channel.send(&Signal::SettingsResult {
                result: settings::Completion {
                    id: request.id,
                    revision: self.revision,
                    outcome,
                },
            })?;
        }
        Ok(())
    }
    pub fn respond_with_settings(
        &mut self,
        mailbox: &settings::ReceiverSettings,
        mut preflight: impl FnMut(&Config) -> Result<()>,
        mut apply: impl FnMut(Config) -> Result<Config>,
    ) -> Result<bool> {
        if !self.receiver_settings {
            return self.respond_with_config(apply);
        }
        match self.channel.receive()? {
            Signal::Ping { sequence } => {
                self.channel.send(&Signal::SettingsPong {
                    sequence,
                    request: mailbox.take(),
                })?;
                self.last_heartbeat = Some(Instant::now());
                Ok(true)
            }
            Signal::Configure { config, revision } => {
                config.validate()?;
                let revision = revision.context("Missing display revision")?;
                ensure!(
                    revision == self.revision.checked_add(1).unwrap_or(0),
                    "Stale display revision"
                );
                if config.mode != self.config.mode
                    || config.codec != self.config.codec
                    || config.separate_cursor != self.config.separate_cursor
                    || preflight(&config).is_err()
                {
                    self.channel.send(&Signal::Configured {
                        config: None,
                        outcome: Some(settings::Outcome::Rejected),
                    })?;
                    return Ok(true);
                }
                let applied = match apply(config) {
                    Ok(config) => config,
                    Err(error) => {
                        self.channel.send(&Signal::Configured {
                            config: None,
                            outcome: Some(settings::Outcome::Failed),
                        })?;
                        return Err(error);
                    }
                };
                let applied = Some(applied);
                if let Some(config) = &applied {
                    config.validate()?;
                    mailbox.configured(revision, config)?;
                    self.revision = revision;
                    self.config = config.clone();
                }
                self.channel.send(&Signal::Configured {
                    config: applied,
                    outcome: None,
                })?;
                Ok(true)
            }
            Signal::SettingsResult { result } => {
                mailbox.complete(result)?;
                Ok(true)
            }
            Signal::Close => {
                mailbox.close();
                Ok(false)
            }
            _ => bail!(EngineError::RequestRejected),
        }
    }
    pub fn configure(&mut self, config: &Config) -> Result<Config> {
        config.validate()?;
        let next_revision = self
            .revision
            .checked_add(1)
            .context("Display revision limit")?;
        self.channel.send(&Signal::Configure {
            config: config.clone(),
            revision: self.receiver_settings.then_some(next_revision),
        })?;
        match self.channel.receive()? {
            Signal::Configured {
                config: Some(applied),
                outcome: None,
            } => {
                applied.validate()?;
                ensure!(
                    applied.mode == config.mode
                        && applied.codec == config.codec
                        && applied.separate_cursor == config.separate_cursor
                        && applied.fps == config.fps
                        && applied.bitrate == config.bitrate
                        && (config.auto_size
                            || (applied.width == config.width
                                && applied.height == config.height
                                && applied.retina == config.retina)),
                    "Invalid display settings answer"
                );
                self.revision = next_revision;
                self.config = applied.clone();
                Ok(applied)
            }
            Signal::Configured {
                config: None,
                outcome: Some(settings::Outcome::Rejected),
            } if self.receiver_settings => Err(settings::Rejected.into()),
            _ => bail!(EngineError::RequestRejected),
        }
    }
    pub fn respond_with_config(
        &mut self,
        mut apply: impl FnMut(Config) -> Result<Config>,
    ) -> Result<bool> {
        let started = Instant::now();
        let request = self.channel.receive().with_context(|| {
            format!(
                "Display heartbeat wait after {} ms",
                started.elapsed().as_millis()
            )
        })?;
        match request {
            Signal::Configure { config, revision } => {
                ensure!(
                    !self.receiver_settings && revision.is_none(),
                    "Settings receiver adapter missing"
                );
                config.validate()?;
                let applied = apply(config).ok();
                self.channel.send(&Signal::Configured {
                    config: applied,
                    outcome: None,
                })?;
                Ok(true)
            }
            Signal::Ping { sequence } => {
                self.channel.send(&Signal::Pong { sequence })?;
                self.last_heartbeat = Some(Instant::now());
                Ok(true)
            }
            Signal::Close => Ok(false),
            _ => bail!(EngineError::RequestRejected),
        }
    }
    pub fn close(&mut self) {
        let _ = self.channel.send(&Signal::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heartbeat_freshness_rejects_authenticated_wrong_sequence() {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let pin = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let store = TrustStore::open(root.path()).unwrap();
        store.remember(&source.fingerprint()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut signal, _) = DisplaySignal::accept(
                listener.accept().unwrap().0,
                &receiver,
                &store,
                |_, _, _| Ok("answer".into()),
            )
            .unwrap();
            assert!(matches!(
                signal.channel.receive().unwrap(),
                Signal::Ping { sequence: 7 }
            ));
            signal.channel.send(&Signal::Pong { sequence: 8 }).unwrap();
        });
        let (mut signal, _, _) = DisplaySignal::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &pin,
            &input_config(),
            "offer",
        )
        .unwrap();
        assert!(signal.pulse(7).is_err());
        assert!(signal.last_heartbeat().is_none());
        server.join().unwrap();
    }

    #[test]
    fn receiver_settings_pinned_apply_reject_stale_and_stop() {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let store = TrustStore::open(root.path()).unwrap();
        store.remember(&source.fingerprint()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut signal, _, _) = DisplaySignal::accept_with_settings_features(
                listener.accept().unwrap().0,
                &receiver,
                &store,
                true,
                |_, _, _, _| Ok("answer".into()),
            )
            .unwrap();
            assert!(signal.receiver_settings_supported());
            assert!(signal.last_heartbeat().is_none());
            let mailbox = settings::ReceiverSettings::new(true, input_config());
            let change = |fps| settings::Settings {
                width: 960,
                height: 600,
                fps,
                bitrate: 8_000_000,
                auto_size: false,
            };
            mailbox.request(0, change(30)).unwrap();
            ready.send(()).unwrap();
            for _ in 0..3 {
                assert!(signal
                    .respond_with_settings(
                        &mailbox,
                        |config| {
                            ensure!(config.fps != 30, "Unsupported decoder rate");
                            Ok(())
                        },
                        Ok,
                    )
                    .unwrap());
            }
            assert!(signal.last_heartbeat().is_some());
            assert_eq!(
                mailbox.snapshot().result.unwrap().outcome,
                settings::Outcome::Rejected
            );
            assert_eq!(mailbox.snapshot().config, input_config());
            mailbox.request(0, change(60)).unwrap();
            ready.send(()).unwrap();
            for _ in 0..3 {
                signal
                    .respond_with_settings(&mailbox, |_| Ok(()), Ok)
                    .unwrap();
            }
            assert_eq!(
                mailbox.snapshot().result.unwrap().outcome,
                settings::Outcome::Applied
            );
            mailbox.request(1, change(60)).unwrap();
            ready.send(()).unwrap();
            for _ in 0..3 {
                signal
                    .respond_with_settings(&mailbox, |_| Ok(()), Ok)
                    .unwrap();
            }
            assert_eq!(
                mailbox.snapshot().result.unwrap().outcome,
                settings::Outcome::Stale
            );
            assert_eq!(mailbox.snapshot().revision, 2);
            mailbox.request(2, change(60)).unwrap();
            ready.send(()).unwrap();
            assert!(!signal
                .respond_with_settings(&mailbox, |_| Ok(()), Ok)
                .unwrap());
            assert_eq!(
                mailbox.snapshot().result.unwrap().outcome,
                settings::Outcome::Closed
            );
        });
        let (mut signal, _, _, _) = DisplaySignal::connect_with_settings_offer(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            &input_config(),
            true,
            |_| Ok("offer".into()),
        )
        .unwrap();
        assert!(signal.last_heartbeat().is_none());
        wait.recv().unwrap();
        signal
            .pulse_with_settings(0, |signal, config| signal.configure(&config))
            .unwrap();
        assert!(signal.last_heartbeat().is_some());
        wait.recv().unwrap();
        signal
            .pulse_with_settings(1, |signal, config| signal.configure(&config))
            .unwrap();
        wait.recv().unwrap();
        signal.configure(&input_config()).unwrap();
        signal
            .pulse_with_settings(2, |_, _| panic!("Stale receiver request applied"))
            .unwrap();
        wait.recv().unwrap();
        signal.close();
        server.join().unwrap();
    }
    #[test]
    fn settings_fields_are_omitted_for_strict_legacy_peers() {
        let offer = serde_json::to_value(Signal::Offer {
            config: input_config(),
            sdp: "offer".into(),
            receiver_settings: false,
        })
        .unwrap();
        assert!(offer.get("receiver_settings").is_none());
        let configure = serde_json::to_value(Signal::Configure {
            config: input_config(),
            revision: None,
        })
        .unwrap();
        assert!(configure.get("revision").is_none());
        let configured = serde_json::to_value(Signal::Configured {
            config: None,
            outcome: None,
        })
        .unwrap();
        assert!(configured.get("outcome").is_none());
    }
    #[test]
    fn receiver_settings_adapter_opt_in_is_required_on_both_ends() {
        for (source_enabled, receiver_enabled) in [(false, true), (true, false)] {
            let source = Identity::generate();
            let receiver = Identity::generate();
            let peer = receiver.fingerprint();
            let root = tempfile::tempdir().unwrap();
            let store = TrustStore::open(root.path()).unwrap();
            store.remember(&source.fingerprint()).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut signal, _, _) = DisplaySignal::accept_with_settings_features(
                    listener.accept().unwrap().0,
                    &receiver,
                    &store,
                    receiver_enabled,
                    |_, _, _, _| Ok("answer".into()),
                )
                .unwrap();
                assert!(!signal.receiver_settings_supported());
                signal.respond().unwrap();
            });
            let (mut signal, _, _, _) = DisplaySignal::connect_with_settings_offer(
                TcpStream::connect(address).unwrap(),
                &source,
                &peer,
                &input_config(),
                source_enabled,
                |_| Ok("offer".into()),
            )
            .unwrap();
            assert!(!signal.receiver_settings_supported());
            signal
                .pulse_with_settings(0, |_, _| panic!("Unsupported settings applied"))
                .unwrap();
            server.join().unwrap();
        }
    }
    #[test]
    fn receiver_settings_source_helper_failure_never_reports_applied() {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let store = TrustStore::open(root.path()).unwrap();
        store.remember(&source.fingerprint()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut signal, _, _) = DisplaySignal::accept_with_settings_features(
                listener.accept().unwrap().0,
                &receiver,
                &store,
                true,
                |_, _, _, _| Ok("answer".into()),
            )
            .unwrap();
            let mailbox = settings::ReceiverSettings::new(true, input_config());
            mailbox
                .request(
                    0,
                    settings::Settings {
                        width: 960,
                        height: 600,
                        auto_size: false,
                        fps: 60,
                        bitrate: 8_000_000,
                    },
                )
                .unwrap();
            ready.send(()).unwrap();
            for _ in 0..3 {
                signal
                    .respond_with_settings(&mailbox, |_| Ok(()), Ok)
                    .unwrap();
            }
            assert_eq!(
                mailbox.snapshot().result.unwrap().outcome,
                settings::Outcome::Failed
            );
            // Preparation happened. A failed source barrier does not roll it back.
            assert_eq!(mailbox.snapshot().config.width, 960);
        });
        let (mut signal, _, _, _) = DisplaySignal::connect_with_settings_offer(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            &input_config(),
            true,
            |_| Ok("offer".into()),
        )
        .unwrap();
        wait.recv().unwrap();
        assert!(signal
            .pulse_with_settings(0, |signal, config| {
                signal.configure(&config)?;
                bail!("Source helper failed after receiver preparation")
            })
            .is_err());
        server.join().unwrap();
    }
    fn input_config() -> Config {
        Config {
            mode: Mode::Extend,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 8_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::H264,
            separate_cursor: true,
        }
    }
    #[test]
    fn display_features_are_provided_only_after_pinned_authenticated_handshake() {
        for correct_pin in [true, false] {
            let source = Identity::generate();
            let receiver = Identity::generate();
            let peer = if correct_pin {
                receiver.fingerprint()
            } else {
                Identity::generate().fingerprint()
            };
            let root = tempfile::tempdir().unwrap();
            let store = TrustStore::open(root.path()).unwrap();
            store.remember(&source.fingerprint()).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                DisplaySignal::accept_with_features(
                    listener.accept().unwrap().0,
                    &receiver,
                    &store,
                    |_, _, offer, features| {
                        assert!(features.input_enabled());
                        assert_eq!(offer, "negotiated-offer");
                        Ok("answer".into())
                    },
                )
            });
            let mut called = false;
            let result = DisplaySignal::connect_with_offer(
                TcpStream::connect(address).unwrap(),
                &source,
                &peer,
                &input_config(),
                |features| {
                    called = true;
                    assert!(features.input_enabled());
                    assert!(!features.without_input().input_enabled());
                    Ok("negotiated-offer".into())
                },
            );
            assert_eq!(called, correct_pin);
            assert_eq!(result.is_ok(), correct_pin);
            assert_eq!(server.join().unwrap().is_ok(), correct_pin);
        }
    }
    #[test]
    fn legacy_encrypted_peer_withholds_display_input_capability() {
        // Emulate the old responder's exact encrypted Hello/Offer schemas,
        // rather than relying on application SemVer or a discovery hint.
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
        let source_peer = source.fingerprint();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || -> Result<()> {
            let mut stream = listener.accept()?.0;
            wire::configure(&stream)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut hello = [0; 9];
            wire::read_exact_until(&mut stream, &mut hello, deadline)?;
            ensure!(
                &hello[..8] == MAGIC && hello[8] == b'D',
                "Invalid legacy greeting"
            );
            let mut noise = Builder::new(BASE.parse()?)
                .local_private_key(receiver.secret())?
                .prologue(b"extend.computer/v1/display")?
                .build_responder()?;
            let mut output = [0; wire::MAX_FRAME];
            for turn in 0..3 {
                if turn == 1 {
                    let len = noise.write_message(&[], &mut output)?;
                    wire::write_frame(&mut stream, &output[..len])?;
                } else {
                    let bytes = wire::read_frame_until(&mut stream, deadline)?;
                    ensure!(
                        noise.read_message(&bytes, &mut output)? == 0,
                        "Invalid legacy handshake"
                    );
                }
            }
            ensure!(
                fingerprint(noise.get_remote_static().unwrap()) == source_peer,
                "Invalid source pin"
            );
            let mut channel = Channel::new(stream, noise.into_transport_mode()?);
            channel.send(&serde_json::json!({"min_protocol":1,"max_protocol":1,
                "app_version":"0.1","capabilities":[crate::protocol::CONTROL]}))?;
            let hello: serde_json::Value = channel.receive()?;
            ensure!(
                hello["capabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == crate::protocol::DISPLAY_INPUT),
                "Missing current capability"
            );
            channel.display_signaling();
            let Signal::Offer {
                config,
                sdp,
                receiver_settings: false,
            } = channel.receive()?
            else {
                bail!("Invalid legacy offer")
            };
            ensure!(sdp == "legacy-offer", "Unexpected legacy offer");
            channel.send(&Signal::Answer {
                config,
                sdp: "legacy-answer".into(),
                receiver_settings: false,
            })
        });
        let (_, _, answer, features) = DisplaySignal::connect_with_offer(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            &input_config(),
            |features| {
                assert!(!features.input_enabled());
                Ok("legacy-offer".into())
            },
        )
        .unwrap();
        assert_eq!(answer, "legacy-answer");
        assert!(!features.input_enabled());
        server.join().unwrap().unwrap();
    }
    #[test]
    fn settings_updates_are_acknowledged_and_rejection_keeps_signaling_alive() {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let pin = receiver.fingerprint();
        let root = tempfile::tempdir().unwrap();
        let store = TrustStore::open(root.path()).unwrap();
        store.remember(&source.fingerprint()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut signal, _) = DisplaySignal::accept(
                listener.accept().unwrap().0,
                &receiver,
                &store,
                |_, _, _| Ok("answer".into()),
            )
            .unwrap();
            assert!(signal.last_heartbeat().is_none());
            assert!(signal
                .respond_with_config(|config| {
                    assert_eq!(config.fps, 30);
                    Ok(config)
                })
                .unwrap());
            assert!(signal.last_heartbeat().is_none());
            assert!(signal
                .respond_with_config(|_| bail!("Native update rejected"))
                .unwrap());
            assert!(signal.last_heartbeat().is_none());
            assert!(signal.respond().unwrap());
            assert!(signal.last_heartbeat().is_some());
        });
        let config = Config {
            mode: Mode::Extend,
            width: 1280,
            height: 720,
            fps: 60,
            bitrate: 20_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::Hevc,
            separate_cursor: true,
        };
        let (mut signal, _, _) = DisplaySignal::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &pin,
            &config,
            "offer",
        )
        .unwrap();
        assert!(signal.last_heartbeat().is_none());
        let mut next = config;
        next.fps = 30;
        next.bitrate = 8_000_000;
        assert_eq!(signal.configure(&next).unwrap(), next);
        assert!(signal.configure(&next).is_err());
        assert!(signal.last_heartbeat().is_none());
        signal.pulse(99).unwrap();
        assert!(signal.last_heartbeat().is_some());
        server.join().unwrap();
    }
    #[test]
    fn receiver_can_resize_only_when_requested() {
        for auto_size in [false, true] {
            let source = Identity::generate();
            let receiver = Identity::generate();
            let pin = receiver.fingerprint();
            let root = tempfile::tempdir().unwrap();
            let store = TrustStore::open(root.path()).unwrap();
            store.remember(&source.fingerprint()).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                DisplaySignal::accept(
                    listener.accept().unwrap().0,
                    &receiver,
                    &store,
                    |_, config, _| {
                        config.width = 1920;
                        config.height = 1080;
                        Ok("answer".repeat(2048))
                    },
                )
            });
            let config = Config {
                mode: Mode::Extend,
                width: 1280,
                height: 720,
                fps: 60,
                bitrate: 10_000_000,
                retina: false,
                auto_size,
                codec: VideoCodec::H264,
                separate_cursor: false,
            };
            let result = DisplaySignal::connect(
                TcpStream::connect(address).unwrap(),
                &source,
                &pin,
                &config,
                &"offer".repeat(2048),
            );
            assert_eq!(result.is_ok(), auto_size);
            if let Ok((_, selected, _)) = result {
                assert_eq!((selected.width, selected.height), (1920, 1080));
            }
            assert!(server.join().unwrap().is_ok());
        }
    }
    #[test]
    fn display_offer_requires_pinned_identity_and_existing_trust() {
        for (trusted, correct_pin, revoke_during_offer) in [
            (false, true, false),
            (true, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            let source = Identity::generate();
            let receiver = Identity::generate();
            let pin = if correct_pin {
                receiver.fingerprint()
            } else {
                Identity::generate().fingerprint()
            };
            let root = tempfile::tempdir().unwrap();
            let store = TrustStore::open(root.path()).unwrap();
            if trusted {
                store.remember(&source.fingerprint()).unwrap();
            }
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let source_pin = source.fingerprint();
            let server = std::thread::spawn(move || {
                DisplaySignal::accept(
                    listener.accept().unwrap().0,
                    &receiver,
                    &store,
                    |_, config, offer| {
                        assert_eq!(config.mode, Mode::Mirror);
                        assert_eq!(offer, "offer");
                        if revoke_during_offer {
                            store.forget(&source_pin)?;
                        }
                        Ok("answer".repeat(2048))
                    },
                )
            });
            let config = Config {
                mode: Mode::Mirror,
                width: 1280,
                height: 720,
                fps: 60,
                bitrate: 10_000_000,
                retina: false,
                auto_size: false,
                codec: VideoCodec::H264,
                separate_cursor: false,
            };
            let result = DisplaySignal::connect(
                TcpStream::connect(address).unwrap(),
                &source,
                &pin,
                &config,
                "offer",
            );
            let accepted = trusted && correct_pin && !revoke_during_offer;
            assert_eq!(result.is_ok(), accepted);
            assert_eq!(server.join().unwrap().is_ok(), accepted);
        }
    }
}
