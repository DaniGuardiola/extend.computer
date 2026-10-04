mod presence;
use crate::error::EngineError;
use crate::{
    identity::{fingerprint, Identity},
    pairing::PairingWindow,
    trust::TrustStore,
    wire::{self, Channel},
};
use anyhow::{bail, ensure, Context, Result};
use hkdf::Hkdf;
pub use presence::{query_presence, serve_presence, Presence};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use snow::Builder;
use spake2::{Ed25519Group, Identity as PakeIdentity, Password, Spake2};
use std::{
    io::Write,
    net::TcpStream,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const BASE: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const PAIRED: &str = "Noise_XXpsk0_25519_ChaChaPoly_SHA256";
const MAGIC: &[u8; 8] = b"EXTEND05";

#[derive(Clone, Copy, Debug)]
pub enum Decision {
    Deny,
    Once,
    Remember,
}

impl Decision {
    fn persist(self, store: &TrustStore, peer: &str) -> Result<()> {
        match self {
            Self::Remember => store.remember(peer),
            _ => Ok(()),
        }
    }
}

/// Logical display-point dimensions disclosed after authenticated input startup.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySize {
    pub width: f64,
    pub height: f64,
}
impl DisplaySize {
    pub fn validate(self) -> Result<Self> {
        ensure!(
            [self.width, self.height]
                .iter()
                .all(|v| v.is_finite() && (16.0..=32768.0).contains(v)),
            "invalid display dimensions"
        );
        Ok(self)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Message {
    Ready,
    NotPaired,
    QueryPresence,
    Presence(Presence),
    QueryPairing,
    PairingConfirmed,
    ExchangeDeviceName {
        name: String,
    },
    DeviceName {
        name: String,
    },
    RequestUnpair,
    Unpaired,
    Denied,
    RequestProbe,
    Granted,
    Ping {
        sequence: u64,
    },
    Pong {
        sequence: u64,
    },
    RequestCursor,
    RequestInput,
    RequestControl {
        // Retained for compatibility with older endpoints; new receivers use pairing/account access.
        remembered_only: bool,
    },
    ControlGranted {
        display: DisplaySize,
    },
    InputGranted {
        display: DisplaySize,
    },
    Input {
        sequence: u64,
        event: crate::input::InputEvent,
    },
    CursorGranted {
        display: DisplaySize,
    },
    Cursor {
        sequence: u64,
        x: f64,
        y: f64,
    },
    CursorStream {
        sequence: u64,
        x: f64,
        y: f64,
    },
    Close,
}

fn pake(
    stream: &mut TcpStream,
    code: &str,
    initiator: bool,
    deadline: Instant,
) -> Result<(Zeroizing<[u8; 32]>, Vec<u8>)> {
    let password = Password::new(code.as_bytes());
    let a = PakeIdentity::new(b"extend.computer/v1/connector");
    let b = PakeIdentity::new(b"extend.computer/v1/listener");
    let (state, outbound) = if initiator {
        Spake2::<Ed25519Group>::start_a(&password, &a, &b)
    } else {
        Spake2::<Ed25519Group>::start_b(&password, &a, &b)
    };
    let inbound;
    if initiator {
        wire::write_frame(stream, &outbound)?;
        inbound = wire::read_frame_until(stream, deadline)?;
    } else {
        inbound = wire::read_frame_until(stream, deadline)?;
        wire::write_frame(stream, &outbound)?;
    }
    let mut prologue = b"extend.computer/v1/pair/probe".to_vec();
    for bytes in if initiator {
        [&outbound, &inbound]
    } else {
        [&inbound, &outbound]
    } {
        prologue.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        prologue.extend_from_slice(bytes);
    }
    let shared = Zeroizing::new(
        state
            .finish(&inbound)
            .map_err(|_| EngineError::PairingFailed)?,
    );
    let mut psk = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(&prologue), &shared)
        .expand(b"extend.computer/v1/noise-psk", psk.as_mut())
        .map_err(|_| anyhow::anyhow!("key derivation failed"))?;
    Ok((psk, prologue))
}

fn handshake(
    mut stream: TcpStream,
    identity: &Identity,
    code: Option<&str>,
    initiator: bool,
    deadline: Instant,
    context: &[u8],
    expected_peer: Option<&str>,
) -> Result<(Channel, String, Vec<u8>)> {
    let pairing = if let Some(code) = code {
        Some(pake(&mut stream, code, initiator, deadline)?)
    } else {
        None
    };
    let prologue = pairing
        .as_ref()
        .map(|(_, p)| p.as_slice())
        .unwrap_or(context);
    let mut builder = Builder::new(if pairing.is_some() { PAIRED } else { BASE }.parse()?)
        .local_private_key(identity.secret())?
        .prologue(prologue)?;
    if let Some((psk, _)) = &pairing {
        builder = builder.psk(0, psk)?;
    }
    let mut noise = if initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    let mut output = [0; wire::MAX_FRAME];
    for turn in 0..3 {
        if (turn % 2 == 0) == initiator {
            let len = noise.write_message(&[], &mut output)?;
            wire::write_frame(&mut stream, &output[..len])?;
        } else {
            let packet = wire::read_frame_until(&mut stream, deadline)?;
            ensure!(
                noise
                    .read_message(&packet, &mut output)
                    .context(if code.is_some() {
                        EngineError::PairingFailed
                    } else {
                        EngineError::AuthenticationFailed
                    })?
                    == 0,
                "unexpected handshake payload"
            );
        }
    }
    let peer = fingerprint(
        noise
            .get_remote_static()
            .ok_or_else(|| anyhow::anyhow!("missing peer identity"))?,
    );
    ensure!(peer != identity.fingerprint(), "self connection rejected");
    if let Some(expected) = expected_peer {
        ensure!(peer == expected, EngineError::PeerIdentityChanged);
    }
    let transcript = noise.get_handshake_hash().to_vec();
    let mut channel = Channel::new(stream, noise.into_transport_mode()?);
    crate::protocol::exchange(&mut channel)?;
    Ok((channel, peer, transcript))
}

/// Single diagnostic session. Caller must obtain local consent through the callback.
pub fn serve_connection(
    stream: TcpStream,
    identity: &Identity,
    store: &TrustStore,
    window: &mut Option<PairingWindow>,
    approve: impl FnMut(&str, bool) -> Decision,
) -> Result<()> {
    serve_connection_with_cursor(
        stream,
        identity,
        store,
        window,
        approve,
        &mut DisabledCursor,
    )
}

/// Platform resources start only for authenticated peers.
pub trait CursorSink {
    fn start_cursor(&mut self, peer: &str) -> Result<bool>;
    fn move_to(&mut self, x: f64, y: f64) -> Result<()>;
    fn start_control(&mut self, _peer: &str) -> Result<bool> {
        Ok(false)
    }
    fn start_input(&mut self, _peer: &str) -> Result<bool> {
        Ok(false)
    }
    fn input(&mut self, _event: &crate::input::InputEvent) -> Result<()> {
        bail!("input disabled")
    }
    fn heartbeat(&mut self) -> Result<()> {
        Ok(())
    }
    fn device_name(&self) -> Option<String> {
        None
    }
    fn peer_name(&mut self, _peer: &str, _name: &str) -> Result<()> {
        Ok(())
    }
    fn finish(&mut self) {}
    fn unpaired(&mut self, _peer: &str) -> Result<()> {
        Ok(())
    }
    /// Synthetic sinks may use a reference canvas; native sinks must override.
    fn display_size(&self) -> Result<DisplaySize> {
        Ok(DisplaySize {
            width: 1920.0,
            height: 1080.0,
        })
    }
}
// Always release per-session resources, including on authentication/protocol errors.
struct CursorSession<'a, S: CursorSink>(&'a mut S);
impl<S: CursorSink> Drop for CursorSession<'_, S> {
    fn drop(&mut self) {
        self.0.finish();
    }
}
struct DisabledCursor;
impl CursorSink for DisabledCursor {
    fn start_cursor(&mut self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
        bail!("cursor disabled")
    }
}

pub fn serve_connection_with_cursor(
    stream: TcpStream,
    identity: &Identity,
    store: &TrustStore,
    window: &mut Option<PairingWindow>,
    approve: impl FnMut(&str, bool) -> Decision,
    cursor: &mut impl CursorSink,
) -> Result<()> {
    serve_connection_with_verification(stream, identity, store, window, approve, cursor, |_, _| {
        false
    })
}

#[allow(clippy::too_many_arguments)]
pub fn serve_connection_with_verification(
    mut stream: TcpStream,
    identity: &Identity,
    store: &TrustStore,
    window: &mut Option<PairingWindow>,
    mut approve: impl FnMut(&str, bool) -> Decision,
    cursor: &mut impl CursorSink,
    mut verify: impl FnMut(&str, &[u8; 8]) -> bool,
) -> Result<()> {
    let cursor_session = CursorSession(cursor);
    let cursor = &mut *cursor_session.0;
    wire::configure(&stream)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut hello = [0; 9];
    wire::read_exact_until(&mut stream, &mut hello, deadline)?;
    ensure!(&hello[..8] == MAGIC, EngineError::ProtocolIncompatible);
    let claim = match hello[8] {
        b'P' | b'S' => Some(
            window
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("pairing closed"))?
                .claim()?,
        ),
        b'R' | b'U' => None,
        _ => bail!("unsupported mode"),
    };
    let (mut channel, peer, transcript) = handshake(
        stream,
        identity,
        claim
            .as_ref()
            .filter(|_| hello[8] == b'P')
            .map(|(s, _)| s.as_str()),
        false,
        deadline,
        match hello[8] {
            b'S' => b"extend.computer/v1/visual-pair",
            b'U' => b"extend.computer/v1/unpair",
            _ => b"extend.computer/v1/reconnect/probe",
        },
        None,
    )?;
    if hello[8] == b'U' {
        // An authenticated identity can remove only its own record, never a
        // caller-supplied fingerprint. Unknown peers are idempotent no-ops.
        channel.send(&Message::Ready)?;
        ensure!(
            matches!(channel.receive()?, Message::RequestUnpair),
            EngineError::RequestRejected
        );
        store.complete_unpair(&peer)?;
        cursor.unpaired(&peer)?;
        channel.send(&Message::Unpaired)?;
        return Ok(());
    }
    let known = match store.peer(&peer) {
        Ok(known) => known,
        Err(error) => {
            let _ = channel.send(&Message::Denied);
            return Err(error);
        }
    };
    if claim.is_none() && known.is_none() {
        channel.send(&Message::NotPaired)?;
        return Ok(());
    }
    if let Some((_, expires)) = &claim {
        ensure!(Instant::now() < *expires, "pairing expired");
    }
    if hello[8] == b'S' {
        crate::verification::verify(&mut channel, &transcript, false, |symbols| {
            verify(&peer, symbols)
                && claim
                    .as_ref()
                    .is_some_and(|(_, expires)| Instant::now() < *expires)
        })?;
        ensure!(!store.is_revoked(&peer)?, "device revoked");
        store.remember(&peer)?;
        return Ok(());
    }
    channel.consent_timeout()?;
    channel.send(&Message::Ready)?;
    match channel.receive()? {
        Message::QueryPairing if claim.is_none() => {
            channel.send(&Message::PairingConfirmed)?;
            return Ok(());
        }
        Message::ExchangeDeviceName { name } if claim.is_none() => {
            validate_device_name(&name)?;
            let local_name = cursor.device_name().ok_or(EngineError::RequestRejected)?;
            validate_device_name(&local_name)?;
            ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
            cursor.peer_name(&peer, &name)?;
            channel.send(&Message::DeviceName { name: local_name })?;
            return Ok(());
        }
        Message::RequestProbe => {}
        _ => bail!(EngineError::RequestRejected),
    }
    let decision = if known.is_some() {
        Decision::Once
    } else {
        approve(&peer, false)
    };
    if matches!(decision, Decision::Deny) {
        channel.send(&Message::Denied)?;
        bail!(EngineError::LocalConsentDenied);
    }
    if let Some((_, expires)) = &claim {
        ensure!(Instant::now() < *expires, "pairing expired during approval");
    }
    ensure!(!store.is_revoked(&peer)?, "device revoked");
    decision.persist(store, &peer)?;
    channel.send(&Message::Granted)?;
    channel.active_timeout()?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut next_sequence = 0;
    let mut cursor_deadline = None;
    let mut pending_cursor = 0;
    let mut input_granted = false;
    let mut session_control = false;
    let mut active_input = false;
    loop {
        let message: Message = channel.receive()?;
        ensure!(
            session_control || Instant::now() < deadline,
            "session expired"
        );
        if store.is_revoked(&peer)? || (active_input && store.peer(&peer)?.is_none()) {
            channel.send(&Message::Denied)?;
            bail!("pairing or account access removed");
        }
        let streamed = matches!(message, Message::CursorStream { .. });
        match message {
            Message::Ping { sequence } => {
                ensure!(sequence == next_sequence, "unexpected probe sequence");
                next_sequence += 1;
                if input_granted {
                    ensure!(
                        (session_control || cursor_deadline.is_some_and(|d| Instant::now() < d)),
                        "input grant expired"
                    );
                    cursor.heartbeat()?;
                }
                channel.send(&Message::Pong { sequence })?;
                pending_cursor = 0;
            }
            Message::RequestControl { .. } => {
                channel.require_capability(crate::protocol::CONTROL)?;
                ensure!(cursor_deadline.is_none(), "control already requested");
                ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
                if !cursor.start_control(&peer)? {
                    channel.send(&Message::Denied)?;
                    bail!(EngineError::RequestRejected);
                }
                ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
                let display = cursor.display_size()?.validate()?;
                active_input = true;
                cursor_deadline = Some(Instant::now()); // Marks a nonrenewable request; session scope uses heartbeat instead.
                input_granted = true;
                session_control = true;
                channel.send(&Message::ControlGranted { display })?;
            }
            Message::RequestInput => {
                channel.require_capability(crate::protocol::INPUT)?;
                ensure!(cursor_deadline.is_none(), "control already requested");
                ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
                ensure!(cursor.start_input(&peer)?, EngineError::RequestRejected);
                active_input = true;
                ensure!(!store.is_revoked(&peer)?, "device revoked");
                let display = cursor.display_size()?.validate()?;
                cursor_deadline = Some(Instant::now() + Duration::from_secs(30));
                input_granted = true;
                channel.send(&Message::InputGranted { display })?;
            }
            Message::Input { sequence, event } => {
                ensure!(
                    input_granted
                        && (session_control || cursor_deadline.is_some_and(|d| Instant::now() < d)),
                    "input grant absent or expired"
                );
                ensure!(sequence == next_sequence, "unexpected input sequence");
                ensure!(pending_cursor < 4, "input window exceeded");
                event.validate()?;
                cursor.input(&event)?;
                pending_cursor += 1;
                next_sequence += 1;
            }
            Message::RequestCursor => {
                channel.require_capability(crate::protocol::CURSOR)?;
                ensure!(cursor_deadline.is_none(), "cursor already requested");
                ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
                ensure!(cursor.start_cursor(&peer)?, EngineError::RequestRejected);
                active_input = true;
                ensure!(!store.is_revoked(&peer)?, "device revoked");
                let display = cursor.display_size()?.validate()?;
                cursor_deadline = Some(Instant::now() + Duration::from_secs(30));
                channel.send(&Message::CursorGranted { display })?;
            }
            Message::Cursor { sequence, x, y } | Message::CursorStream { sequence, x, y } => {
                if streamed {
                    ensure!(pending_cursor < 4, "cursor window exceeded");
                    pending_cursor += 1;
                }
                ensure!(
                    (session_control || cursor_deadline.is_some_and(|d| Instant::now() < d)),
                    "cursor grant absent or expired"
                );
                ensure!(sequence == next_sequence, "unexpected cursor sequence");
                ensure!(
                    x.is_finite()
                        && y.is_finite()
                        && (0.0..=1.0).contains(&x)
                        && (0.0..=1.0).contains(&y),
                    "invalid cursor coordinates"
                );
                cursor.move_to(x, y)?;
                next_sequence += 1;
                if !streamed {
                    channel.send(&Message::Pong { sequence })?;
                }
            }
            Message::Close => return Ok(()),
            _ => bail!("message not authorized for this session"),
        }
    }
}

/// Read-only status over the existing pinned reconnect handshake. Failure is
/// never equivalent to NotPaired; this function does not change local trust.
pub fn pairing_status(mut stream: TcpStream, identity: &Identity, expected: &str) -> Result<bool> {
    wire::configure(&stream)?;
    stream.write_all(MAGIC)?;
    stream.write_all(b"R")?;
    let (mut channel, peer, _) = handshake(
        stream,
        identity,
        None,
        true,
        Instant::now() + Duration::from_secs(10),
        b"extend.computer/v1/reconnect/probe",
        Some(expected),
    )?;
    ensure!(peer == expected, EngineError::PeerIdentityChanged);
    match channel.receive()? {
        Message::NotPaired => return Ok(false),
        Message::Ready => {}
        _ => bail!(EngineError::RequestRejected),
    }
    channel.send(&Message::QueryPairing)?;
    ensure!(
        matches!(channel.receive()?, Message::PairingConfirmed),
        EngineError::RequestRejected
    );
    Ok(true)
}

/// Names are display metadata exchanged only after a pinned, trusted handshake.
pub fn exchange_device_name(
    mut stream: TcpStream,
    identity: &Identity,
    expected: &str,
    name: &str,
) -> Result<String> {
    validate_device_name(name)?;
    wire::configure(&stream)?;
    stream.write_all(MAGIC)?;
    stream.write_all(b"R")?;
    let (mut channel, peer, _) = handshake(
        stream,
        identity,
        None,
        true,
        Instant::now() + Duration::from_secs(10),
        b"extend.computer/v1/reconnect/probe",
        Some(expected),
    )?;
    ensure!(peer == expected, EngineError::PeerIdentityChanged);
    ensure!(
        matches!(channel.receive()?, Message::Ready),
        EngineError::RequestRejected
    );
    channel.send(&Message::ExchangeDeviceName { name: name.into() })?;
    let Message::DeviceName { name } = channel.receive()? else {
        bail!(EngineError::RequestRejected)
    };
    validate_device_name(&name)?;
    Ok(name)
}
pub fn validate_device_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty() && name.len() <= 100 && !name.chars().any(char::is_control),
        EngineError::RequestRejected
    );
    Ok(())
}

/// Notify an already selected, pinned identity after local access is removed.
pub fn notify_unpair(stream: TcpStream, identity: &Identity, expected: &str) -> Result<()> {
    notify_unpair_if(stream, identity, expected, || true)
}
pub fn notify_unpair_if(
    mut stream: TcpStream,
    identity: &Identity,
    expected: &str,
    still_unpaired: impl FnOnce() -> bool,
) -> Result<()> {
    wire::configure(&stream)?;
    stream.write_all(MAGIC)?;
    stream.write_all(b"U")?;
    let (mut channel, peer, _) = handshake(
        stream,
        identity,
        None,
        true,
        Instant::now() + Duration::from_secs(10),
        b"extend.computer/v1/unpair",
        Some(expected),
    )?;
    ensure!(peer == expected, EngineError::PeerIdentityChanged);
    ensure!(
        matches!(channel.receive()?, Message::Ready),
        EngineError::RequestRejected
    );
    ensure!(still_unpaired(), "pairing changed during notification");
    channel.send(&Message::RequestUnpair)?;
    ensure!(
        matches!(channel.receive()?, Message::Unpaired),
        EngineError::RequestRejected
    );
    Ok(())
}

/// Pair only: never enables diagnostic or input control on this connection.
pub fn pair_visually(
    mut stream: TcpStream,
    identity: &Identity,
    store: &TrustStore,
    mut approve: impl FnMut(&str, &[u8; 8]) -> bool,
) -> Result<String> {
    wire::configure(&stream)?;
    stream.write_all(MAGIC)?;
    stream.write_all(b"S")?;
    let (mut channel, peer, transcript) = handshake(
        stream,
        identity,
        None,
        true,
        Instant::now() + Duration::from_secs(10),
        b"extend.computer/v1/visual-pair",
        None,
    )?;
    ensure!(!store.is_revoked(&peer)?, "device revoked");
    crate::verification::verify(&mut channel, &transcript, true, |symbols| {
        approve(&peer, symbols)
    })?;
    ensure!(!store.is_revoked(&peer)?, "device revoked");
    store.remember(&peer)?;
    Ok(peer)
}

pub struct Client {
    channel: Channel,
    address: std::net::SocketAddr,
    peer: String,
    sequence: u64,
    store: TrustStore,
}

impl Client {
    /// Reconnect requires a previously remembered, pinned fingerprint. No trust-on-first-use.
    pub fn connect(
        mut stream: TcpStream,
        identity: &Identity,
        store: &TrustStore,
        code: Option<&str>,
        expected_peer: Option<&str>,
        mut approve: impl FnMut(&str, bool) -> Decision,
    ) -> Result<Self> {
        let address = stream.peer_addr()?;
        if code.is_none() {
            let expected =
                expected_peer.ok_or_else(|| anyhow::anyhow!("reconnect requires a pinned peer"))?;
            ensure!(store.peer(expected)?.is_some(), "unknown device");
        }
        wire::configure(&stream)?;
        stream.write_all(MAGIC)?;
        stream.write_all(if code.is_some() { b"P" } else { b"R" })?;
        let (mut channel, peer, _) = handshake(
            stream,
            identity,
            code,
            true,
            Instant::now() + Duration::from_secs(10),
            b"extend.computer/v1/reconnect/probe",
            expected_peer,
        )?;
        if let Some(expected) = expected_peer {
            ensure!(peer == expected, EngineError::PeerIdentityChanged);
        }
        let known = store.peer(&peer)?;
        channel.consent_timeout()?;
        match channel.receive()? {
            Message::Ready => {}
            Message::NotPaired if code.is_none() => bail!(EngineError::PeerUnpaired),
            _ => bail!(EngineError::RequestRejected),
        }
        let decision = if known.is_some() {
            Decision::Once
        } else {
            approve(&peer, false)
        };
        ensure!(
            !matches!(decision, Decision::Deny),
            EngineError::LocalConsentDenied
        );
        channel.send(&Message::RequestProbe)?;
        match channel.receive()? {
            Message::Granted => {}
            Message::Denied => bail!(EngineError::RemoteConsentDenied),
            _ => bail!(EngineError::RequestRejected),
        }
        ensure!(!store.is_revoked(&peer)?, "device revoked");
        decision.persist(store, &peer)?;
        channel.active_timeout()?;
        Ok(Self {
            address,
            channel,
            peer,
            sequence: 0,
            store: store.clone(),
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }
    pub fn peer(&self) -> &str {
        &self.peer
    }
    pub fn probe(&mut self) -> Result<Duration> {
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        let start = Instant::now();
        self.channel.send(&Message::Ping {
            sequence: self.sequence,
        })?;
        match self.channel.receive()? {
            Message::Pong { sequence } if sequence == self.sequence => {
                self.sequence += 1;
                Ok(start.elapsed())
            }
            _ => bail!("invalid probe response"),
        }
    }
    pub fn request_control(&mut self) -> Result<DisplaySize> {
        self.channel.require_capability(crate::protocol::CONTROL)?;
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        self.channel.send(&Message::RequestControl {
            remembered_only: false,
        })?;
        self.channel.consent_timeout()?;
        let display = match self.channel.receive()? {
            Message::ControlGranted { display } => display,
            Message::Denied => bail!(EngineError::RemoteConsentDenied),
            _ => bail!(EngineError::RequestRejected),
        };
        self.channel.active_timeout()?;
        display.validate()
    }
    pub fn request_input(&mut self) -> Result<DisplaySize> {
        self.channel.require_capability(crate::protocol::INPUT)?;
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        self.channel.send(&Message::RequestInput)?;
        self.channel.consent_timeout()?;
        let display = match self.channel.receive()? {
            Message::InputGranted { display } => display,
            Message::Denied => bail!(EngineError::RemoteConsentDenied),
            _ => bail!(EngineError::RequestRejected),
        };
        self.channel.active_timeout()?;
        display.validate()
    }
    pub fn queue_input(&mut self, event: crate::input::InputEvent) -> Result<()> {
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        event.validate()?;
        self.channel.send(&Message::Input {
            sequence: self.sequence,
            event,
        })?;
        self.sequence += 1;
        Ok(())
    }
    pub fn request_cursor(&mut self) -> Result<DisplaySize> {
        self.channel.require_capability(crate::protocol::CURSOR)?;
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        self.channel.send(&Message::RequestCursor)?;
        self.channel.consent_timeout()?;
        let display = match self.channel.receive()? {
            Message::CursorGranted { display } => display,
            Message::Denied => bail!(EngineError::RemoteConsentDenied),
            _ => bail!(EngineError::RequestRejected),
        };
        let display = display.validate()?;
        self.channel.active_timeout()?;
        Ok(display)
    }
    pub fn move_cursor(&mut self, x: f64, y: f64) -> Result<()> {
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        ensure!(
            x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            "invalid cursor coordinates"
        );
        self.channel.send(&Message::Cursor {
            sequence: self.sequence,
            x,
            y,
        })?;
        ensure!(
            matches!(self.channel.receive()?, Message::Pong { sequence } if sequence == self.sequence),
            "invalid cursor acknowledgment"
        );
        self.sequence += 1;
        Ok(())
    }
    /// Send within a four-position window. `probe` acknowledges all preceding positions.
    /// Callers must probe before a fifth queued position; the receiver enforces this bound.
    pub fn queue_cursor(&mut self, x: f64, y: f64) -> Result<()> {
        ensure!(!self.store.is_revoked(&self.peer)?, "device revoked");
        ensure!(
            x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            "invalid cursor coordinates"
        );
        self.channel.send(&Message::CursorStream {
            sequence: self.sequence,
            x,
            y,
        })?;
        self.sequence += 1;
        Ok(())
    }
    pub fn close(mut self) -> Result<()> {
        self.channel.send(&Message::Close)
    }
}
