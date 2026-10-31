//! Display negotiation is encrypted and pinned before any media resources start.
use super::*;
use crate::protocol::Offers;
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
    /// Sent instead of an Answer, only when both ends negotiated
    /// DISPLAY_DECLINE: older receivers reject unknown signals.
    Declined {
        reason: DeclineReason,
    },
    Close,
}
/// Why a receiver turned an offer down before starting any media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// The receiver can't present this mode or codec.
    Unsupported,
    /// A reason added by a newer build.
    #[serde(other)]
    Other,
}
/// A display offer turned down on purpose before any media started. The
/// receiver's answer closure returns it with `local` set; the source gets it
/// back with `local` unset when the receiver could say why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declined {
    pub reason: DeclineReason,
    /// This device made the decision.
    pub local: bool,
}
impl Declined {
    pub fn local(reason: DeclineReason) -> Self {
        Self {
            reason,
            local: true,
        }
    }
}
impl std::fmt::Display for Declined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match (self.local, self.reason) {
            (true, _) => "This device can’t show this display yet.",
            (false, DeclineReason::Unsupported) => {
                "The other device can’t show this display yet. Update extend.computer on that device."
            }
            (false, DeclineReason::Other) => "The other device declined this display.",
        })
    }
}
impl std::error::Error for Declined {}
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
        stream: TcpStream,
        identity: &Identity,
        peer: &str,
        config: &Config,
        enable_settings: bool,
        make_offer: impl FnOnce(DisplayFeatures) -> Result<String>,
    ) -> Result<(Self, Config, String, DisplayFeatures)> {
        Self::connect_for_peer(
            stream,
            identity,
            peer,
            config.clone(),
            enable_settings,
            |_, features, _| make_offer(features),
        )
    }
    /// Like `connect_with_settings_offer`, but `make_offer` may also adapt the
    /// configuration (codec, pointer) to the roles this peer offered in the
    /// pinned handshake, before any media is built and before the Offer.
    pub fn connect_for_peer(
        mut stream: TcpStream,
        identity: &Identity,
        peer: &str,
        mut config: Config,
        enable_settings: bool,
        make_offer: impl FnOnce(&mut Config, DisplayFeatures, &Offers) -> Result<String>,
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
        let offers = channel.remote_offers().clone();
        let offer = make_offer(&mut config, features, &offers)?;
        config.validate()?;
        channel.display_signaling();
        let offered_settings =
            enable_settings && channel.supports_capability(crate::protocol::RECEIVER_SETTINGS);
        channel.send(&Signal::Offer {
            config: config.clone(),
            sdp: offer,
            receiver_settings: offered_settings,
        })?;
        let (selected, sdp, receiver_settings) = match channel.receive()? {
            Signal::Answer {
                config,
                sdp,
                receiver_settings,
            } => (config, sdp, receiver_settings),
            Signal::Declined { reason }
                if channel.supports_capability(crate::protocol::DISPLAY_DECLINE) =>
            {
                bail!(Declined {
                    reason,
                    local: false
                })
            }
            _ => bail!(EngineError::RequestRejected),
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
        let sdp = match answer(&peer, &mut config, &sdp, features) {
            Ok(sdp) => sdp,
            Err(error) => {
                // Tell a source that can read it why, then end the exchange.
                if let Some(declined) = error.downcast_ref::<Declined>() {
                    if channel.supports_capability(crate::protocol::DISPLAY_DECLINE) {
                        let _ = channel.send(&Signal::Declined {
                            reason: declined.reason,
                        });
                    }
                }
                return Err(error);
            }
        };
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
    /// A receiver that sends `hello` as its Hello, then runs `then`, as an
    /// older or other-platform build would.
    fn scripted_receiver(
        receiver: Identity,
        source_peer: String,
        hello: serde_json::Value,
        then: impl FnOnce(&mut Channel) -> Result<()> + Send + 'static,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<Result<()>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || -> Result<()> {
            let mut stream = listener.accept()?.0;
            wire::configure(&stream)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut greeting = [0; 9];
            wire::read_exact_until(&mut stream, &mut greeting, deadline)?;
            ensure!(
                &greeting[..8] == MAGIC && greeting[8] == b'D',
                "Invalid greeting"
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
                        "Bad handshake"
                    );
                }
            }
            ensure!(
                fingerprint(noise.get_remote_static().unwrap()) == source_peer,
                "Invalid source pin"
            );
            let mut channel = Channel::new(stream, noise.into_transport_mode()?);
            channel.send(&hello)?;
            let _: serde_json::Value = channel.receive()?;
            channel.display_signaling();
            then(&mut channel)
        });
        (address, server)
    }
    fn current_capabilities() -> serde_json::Value {
        serde_json::json!([
            crate::protocol::CONTROL,
            crate::protocol::DISPLAY_INPUT,
            crate::protocol::RECEIVER_SETTINGS,
            crate::protocol::DISPLAY_DECLINE
        ])
    }
    #[test]
    fn source_adapts_the_offer_to_roles_from_the_pinned_handshake() {
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
        // A viewer that decodes only H.264 and can't draw a separate pointer.
        let hello = serde_json::json!({"min_protocol":1,"max_protocol":1,"app_version":"1",
            "capabilities":current_capabilities(),
            "offers":[crate::protocol::offer::DISPLAY_VIEWER]});
        let (address, server) =
            scripted_receiver(receiver, source.fingerprint(), hello, |channel| {
                let Signal::Offer { config, sdp, .. } = channel.receive()? else {
                    bail!("Expected an offer")
                };
                ensure!(sdp == "chosen-offer", "Unexpected offer");
                ensure!(
                    config.codec == VideoCodec::H264 && !config.separate_cursor,
                    "Offer kept choices the viewer can't show"
                );
                channel.send(&Signal::Answer {
                    config,
                    sdp: "answer".into(),
                    receiver_settings: false,
                })
            });
        let mut preferred = input_config();
        preferred.codec = VideoCodec::Hevc;
        let (_, selected, _, _) = DisplaySignal::connect_for_peer(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            preferred,
            false,
            |config, _, offers| {
                assert_eq!(
                    offers.tokens(),
                    Some(&[crate::protocol::offer::DISPLAY_VIEWER.to_string()][..])
                );
                if !offers.allows(crate::protocol::offer::DISPLAY_DECODE_HEVC) {
                    config.codec = VideoCodec::H264;
                }
                config.separate_cursor &=
                    offers.allows(crate::protocol::offer::DISPLAY_CURSOR_OVERLAY);
                Ok("chosen-offer".into())
            },
        )
        .unwrap();
        assert_eq!(selected.codec, VideoCodec::H264);
        assert!(!selected.separate_cursor);
        server.join().unwrap().unwrap();
    }
    #[test]
    fn receiver_declines_with_a_reason_only_when_both_ends_negotiated_it() {
        // Current builds on both ends: the source learns why.
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
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
                |_, _, _, _| Err(Declined::local(DeclineReason::Unsupported).into()),
            )
            .map(|_| ())
        });
        let error = DisplaySignal::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            &input_config(),
            "offer",
        )
        .err()
        .unwrap();
        assert_eq!(
            error.downcast_ref::<Declined>(),
            Some(&Declined {
                reason: DeclineReason::Unsupported,
                local: false
            })
        );
        let local = server.join().unwrap().unwrap_err();
        assert_eq!(
            local.downcast_ref::<Declined>(),
            Some(&Declined::local(DeclineReason::Unsupported))
        );
        assert!(!error.to_string().is_empty() && local.to_string() != error.to_string());

        // A source without the capability (v0.3.1 rejects unknown signals)
        // sees the channel end, never a Declined message.
        let source = Identity::generate();
        let receiver = Identity::generate();
        let receiver_peer = receiver.fingerprint();
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
                |_, _, _, _| Err(Declined::local(DeclineReason::Unsupported).into()),
            )
            .map(|_| ())
        });
        let mut stream = TcpStream::connect(address).unwrap();
        wire::configure(&stream).unwrap();
        stream.write_all(MAGIC).unwrap();
        stream.write_all(b"D").unwrap();
        let mut noise = Builder::new(BASE.parse().unwrap())
            .local_private_key(source.secret())
            .unwrap()
            .prologue(b"extend.computer/v1/display")
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut output = [0; wire::MAX_FRAME];
        let deadline = Instant::now() + Duration::from_secs(5);
        for turn in 0..3 {
            if turn == 1 {
                let bytes = wire::read_frame_until(&mut stream, deadline).unwrap();
                noise.read_message(&bytes, &mut output).unwrap();
            } else {
                let len = noise.write_message(&[], &mut output).unwrap();
                wire::write_frame(&mut stream, &output[..len]).unwrap();
            }
        }
        assert_eq!(
            fingerprint(noise.get_remote_static().unwrap()),
            receiver_peer
        );
        let mut channel = Channel::new(stream, noise.into_transport_mode().unwrap());
        let _: serde_json::Value = channel.receive().unwrap();
        channel
            .send(
                &serde_json::json!({"min_protocol":1,"max_protocol":1,"app_version":"0.3.1",
                "capabilities":[crate::protocol::CONTROL, crate::protocol::DISPLAY_INPUT]}),
            )
            .unwrap();
        channel.display_signaling();
        channel
            .send(&serde_json::json!({"type":"Offer","config":input_config(),"sdp":"offer"}))
            .unwrap();
        assert!(channel.receive::<serde_json::Value>().is_err());
        assert!(server
            .join()
            .unwrap()
            .unwrap_err()
            .downcast_ref::<Declined>()
            .is_some_and(|declined| declined.local));

        // A receiver that never negotiated it can't make the source accept one.
        let source = Identity::generate();
        let receiver = Identity::generate();
        let peer = receiver.fingerprint();
        let hello = serde_json::json!({"min_protocol":1,"max_protocol":1,"app_version":"1",
            "capabilities":[crate::protocol::CONTROL]});
        let (address, server) =
            scripted_receiver(receiver, source.fingerprint(), hello, |channel| {
                let _: serde_json::Value = channel.receive()?;
                channel.send(&Signal::Declined {
                    reason: DeclineReason::Unsupported,
                })
            });
        let error = DisplaySignal::connect(
            TcpStream::connect(address).unwrap(),
            &source,
            &peer,
            &input_config(),
            "offer",
        )
        .err()
        .unwrap();
        assert!(error.downcast_ref::<Declined>().is_none());
        assert_eq!(
            error.downcast_ref::<EngineError>(),
            Some(&EngineError::RequestRejected)
        );
        server.join().unwrap().unwrap();
    }
    #[test]
    fn decline_reasons_from_newer_builds_stay_readable() {
        let signal: Signal =
            serde_json::from_str(r#"{"type":"Declined","reason":"busy"}"#).unwrap();
        assert!(matches!(
            signal,
            Signal::Declined {
                reason: DeclineReason::Other
            }
        ));
        assert_eq!(
            serde_json::to_value(Signal::Declined {
                reason: DeclineReason::Unsupported
            })
            .unwrap(),
            serde_json::json!({"type":"Declined","reason":"unsupported"})
        );
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
