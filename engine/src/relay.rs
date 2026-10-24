//! Account relay transports opaque pinned engine bytes, never grants input access.
//! The platform policy owns current membership and the existing receiving gate.
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;
use zeroize::Zeroizing;
type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub server: String,
    pub scope: String,
    pub device: String,
    pub token: Zeroizing<String>,
}
pub trait Policy: Send + Sync + 'static {
    /// Fresh account membership is required even when manual trust still exists.
    fn active(&self) -> bool;
    fn allowed(&self, peer: &str, kind: &str) -> bool;
    /// Only a fixed platform-owned presence/control endpoint may be returned.
    fn incoming(&self, peer: &str, kind: &str) -> Result<TcpStream>;
    fn receiving(&self, _kind: &str) -> bool {
        true
    }
}
/// Per-operation cancellation closes tracked sockets without canceling another
/// connection to the same peer. DNS waits remain bounded by the resolver budget.
#[derive(Default)]
pub struct Cancellation {
    retired: AtomicBool,
    sockets: Mutex<Vec<TcpStream>>,
}
impl Cancellation {
    pub fn cancel(&self) {
        self.retired.store(true, Ordering::SeqCst);
        for socket in self.sockets.lock().unwrap().iter() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    fn active(&self) -> bool {
        !self.retired.load(Ordering::SeqCst)
    }
    fn track(&self, socket: &TcpStream) -> Result<()> {
        let mut sockets = self.sockets.lock().unwrap();
        ensure!(self.active(), "Internet connection canceled");
        ensure!(sockets.len() < 3, "Too many relay sockets");
        sockets.push(socket.try_clone()?);
        Ok(())
    }
}
struct Connection {
    peer: String,
    kind: String,
    incoming: bool,
    cancellation: Cancellation,
    operation: Option<Arc<Cancellation>>,
}
impl Connection {
    fn retire(&self) {
        self.cancellation.cancel();
    }
    fn track(&self, socket: &TcpStream) -> Result<()> {
        self.cancellation.track(socket)?;
        if let Some(operation) = &self.operation {
            operation.track(socket)?;
        }
        Ok(())
    }
}
struct State {
    credentials: Option<Credentials>,
    generation: u64,
    availability: Option<bool>,
    connections: BTreeMap<u64, Arc<Connection>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            credentials: None,
            generation: 0,
            availability: None,
            connections: BTreeMap::new(),
        }
    }
}
#[derive(Default)]
pub struct Relay {
    state: Mutex<State>,
    ids: AtomicU64,
    online: AtomicBool,
    started: AtomicBool,
    checking: AtomicBool,
}
struct Guard {
    relay: Arc<Relay>,
    id: u64,
    connection: Arc<Connection>,
    generation: u64,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.connection.retire();
        self.relay
            .state
            .lock()
            .unwrap()
            .connections
            .remove(&self.id);
    }
}
impl Guard {
    fn active(&self, policy: &dyn Policy) -> bool {
        let current = self.relay.state.lock().unwrap().generation == self.generation;
        self.connection.cancellation.active()
            && self
                .connection
                .operation
                .as_ref()
                .is_none_or(|op| op.active())
            && current
            && policy.active()
            && (!self.connection.incoming || policy.receiving(&self.connection.kind))
            && (self.connection.kind == "registration"
                || policy.allowed(&self.connection.peer, &self.connection.kind))
    }
}
impl Relay {
    pub fn configure(&self, credentials: Option<Credentials>) -> Result<()> {
        if let Some(c) = &credentials {
            origin(&c.server)?;
            ensure!(
                hex(&c.device)
                    && hex(&c.token)
                    && !c.scope.is_empty()
                    && c.scope.len() <= 2048
                    && !c.scope.chars().any(char::is_control),
                "Invalid relay credentials"
            );
        }
        let mut state = self.state.lock().unwrap();
        if state.credentials != credentials {
            state.generation = state.generation.wrapping_add(1);
            state.availability = None;
            self.online.store(false, Ordering::SeqCst);
            for connection in state.connections.values() {
                connection.retire();
            }
            state.credentials = credentials;
        }
        Ok(())
    }
    pub fn cancel_peer(&self, peer: &str) {
        for connection in self
            .state
            .lock()
            .unwrap()
            .connections
            .values()
            .filter(|c| c.peer == peer)
        {
            connection.retire();
        }
    }
    pub fn cancel_kind(&self, kind: &str) {
        for connection in self
            .state
            .lock()
            .unwrap()
            .connections
            .values()
            .filter(|c| c.incoming && c.kind == kind)
        {
            connection.retire();
        }
    }
    pub fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }
    pub fn available(&self, refresh: bool) -> bool {
        let (credentials, generation) = {
            let state = self.state.lock().unwrap();
            let Some(c) = state.credentials.clone() else {
                return false;
            };
            if !refresh {
                if let Some(enabled) = state.availability {
                    return enabled;
                }
            }
            (c, state.generation)
        };
        if self
            .checking
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        struct Checking<'a>(&'a AtomicBool);
        impl Drop for Checking<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _checking = Checking(&self.checking);
        let enabled = (|| -> Result<bool> {
            let mut url = origin(&credentials.server)?;
            url.set_path("/v1/relay/status");
            let response = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()?
                .get(url)
                .send()?
                .error_for_status()?;
            let value: Value = serde_json::from_reader(response.take(16 * 1024))?;
            Ok(value["relay_enabled"] == true)
        })()
        .unwrap_or(false);
        let mut state = self.state.lock().unwrap();
        if state.generation != generation {
            return false;
        }
        state.availability = Some(enabled);
        enabled
    }
    fn reserve(
        self: &Arc<Self>,
        peer: &str,
        kind: &str,
        incoming: bool,
        expected: Option<u64>,
        operation: Option<Arc<Cancellation>>,
    ) -> Result<(Guard, Credentials)> {
        let mut state = self.state.lock().unwrap();
        ensure!(
            expected.is_none_or(|expected| expected == state.generation),
            "Relay registration changed"
        );
        let credentials = state
            .credentials
            .clone()
            .context("Sign in to connect over the internet.")?;
        let registration = kind == "registration";
        ensure!(
            state
                .connections
                .values()
                .filter(|c| (c.kind == "registration") == registration)
                .count()
                < if registration { 1 } else { 4 },
            "Too many internet connections"
        );
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection {
            peer: peer.into(),
            kind: kind.into(),
            incoming,
            cancellation: Cancellation::default(),
            operation,
        });
        state.connections.insert(id, connection.clone());
        Ok((
            Guard {
                relay: self.clone(),
                id,
                connection,
                generation: state.generation,
            },
            credentials,
        ))
    }
    pub fn tunnel(
        self: &Arc<Self>,
        peer: &str,
        device: &str,
        kind: &str,
        policy: Arc<dyn Policy>,
        refresh: bool,
        operation: Arc<Cancellation>,
    ) -> Result<TcpStream> {
        let deadline =
            Instant::now() + Duration::from_secs(if kind == "presence" { 8 } else { 30 });
        ensure!(
            hex(peer) && hex(device) && matches!(kind, "presence" | "control"),
            "Invalid relay target"
        );
        ensure!(
            policy.active() && policy.allowed(peer, kind),
            "Account device access expired"
        );
        ensure!(self.available(refresh), "Internet relay is unavailable");
        let (guard, credentials) = self.reserve(peer, kind, false, None, Some(operation))?;
        let mut ws = connect(
            &credentials,
            "tunnel",
            &[("peer", device), ("kind", kind)],
            &guard,
            &*policy,
            deadline,
        )?;
        ready(&mut ws, || guard.active(&*policy), deadline)?;
        let (local, bridge) = socket_pair()?;
        guard.connection.track(&bridge)?;
        std::thread::Builder::new()
            .name("extend-relay-pump".into())
            .spawn(move || {
                let _ = pump(ws, bridge, || guard.active(&*policy));
            })?;
        Ok(local)
    }
    /// One registration actor per process. Offers never supply a local address/port.
    pub fn start(self: &Arc<Self>, policy: Arc<dyn Policy>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            let Some(relay) = weak.upgrade() else {
                break;
            };
            if policy.active() && relay.available(false) {
                let generation = relay.state.lock().unwrap().generation;
                let result = (|| -> Result<()> {
                    let (guard, credentials) =
                        relay.reserve("", "registration", false, None, None)?;
                    let deadline = Instant::now() + Duration::from_secs(30);
                    let mut ws = connect(&credentials, "connect", &[], &guard, &*policy, deadline)?;
                    ready(&mut ws, || guard.active(&*policy), deadline)?;
                    ensure!(guard.active(&*policy), "Internet connection canceled");
                    relay.online.store(true, Ordering::SeqCst);
                    let mut ping = Instant::now();
                    while guard.active(&*policy) {
                        if ping.elapsed() > Duration::from_secs(20) {
                            ws.send(Message::Text("ping".into()))?;
                            ping = Instant::now();
                        }
                        match ws.read() {
                            Ok(Message::Text(text)) if text != "pong" => {
                                let Ok(offer) = serde_json::from_str::<Offer>(&text) else {
                                    continue;
                                };
                                if offer.kind != "presence" && offer.kind != "control" {
                                    continue;
                                }
                                if offer.message_type != "offer"
                                    || !hex(&offer.channel)
                                    || !hex(&offer.peer)
                                    || !policy.allowed(&offer.peer, &offer.kind)
                                {
                                    continue;
                                }
                                if !guard.active(&*policy) {
                                    break;
                                }
                                let Ok((bridge, credentials)) = relay.reserve(
                                    &offer.peer,
                                    &offer.kind,
                                    true,
                                    Some(guard.generation),
                                    None,
                                ) else {
                                    continue;
                                };
                                let policy = policy.clone();
                                let _ = std::thread::Builder::new()
                                    .name("extend-relay-accept".into())
                                    .spawn(move || {
                                        let _ = (|| -> Result<()> {
                                            ensure!(
                                                bridge.active(&*policy),
                                                "Internet connection canceled"
                                            );
                                            let socket =
                                                policy.incoming(&offer.peer, &offer.kind)?;
                                            bridge.connection.track(&socket)?;
                                            let deadline = Instant::now() + Duration::from_secs(30);
                                            let mut ws = connect(
                                                &credentials,
                                                "accept",
                                                &[("channel", &offer.channel)],
                                                &bridge,
                                                &*policy,
                                                deadline,
                                            )?;
                                            ready(&mut ws, || bridge.active(&*policy), deadline)?;
                                            pump(ws, socket, || bridge.active(&*policy))
                                        })();
                                    });
                            }
                            Ok(Message::Close(_)) => break,
                            Ok(_) => {}
                            Err(error) if idle(&error) => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                    Ok(())
                })();
                // Never log WebSocket errors/URLs: credentials must stay native.
                if result.as_ref().err().is_some_and(disabled) {
                    let mut state = relay.state.lock().unwrap();
                    if state.generation == generation {
                        state.availability = Some(false);
                    }
                }
                relay.online.store(false, Ordering::SeqCst);
            }
            drop(relay);
            std::thread::sleep(Duration::from_secs(3));
        });
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Offer {
    #[serde(rename = "type")]
    message_type: String,
    channel: String,
    peer: String,
    kind: String,
}
fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn origin(raw: &str) -> Result<Url> {
    let url = Url::parse(raw)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/"),
        "Invalid relay server"
    );
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "Relay requires HTTPS"
    );
    ensure!(url.port() != Some(0), "Invalid relay port");
    Ok(url)
}
fn disabled(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<tungstenite::Error>(), Some(tungstenite::Error::Http(response)) if response.status().as_u16() == 503 && response.headers().get("x-extend-relay-enabled").is_some_and(|v| v == "false"))
}
pub fn socket_pair() -> Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let local = TcpStream::connect(listener.local_addr()?)?;
    let (bridge, _) = listener.accept()?;
    local.set_nodelay(true)?;
    bridge.set_nodelay(true)?;
    Ok((local, bridge))
}
// A single bounded resolver worker prevents timed-out DNS requests accumulating threads.
struct Dns {
    host: String,
    port: u16,
    until: Instant,
    reply: mpsc::SyncSender<Vec<SocketAddr>>,
}
fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    static RESOLVER: OnceLock<mpsc::SyncSender<Dns>> = OnceLock::new();
    let resolver = RESOLVER.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel::<Dns>(4);
        std::thread::spawn(move || {
            while let Ok(request) = receive.recv() {
                if Instant::now() >= request.until {
                    continue;
                }
                let addresses = (request.host.as_str(), request.port)
                    .to_socket_addrs()
                    .map(|a| a.take(8).collect())
                    .unwrap_or_default();
                let _ = request.reply.try_send(addresses);
            }
        });
        send
    });
    let (reply, receive) = mpsc::sync_channel(1);
    resolver
        .try_send(Dns {
            host: host.into(),
            port,
            until: Instant::now() + Duration::from_secs(2),
            reply,
        })
        .context("Relay DNS is busy")?;
    let addresses = receive
        .recv_timeout(Duration::from_secs(2))
        .context("Relay DNS timed out")?;
    ensure!(!addresses.is_empty(), "Relay host unavailable");
    Ok(addresses)
}
fn connect(
    c: &Credentials,
    action: &str,
    parameters: &[(&str, &str)],
    guard: &Guard,
    policy: &dyn Policy,
    deadline: Instant,
) -> Result<Socket> {
    ensure!(guard.active(policy), "Internet connection canceled");
    let deadline = deadline.min(Instant::now() + Duration::from_secs(20));
    // A trickling TLS/HTTP peer cannot extend a per-read timeout indefinitely.
    let (finished, waiting) = mpsc::channel();
    let connection = guard.connection.clone();
    std::thread::spawn(move || {
        if waiting
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_err()
        {
            connection.retire();
        }
    });
    struct Deadline(mpsc::Sender<()>);
    impl Drop for Deadline {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let _deadline = Deadline(finished);
    let mut url = origin(&c.server)?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| anyhow::anyhow!("Invalid relay server"))?;
    url.set_path(&format!("/v1/relay/{action}"));
    url.query_pairs_mut()
        .append_pair("device", &c.device)
        .extend_pairs(parameters.iter().copied());
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {}", &*c.token).parse()?);
    let addresses = resolve(
        url.host_str().context("Missing relay host")?,
        url.port_or_known_default().context("Missing relay port")?,
    )?;
    let mut socket = None;
    for address in addresses {
        ensure!(
            guard.active(policy) && Instant::now() < deadline,
            "Internet connection canceled"
        );
        if let Ok(tcp) = TcpStream::connect_timeout(
            &address,
            Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
        ) {
            socket = Some(tcp);
            break;
        }
    }
    let socket = socket.context("Could not reach account relay")?;
    guard.connection.track(&socket)?;
    socket.set_nodelay(true)?;
    let timeout = Duration::from_secs(8).min(deadline.saturating_duration_since(Instant::now()));
    ensure!(!timeout.is_zero(), "Relay connection timed out");
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    let (mut ws, _) = tungstenite::client_tls_with_config(
        request,
        socket,
        Some(
            tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(65536))
                .max_frame_size(Some(65536)),
        ),
        None,
    )
    .map_err(|error| match error {
        tungstenite::HandshakeError::Failure(e) => anyhow::Error::new(e),
        _ => anyhow::anyhow!("Relay handshake failed"),
    })?;
    ensure!(
        guard.active(policy) && Instant::now() < deadline,
        "Internet connection canceled"
    );
    tcp(&mut ws)?.set_read_timeout(Some(Duration::from_millis(50)))?;
    tcp(&mut ws)?.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(ws)
}
fn tcp(ws: &mut Socket) -> Result<&mut TcpStream> {
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => Ok(s),
        MaybeTlsStream::Rustls(s) => Ok(&mut s.sock),
        _ => bail!("Unsupported relay TLS transport"),
    }
}
fn idle(error: &tungstenite::Error) -> bool {
    matches!(error,tungstenite::Error::Io(e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut))
}
fn ready(ws: &mut Socket, active: impl Fn() -> bool, deadline: Instant) -> Result<()> {
    while active() && Instant::now() < deadline {
        match ws.read() {
            Ok(Message::Text(text))
                if serde_json::from_str::<Value>(&text)
                    .ok()
                    .is_some_and(|v| v["type"] == "ready") =>
            {
                return Ok(())
            }
            Ok(Message::Close(_)) => bail!("Relay connection closed"),
            Ok(_) => {}
            Err(e) if idle(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    bail!("Other device did not answer. Check Allow connections there.")
}
fn pump(mut ws: Socket, mut local: TcpStream, active: impl Fn() -> bool) -> Result<()> {
    tcp(&mut ws)?.set_read_timeout(Some(Duration::from_millis(2)))?;
    local.set_read_timeout(Some(Duration::from_millis(1)))?;
    local.set_write_timeout(Some(Duration::from_secs(2)))?;
    let result = (|| -> Result<()> {
        let mut buffer = [0; 16 * 1024];
        let mut checked = Instant::now() - Duration::from_millis(100);
        loop {
            if checked.elapsed() >= Duration::from_millis(100) {
                if !active() {
                    break;
                }
                checked = Instant::now();
            }
            match local.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => ws.send(Message::Binary(buffer[..n].to_vec().into()))?,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.into()),
            }
            match ws.read() {
                Ok(Message::Binary(data)) => {
                    ensure!(data.len() <= 65536, "Relay frame too large");
                    local.write_all(&data)?;
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(e) if idle(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    })();
    let _ = local.shutdown(Shutdown::Both);
    let _ = ws.close(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Allowed;
    impl Policy for Allowed {
        fn active(&self) -> bool {
            true
        }
        fn allowed(&self, _: &str, _: &str) -> bool {
            true
        }
        fn incoming(&self, _: &str, _: &str) -> Result<TcpStream> {
            bail!("Not used")
        }
    }
    fn credentials() -> Credentials {
        Credentials {
            server: "http://127.0.0.1:1234".into(),
            scope: "local/account".into(),
            device: "1".repeat(64),
            token: Zeroizing::new("2".repeat(64)),
        }
    }
    #[test]
    fn origins_credentials_and_offer_fields_fail_closed() {
        for origin in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
            "https://example.com#fragment",
        ] {
            assert!(super::origin(origin).is_err());
        }
        for origin in [
            "https://example.com",
            "http://127.0.0.1:1234",
            "http://[::1]:1234",
        ] {
            assert!(super::origin(origin).is_ok());
        }
        let relay = Relay::default();
        let mut c = credentials();
        c.token = Zeroizing::new("not a token".into());
        assert!(relay.configure(Some(c)).is_err());
        assert!(serde_json::from_str::<Offer>(
            r#"{"type":"offer","channel":"1","peer":"2","kind":"control","port":42}"#
        )
        .is_err());
    }
    #[test]
    fn bounded_channels_cancel_peer_direction_and_generation_without_stale_online() {
        let relay = Arc::new(Relay::default());
        relay.configure(Some(credentials())).unwrap();
        let mut guards = Vec::new();
        for _ in 0..4 {
            guards.push(relay.reserve("a", "control", false, None, None).unwrap().0);
        }
        assert!(relay.reserve("b", "presence", false, None, None).is_err());
        let registration = relay
            .reserve("", "registration", false, None, None)
            .unwrap()
            .0;
        assert!(relay
            .reserve("", "registration", false, None, None)
            .is_err());
        relay.cancel_kind("control");
        assert!(
            guards.iter().all(|g| g.active(&Allowed)),
            "Receiving Stop must not cancel outgoing input"
        );
        drop(guards.pop());
        let incoming = relay.reserve("b", "control", true, None, None).unwrap().0;
        let (mut client, server) = socket_pair().unwrap();
        incoming.connection.track(&server).unwrap();
        relay.cancel_kind("control");
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
        assert!(!incoming.active(&Allowed));
        assert!(guards.iter().all(|g| g.active(&Allowed)));
        relay.cancel_peer("a");
        assert!(guards.iter().all(|g| !g.active(&Allowed)));
        let mut next = credentials();
        next.scope = "different/account".into();
        relay.configure(Some(next)).unwrap();
        assert!(!registration.active(&Allowed));
        assert!(!relay.online());
    }
    #[test]
    fn late_offer_cannot_reserve_against_new_credentials_and_operation_cancel_closes_its_socket() {
        let relay = Arc::new(Relay::default());
        relay.configure(Some(credentials())).unwrap();
        let old = relay
            .reserve("", "registration", false, None, None)
            .unwrap()
            .0;
        let mut new = credentials();
        new.scope = "second/account".into();
        relay.configure(Some(new)).unwrap();
        assert!(relay
            .reserve("a", "control", true, Some(old.generation), None)
            .is_err());
        let operation = Arc::new(Cancellation::default());
        let guard = relay
            .reserve("a", "control", false, None, Some(operation.clone()))
            .unwrap()
            .0;
        let (mut client, server) = socket_pair().unwrap();
        guard.connection.track(&server).unwrap();
        operation.cancel();
        assert!(!guard.active(&Allowed));
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
        let unrelated = relay.reserve("a", "control", false, None, None).unwrap().0;
        assert!(unrelated.active(&Allowed));
    }
    #[test]
    fn ready_cancellation_interrupts_silent_peer_instead_of_waiting_thirty_seconds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(tcp).unwrap();
            let _ = ws.read();
        });
        let relay = Arc::new(Relay::default());
        let mut c = credentials();
        c.server = format!("http://{address}");
        relay.configure(Some(c.clone())).unwrap();
        let (guard, _) = relay.reserve("a", "control", false, None, None).unwrap();
        let mut ws = connect(
            &c,
            "tunnel",
            &[],
            &guard,
            &Allowed,
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap();
        relay.cancel_peer("a");
        let start = Instant::now();
        assert!(ready(
            &mut ws,
            || guard.active(&Allowed),
            Instant::now() + Duration::from_secs(30)
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_millis(250));
        server.join().unwrap();
    }
}
