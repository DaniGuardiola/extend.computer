//! WebSocket-to-TCP bridge. The existing pinned Noise engine owns encryption.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
#[derive(Clone)]
pub struct RelayCredentials {
    pub server: String,
    pub device: String,
    pub token: String,
}
#[derive(Default)]
pub struct Relay {
    credentials: Mutex<Option<RelayCredentials>>,
    online: AtomicBool,
    availability: Mutex<Option<bool>>,
    connections: AtomicUsize,
}
struct ConnectionGuard(Arc<Relay>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Relay {
    fn reserve(self: &Arc<Self>) -> Result<ConnectionGuard> {
        self.connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| anyhow::anyhow!("Too many internet connections"))?;
        Ok(ConnectionGuard(self.clone()))
    }
    pub fn configure(&self, credentials: Option<RelayCredentials>) {
        let mut current = self.credentials.lock().unwrap();
        let changed = current.as_ref().map(|c| (&c.server, &c.device, &c.token))
            != credentials
                .as_ref()
                .map(|c| (&c.server, &c.device, &c.token));
        if changed {
            self.online.store(false, Ordering::SeqCst);
            *self.availability.lock().unwrap() = None;
        }
        *current = credentials;
    }
    fn current(&self, credentials: &RelayCredentials) -> bool {
        self.credentials.lock().unwrap().as_ref().is_some_and(|c| {
            c.server == credentials.server
                && c.device == credentials.device
                && c.token == credentials.token
        })
    }
    pub fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }
    // Cache a negative result too: discovery and LAN retries must not poll the backend.
    // Each user-initiated account connection refreshes this once.
    pub fn available(&self, refresh: bool) -> bool {
        let Some(credentials) = self.credentials.lock().unwrap().clone() else {
            return false;
        };
        let mut availability = self.availability.lock().unwrap();
        if !refresh {
            if let Some(enabled) = *availability {
                return enabled;
            }
        }
        let enabled = (|| -> Result<bool> {
            let mut url = Url::parse(&credentials.server)?;
            url.set_path("/v1/relay/status");
            url.set_query(None);
            let response = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()?
                .get(url)
                .send()?
                .error_for_status()?;
            let response: Value = serde_json::from_reader(response.take(16 * 1024))?;
            Ok(response["relay_enabled"] == true)
        })()
        .unwrap_or(false);
        *availability = Some(enabled);
        enabled
    }
    pub fn tunnel(self: &Arc<Self>, peer_device: &str, kind: &str) -> Result<TcpStream> {
        ensure!(
            self.available(false),
            std::io::Error::new(std::io::ErrorKind::NotConnected, "Device is not reachable")
        );
        let guard = self.reserve()?;
        let credentials = self
            .credentials
            .lock()
            .unwrap()
            .clone()
            .context("Sign in to connect over the internet.")?;
        let mut ws = match connect(
            &credentials,
            "tunnel",
            &[("peer", peer_device), ("kind", kind)],
        ) {
            Ok(ws) => ws,
            Err(error) => {
                if relay_disabled(&error) {
                    *self.availability.lock().unwrap() = Some(false);
                }
                if matches!(error.downcast_ref::<tungstenite::Error>(),
                    Some(tungstenite::Error::Http(response)) if matches!(response.status().as_u16(), 401 | 403))
                {
                    bail!("Device access expired");
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "Device is not reachable",
                )
                .into());
            }
        };
        ready(&mut ws)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let local = TcpStream::connect(listener.local_addr()?)?;
        let (bridge, _) = listener.accept()?;
        let relay = self.clone();
        std::thread::spawn(move || {
            let _guard = guard;
            let _ = pump(ws, bridge, || relay.current(&credentials));
        });
        Ok(local)
    }
    pub fn start(self: &Arc<Self>, desktop: &Arc<crate::runtime::Desktop>) {
        let weak = Arc::downgrade(self);
        let app = Arc::downgrade(desktop);
        std::thread::spawn(move || loop {
            let (Some(relay), Some(desktop)) = (weak.upgrade(), app.upgrade()) else {
                break;
            };
            if desktop.is_closing() {
                break;
            }
            let credentials = relay.credentials.lock().unwrap().clone();
            if let Some(credentials) = credentials.filter(|_| relay.available(false)) {
                let result = (|| -> Result<()> {
                    let mut ws = connect(&credentials, "connect", &[])?;
                    ready(&mut ws)?;
                    relay.online.store(true, Ordering::SeqCst);
                    let mut ping = Instant::now();
                    while relay.current(&credentials) && !desktop.is_closing() {
                        if ping.elapsed() > Duration::from_secs(20) {
                            ws.send(Message::Text("ping".into()))?;
                            ping = Instant::now();
                        }
                        match ws.read() {
                            Ok(Message::Text(text)) if text != "pong" => {
                                let offer: Value = serde_json::from_str(&text)?;
                                if offer["type"] != "offer" {
                                    continue;
                                }
                                let channel =
                                    offer["channel"].as_str().context("Invalid relay offer")?;
                                let peer = offer["peer"].as_str().context("Invalid relay peer")?;
                                let kind = offer["kind"].as_str().context("Invalid relay kind")?;
                                if channel.len() != 64
                                    || hex::decode(channel).is_err()
                                    || !desktop.account_peer_allowed(peer)?
                                {
                                    continue;
                                }
                                // Only these two internal listeners may be bridged, never arbitrary ports.
                                let port = desktop.relay_listener_port(kind)?;
                                let Ok(guard) = relay.reserve() else { continue };
                                let relay = relay.clone();
                                let credentials = credentials.clone();
                                let channel = channel.to_owned();
                                std::thread::spawn(move || {
                                    let _guard = guard;
                                    let result = (|| -> Result<()> {
                                        let socket = TcpStream::connect(("127.0.0.1", port))?;
                                        let mut ws = connect(
                                            &credentials,
                                            "accept",
                                            &[("channel", &channel)],
                                        )?;
                                        ready(&mut ws)?;
                                        pump(ws, socket, || relay.current(&credentials))
                                    })();
                                    if let Err(error) = result {
                                        eprintln!("Relay connection ended: {error}");
                                    }
                                });
                            }
                            Ok(Message::Close(_)) => break,
                            Ok(_) => {}
                            Err(error) if idle(&error) => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                    let _ = ws.close(None);
                    Ok(())
                })();
                relay.online.store(false, Ordering::SeqCst);
                if let Err(error) = result {
                    if relay_disabled(&error) {
                        *relay.availability.lock().unwrap() = Some(false);
                    } else {
                        eprintln!("Account relay disconnected: {error}");
                    }
                }
            }
            drop(desktop);
            drop(relay);
            std::thread::sleep(Duration::from_secs(3));
        });
    }
}
fn relay_disabled(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<tungstenite::Error>(),
        Some(tungstenite::Error::Http(response))
        if response.status().as_u16() == 503
            && (response.headers().get("x-extend-relay-enabled").is_some_and(|v| v == "false")
                || response.body().as_ref().and_then(|body| serde_json::from_slice::<Value>(body).ok())
                    .is_some_and(|body| body["code"] == "relay_disabled")))
}

fn connect(
    credentials: &RelayCredentials,
    action: &str,
    parameters: &[(&str, &str)],
) -> Result<Socket> {
    let mut url = Url::parse(&credentials.server)?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| anyhow::anyhow!("Invalid relay server"))?;
    url.set_path(&format!("/v1/relay/{action}"));
    url.query_pairs_mut()
        .append_pair("device", &credentials.device)
        .extend_pairs(parameters.iter().copied());
    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", credentials.token).parse()?,
    );
    // Resolve/connect explicitly to bound network stalls; tungstenite's helper has no connect timeout.
    use std::net::ToSocketAddrs;
    let addresses = (
        url.host_str().context("Missing relay host")?,
        url.port_or_known_default().context("Missing relay port")?,
    )
        .to_socket_addrs()?;
    let mut connection = None;
    for address in addresses.take(8) {
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            connection = Some(socket);
            break;
        }
    }
    let socket = connection.context("Could not reach account relay")?;
    // Input arrives as small frames. Nagle buffering here delays the encrypted
    // bridge even though its loopback socket already has TCP_NODELAY enabled.
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(8)))?;
    socket.set_write_timeout(Some(Duration::from_secs(8)))?;
    let (mut ws, _) = tungstenite::client_tls_with_config(
        request,
        socket,
        Some(
            tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(65536))
                .max_frame_size(Some(65536)),
        ),
        None,
    ).map_err(|error| match error {
        tungstenite::HandshakeError::Failure(error) => anyhow::Error::new(error),
        error => anyhow::anyhow!(error),
    })?;
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => {
            s.set_read_timeout(Some(Duration::from_millis(50)))?;
            s.set_write_timeout(Some(Duration::from_secs(2)))?;
        }
        MaybeTlsStream::Rustls(s) => {
            s.sock.set_read_timeout(Some(Duration::from_millis(50)))?;
            s.sock.set_write_timeout(Some(Duration::from_secs(2)))?;
        }
        _ => bail!("Unsupported relay TLS transport"),
    }
    Ok(ws)
}
fn ready(ws: &mut Socket) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
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
            Err(error) if idle(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Other device did not answer. Check Allow connections there.")
}
fn idle(error: &tungstenite::Error) -> bool {
    matches!(error,tungstenite::Error::Io(e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut))
}
fn pump(mut ws: Socket, mut tcp: TcpStream, active: impl Fn() -> bool) -> Result<()> {
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_millis(2)))?,
        MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(Some(Duration::from_millis(2)))?,
        _ => bail!("Unsupported relay TLS transport"),
    }
    tcp.set_read_timeout(Some(Duration::from_millis(1)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(2)))?;
    tcp.set_nodelay(true)?;
    let result = (|| -> Result<()> {
        let mut buffer = [0; 16 * 1024];
        while active() {
            match tcp.read(&mut buffer) {
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
                    tcp.write_all(&data)?;
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(e) if idle(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    })();
    let _ = tcp.shutdown(Shutdown::Both);
    let _ = ws.close(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{identity::Identity, session::query_presence};
    use serde_json::json;
    #[test]
    fn relay_connection_disables_nagle_and_forwards_small_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(tcp).unwrap();
            assert_eq!(ws.read().unwrap().into_data().as_ref(), &[1, 2, 3]);
            ws.send(Message::Binary(vec![4, 5].into())).unwrap();
        });
        let mut ws = connect(
            &RelayCredentials {
                server: format!("http://{address}"),
                device: "test-device".into(),
                token: "test-token".into(),
            },
            "tunnel",
            &[],
        )
        .unwrap();
        match ws.get_ref() {
            MaybeTlsStream::Plain(socket) => assert!(socket.nodelay().unwrap()),
            _ => panic!("Expected local plaintext test transport"),
        }
        ws.send(Message::Binary(vec![1, 2, 3].into())).unwrap();
        assert_eq!(ws.read().unwrap().into_data().as_ref(), &[4, 5]);
        server.join().unwrap();
    }
    #[test]
    fn disabled_capability_is_cached_and_tunnel_never_opens_a_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let count = tcp.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /v1/relay/status "));
            let body = r#"{"relay_enabled":false}"#;
            write!(tcp, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            drop(tcp);
            listener.set_nonblocking(true).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
        let relay = Arc::new(Relay::default());
        relay.configure(Some(RelayCredentials {
            server: format!("http://{address}"),
            device: "test".into(),
            token: "secret".into(),
        }));
        assert!(!relay.available(true));
        for _ in 0..5 {
            assert!(!relay.available(false));
            let error = relay.tunnel("peer", "control").unwrap_err();
            assert!(extend_computer_agent::error::is_connection_failure(&error));
            assert!(!error.to_string().to_lowercase().contains("relay"));
        }
        server.join().unwrap();
    }
    #[test]
    fn disabling_relay_after_capability_check_stops_further_attempts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                ("200 OK", r#"{"relay_enabled":true}"#),
                (
                    "503 Service Unavailable",
                    r#"{"code":"relay_disabled","relay_enabled":false}"#,
                ),
            ] {
                let (mut tcp, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                tcp.read(&mut request).unwrap();
                let policy = if status.starts_with("503") { "X-Extend-Relay-Enabled: false\r\n" } else { "" };
                // Deliberately split headers from body: WebSocket rejection may
                // arrive before the JSON payload has been read.
                write!(tcp, "HTTP/1.1 {status}\r\n{policy}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                std::thread::sleep(Duration::from_millis(30));
                let _ = tcp.write_all(body.as_bytes());
            }
            listener.set_nonblocking(true).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
        let relay = Arc::new(Relay::default());
        relay.configure(Some(RelayCredentials {
            server: format!("http://{address}"),
            device: "test".into(),
            token: "secret".into(),
        }));
        assert!(relay.available(true));
        let error = relay.tunnel("peer", "control").unwrap_err();
        assert!(extend_computer_agent::error::is_connection_failure(&error));
        assert!(!relay.available(false));
        assert!(relay.tunnel("peer", "control").is_err());
        server.join().unwrap();
    }
    fn api(server: &str, path: &str, token: Option<&str>, value: Value) -> (u16, Value) {
        let client = reqwest::blocking::Client::new();
        let mut request = client
            .post(format!("{server}/v1{path}"))
            .header("Origin", server)
            .header("X-Extend-Client", "desktop")
            .json(&value);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().unwrap();
        let status = response.status().as_u16();
        (status, response.json().unwrap_or(Value::Null))
    }
    fn register(server: &str, account: &str, identity: &Identity, name: &str) -> RelayCredentials {
        let (status, device) = api(
            server,
            "/devices",
            Some(account),
            json!({"public_key":hex::encode(identity.public()),"name":name,"platform":"macos"}),
        );
        assert_eq!(status, 200, "{device}");
        let credentials = RelayCredentials {
            server: server.into(),
            device: device["id"].as_str().unwrap().into(),
            token: device["device_token"].as_str().unwrap().into(),
        };
        let path = format!("/devices/{}/proof", credentials.device);
        let (status, challenge) = api(
            server,
            &(path.clone() + "/options"),
            Some(&credentials.token),
            json!({}),
        );
        assert_eq!(status, 200, "{challenge}");
        let wrong = Identity::generate()
            .account_proof(
                challenge["server_key"].as_str().unwrap(),
                challenge["challenge"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(
            api(
                server,
                &(path.clone() + "/verify"),
                Some(&credentials.token),
                json!({"challenge":challenge["challenge"],"proof":wrong})
            )
            .0,
            401
        );
        let (status, challenge) = api(
            server,
            &(path.clone() + "/options"),
            Some(&credentials.token),
            json!({}),
        );
        assert_eq!(status, 200);
        let proof = identity
            .account_proof(
                challenge["server_key"].as_str().unwrap(),
                challenge["challenge"].as_str().unwrap(),
            )
            .unwrap();
        let body = json!({"challenge":challenge["challenge"],"proof":proof});
        assert_eq!(
            api(
                server,
                &(path.clone() + "/verify"),
                Some(&credentials.token),
                body.clone()
            )
            .0,
            204
        );
        assert_eq!(
            api(server, &(path + "/verify"), Some(&credentials.token), body).0,
            401
        );
        credentials
    }
    #[test]
    #[ignore = "Requires isolated local account server; performs real encrypted relay and revocation checks"]
    fn encrypted_presence_cross_account_isolation_and_revocation() {
        let server = std::env::var("EXTEND_RELAY_TEST_URL").expect("EXTEND_RELAY_TEST_URL");
        let fixtures = std::env::var("EXTEND_RELAY_TEST_SESSIONS_FILE").ok().map(|path| serde_json::from_slice::<Vec<Value>>(&std::fs::read(path).unwrap()).unwrap());
        assert!(server.starts_with("http://localhost:") || (server == "https://extend.computer" && fixtures.is_some()));
        let unique = hex::encode(rand::random::<[u8; 16]>());
        let password = "isolated-relay-test-password";
        let (status, signup) = if let Some(fixtures) = &fixtures { (201, fixtures[0].clone()) } else {
            api(&server, "/auth/signup", None, json!({"email":format!("relay-{unique}@example.invalid"),"password":password}))
        };
        assert_eq!(status, 201, "{signup}");
        let account = signup["token"].as_str().unwrap();
        if let Ok(path) = std::env::var("EXTEND_RELAY_TEST_CLEANUP") {
            std::fs::write(path, signup["account"]["id"].as_str().unwrap()).unwrap();
        }
        let sender = Identity::generate();
        let receiver = Identity::generate();
        let first = register(&server, account, &sender, "Relay sender");
        let second = register(&server, account, &receiver, "Relay receiver");
        let first_relay = Arc::new(Relay::default());
        first_relay.configure(Some(first.clone()));
        let receiver_temp = tempfile::tempdir().unwrap();
        let desktop = crate::runtime::Desktop::new(
            receiver_temp.path().into(),
            receiver_temp.path().join("unused-helper"),
        )
        .unwrap();
        desktop.set_test_identity(receiver);
        desktop.sync_account_peers("test-account",&[json!({"id":first.device,"fingerprint":sender.fingerprint(),"name":"sender","key_verified":true})]).unwrap();
        desktop.start_presence().unwrap();
        desktop.account_relay.configure(Some(second.clone()));
        desktop.account_relay.start(&desktop);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !desktop.account_relay.online() {
            assert!(Instant::now() < deadline, "Receiver relay never connected");
            std::thread::sleep(Duration::from_millis(50));
        }
        let receiver_fp = desktop.local_device_info().unwrap().identity;
        let socket = first_relay.tunnel(&second.device, "presence").unwrap();
        let presence = query_presence(socket, &sender, &receiver_fp)
            .unwrap()
            .unwrap();
        assert!(
            !presence.receiving,
            "Account membership must not enable control"
        );
        let socket = first_relay.tunnel(&second.device, "presence").unwrap();
        assert!(
            query_presence(socket, &sender, &"f".repeat(64)).is_err(),
            "Wrong identity must fail closed"
        );
        #[cfg(unix)]
        let control_sender = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&desktop.helper, r#"#!/bin/sh
case "$1" in
status) echo 'listen=true post=true wifi=true';;
inject-control) echo 'READY 1728 1117'; while IFS= read -r line; do echo OK; done;;
capture-control-*) echo 'READY 1512 982'; while IFS= read -r line; do :; done;;
*) exit 1;;
esac
"#).unwrap();
            std::fs::set_permissions(&desktop.helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port(); drop(listener);
            desktop.receive_test_port(port).unwrap();
            let app = crate::runtime::Desktop::new(receiver_temp.path().join("sender"), desktop.helper.clone()).unwrap();
            app.set_test_identity(sender);
            app.sync_account_peers("test-account", &[json!({"id":second.device,"fingerprint":receiver_fp,"name":"receiver","key_verified":true})]).unwrap();
            app.account_relay.configure(Some(first.clone()));
            app.connect(receiver_fp.clone(), crate::peers::Device { name:"Receiver".into(), address:"0.0.0.0:48177".into(), edge:"left".into() }).unwrap();
            let deadline=Instant::now()+Duration::from_secs(15);
            while !app.snapshot().unwrap().session.is_some_and(|session|serde_json::to_value(session).unwrap()["phase"]=="connected") { assert!(Instant::now()<deadline,"Internet control never connected");std::thread::sleep(Duration::from_millis(50)); }
            assert!(desktop.approvals.current().is_none());
            app
        };
        let (status, other) = if let Some(fixtures) = &fixtures { (201, fixtures[1].clone()) } else {
            api(&server, "/auth/signup", None, json!({"email":format!("relay-other-{unique}@example.invalid"),"password":password}))
        };
        assert_eq!(status, 201);
        if let Ok(path) = std::env::var("EXTEND_RELAY_TEST_CLEANUP") {
            use std::fs::OpenOptions;
            let mut file = OpenOptions::new().append(true).open(path).unwrap();
            writeln!(file, "\n{}", other["account"]["id"].as_str().unwrap()).unwrap();
        }
        let outsider = register(
            &server,
            other["token"].as_str().unwrap(),
            &Identity::generate(),
            "Other account",
        );
        assert!(connect(
            &outsider,
            "tunnel",
            &[("peer", &second.device), ("kind", "presence")]
        )
        .is_err());
        let (status, _) = api(&server, "/auth/logout", Some(account), json!({}));
        assert_eq!(status, 204);
        assert!(connect(&first, "connect", &[]).is_err());
        let deadline = Instant::now() + Duration::from_secs(40);
        while desktop.account_relay.online() {
            assert!(
                Instant::now() < deadline,
                "Revoked receiver relay stayed connected"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = api(&server, "/auth/logout", other["token"].as_str(), json!({}));
        #[cfg(unix)] {
            let deadline=Instant::now()+Duration::from_secs(10);
            while desktop.snapshot().unwrap().session.is_some() { assert!(Instant::now()<deadline,"Revoked internet control stayed active");std::thread::sleep(Duration::from_millis(50)); }
            control_sender.shutdown();
        }
        desktop.shutdown();
    }
}
