//! Real loopback HTTP regression for the contract used by desktop and Android.
//! Identity::account_proof is the native implementation, not a copied test hash.
use extend_computer_agent::identity::Identity;
use extend_computer_server::{router, Config, Server};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use std::{net::SocketAddr, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

struct HttpServer {
    _directory: tempfile::TempDir,
    origin: String,
    client: Client,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<std::io::Result<()>>>,
}
impl HttpServer {
    async fn start() -> Self {
        Self::start_with_relay(false).await
    }
    async fn start_with_relay(relay_enabled: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = Server::open(
            &directory.path().join("account.sqlite3"),
            Config {
                signup_enabled: true,
                relay_enabled,
                ..Config::default()
            },
        )
        .unwrap();
        let (shutdown, stopped) = oneshot::channel();
        let worker = tokio::spawn(async move {
            axum::serve(
                listener,
                router(server).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
        });
        Self {
            _directory: directory,
            origin,
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .pool_max_idle_per_host(0)
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            shutdown: Some(shutdown),
            worker: Some(worker),
        }
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        payload: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let mut request = self
            .client
            .request(method.clone(), format!("{}{path}", self.origin))
            // Android reuses the existing desktop/native API contract.
            .header("X-Extend-Client", "desktop")
            .header("Origin", &self.origin);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(payload) = payload {
            request = request.json(&payload);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), expected, "{method} {path}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = response.bytes().await.unwrap();
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        }
    }
    async fn signup(&self, email: &str) -> String {
        let session = self
            .request(
                Method::POST,
                "/v1/auth/signup",
                None,
                Some(json!({"email":email,"password":"disposable HTTP test password"})),
                StatusCode::CREATED,
            )
            .await;
        session["token"].as_str().unwrap().to_owned()
    }
    async fn register(
        &self,
        session: &str,
        identity: &Identity,
        platform: &str,
        name: &str,
    ) -> Device {
        let result = self
            .request(
                Method::POST,
                "/v1/devices",
                Some(session),
                Some(json!({"public_key":hex::encode(identity.public()),"platform":platform,"name":name})),
                StatusCode::OK,
            )
            .await;
        assert_eq!(result["fingerprint"], identity.fingerprint());
        Device {
            id: result["id"].as_str().unwrap().to_owned(),
            token: result["device_token"].as_str().unwrap().to_owned(),
        }
    }
    async fn options(&self, device: &Device) -> Value {
        self.request(
            Method::POST,
            &format!("/v1/devices/{}/proof/options", device.id),
            Some(&device.token),
            Some(json!({})),
            StatusCode::OK,
        )
        .await
    }
    async fn verify(&self, device: &Device, proof: Value, expected: StatusCode) {
        self.request(
            Method::POST,
            &format!("/v1/devices/{}/proof/verify", device.id),
            Some(&device.token),
            Some(proof),
            expected,
        )
        .await;
    }
    async fn directory(&self, session: &str) -> Vec<Value> {
        self.request(
            Method::GET,
            "/v1/devices",
            Some(session),
            None,
            StatusCode::OK,
        )
        .await["devices"]
            .as_array()
            .unwrap()
            .clone()
    }
    async fn finish(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.worker.take().unwrap())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}
struct Device {
    id: String,
    token: String,
}
fn proof(identity: &Identity, options: &Value) -> Value {
    json!({
        "challenge":options["challenge"],
        "proof":identity.account_proof(options["server_key"].as_str().unwrap(),
            options["challenge"].as_str().unwrap()).unwrap()
    })
}
fn entry<'a>(devices: &'a [Value], device: &Device) -> &'a Value {
    devices.iter().find(|row| row["id"] == device.id).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn android_native_identity_registers_proves_lists_heartbeats_and_revokes_over_http() {
    let http = HttpServer::start().await;
    let alice = http.signup("android-http@example.invalid").await;
    let bob = http.signup("other-account@example.invalid").await;
    let android_identity = Identity::generate();
    let desktop_identity = Identity::generate();
    let android = http
        .register(&alice, &android_identity, "android", "Android tablet")
        .await;
    let desktop = http
        .register(&alice, &desktop_identity, "macos", "Desktop Mac")
        .await;
    let initial = http.directory(&alice).await;
    let tablet = entry(&initial, &android);
    assert_eq!(tablet["platform"], "android");
    assert_eq!(tablet["name"], "Android tablet");
    assert_eq!(tablet["fingerprint"], android_identity.fingerprint());
    assert_eq!(tablet["key_verified"], false);
    assert_eq!(tablet["online"], false);
    assert!(tablet["last_seen"].is_null());
    assert!(http.directory(&bob).await.is_empty());

    // Device credentials never gain account-wide listing/removal privileges.
    http.request(
        Method::GET,
        "/v1/devices",
        Some(&android.token),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    http.request(
        Method::DELETE,
        &format!("/v1/devices/{}", desktop.id),
        Some(&android.token),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/heartbeat", desktop.id),
        Some(&android.token),
        Some(json!({})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/proof/options", android.id),
        Some(&alice),
        Some(json!({})),
        StatusCode::UNAUTHORIZED,
    )
    .await;

    let options = http.options(&android).await;
    let wrong_identity = Identity::generate();
    http.verify(
        &android,
        proof(&wrong_identity, &options),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    // A failed proof consumes its challenge; even the correct key cannot replay it.
    http.verify(
        &android,
        proof(&android_identity, &options),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_eq!(
        entry(&http.directory(&alice).await, &android)["key_verified"],
        false
    );

    let old_options = http.options(&android).await;
    let fresh_options = http.options(&android).await;
    assert!(old_options["challenge"] != fresh_options["challenge"]);
    http.verify(
        &android,
        proof(&android_identity, &old_options),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    // The stale attempt consumes the credential's pending challenge as well.
    http.verify(
        &android,
        proof(&android_identity, &fresh_options),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let options = http.options(&android).await;
    let accepted = proof(&android_identity, &options);
    http.verify(&android, accepted.clone(), StatusCode::NO_CONTENT)
        .await;
    http.verify(&android, accepted, StatusCode::UNAUTHORIZED)
        .await;
    let options = http.options(&desktop).await;
    http.verify(
        &desktop,
        proof(&desktop_identity, &options),
        StatusCode::NO_CONTENT,
    )
    .await;
    let verified = http.directory(&alice).await;
    assert_eq!(entry(&verified, &android)["key_verified"], true);
    assert_eq!(entry(&verified, &desktop)["key_verified"], true);
    assert_eq!(entry(&verified, &android)["online"], false);

    for device in [&android, &desktop] {
        http.request(
            Method::POST,
            &format!("/v1/devices/{}/heartbeat", device.id),
            Some(&device.token),
            Some(json!({})),
            StatusCode::NO_CONTENT,
        )
        .await;
    }
    let online = http.directory(&alice).await;
    assert_eq!(online.len(), 2);
    for device in [&android, &desktop] {
        let row = entry(&online, device);
        assert_eq!(row["online"], true);
        assert!(row["last_seen"].as_i64().is_some());
        assert!(row.get("device_token").is_none());
        assert!(row.get("public_key").is_none());
    }

    // Re-registering the same stable identity keeps its ID but rotates the
    // credential and requires a fresh proof/heartbeat, as on desktop rename.
    let renamed = http
        .register(&alice, &android_identity, "android", "Renamed tablet")
        .await;
    assert_eq!(renamed.id, android.id);
    assert!(renamed.token != android.token);
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/heartbeat", android.id),
        Some(&android.token),
        Some(json!({})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let reset = http.directory(&alice).await;
    assert_eq!(entry(&reset, &renamed)["name"], "Renamed tablet");
    assert_eq!(entry(&reset, &renamed)["key_verified"], false);
    assert_eq!(entry(&reset, &renamed)["online"], false);
    let options = http.options(&renamed).await;
    http.verify(
        &renamed,
        proof(&android_identity, &options),
        StatusCode::NO_CONTENT,
    )
    .await;
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/heartbeat", renamed.id),
        Some(&renamed.token),
        Some(json!({})),
        StatusCode::NO_CONTENT,
    )
    .await;

    // Account isolation, removal, and credential revocation are HTTP-visible.
    http.request(
        Method::DELETE,
        &format!("/v1/devices/{}", renamed.id),
        Some(&bob),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(
        entry(&http.directory(&alice).await, &renamed)["online"],
        true
    );
    http.request(
        Method::DELETE,
        &format!("/v1/devices/{}", renamed.id),
        Some(&alice),
        None,
        StatusCode::NO_CONTENT,
    )
    .await;
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/heartbeat", renamed.id),
        Some(&renamed.token),
        Some(json!({})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    http.request(
        Method::POST,
        &format!("/v1/devices/{}/proof/options", renamed.id),
        Some(&renamed.token),
        Some(json!({})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let remaining = http.directory(&alice).await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], desktop.id);
    assert_eq!(remaining[0]["online"], true);
    http.finish().await;
}

// Actual HTTP/WebSocket relay, native Noise identities and ordered input in both
// directions. These disposable records never touch a running account service.
mod relay_regression {
    use super::*;
    use extend_computer_agent::{
        input::InputEvent,
        relay::{Cancellation, Credentials, Policy, Relay},
        session::{self, Client as EngineClient, CursorSink, Decision, Presence},
        trust::TrustStore,
    };
    use std::{
        net::TcpStream,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        time::Instant,
    };
    struct Peer {
        identity: Arc<Identity>,
        trust: TrustStore,
        remote: String,
        fresh: AtomicBool,
        receiving: AtomicBool,
        events: Arc<Mutex<Vec<String>>>,
    }
    struct Sink(Arc<Mutex<Vec<String>>>);
    impl CursorSink for Sink {
        fn start_cursor(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn move_to(&mut self, _: f64, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn start_control(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn supports_text_input(&self) -> bool {
            true
        }
        fn input(&mut self, event: &InputEvent) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(serde_json::to_string(event)?);
            Ok(())
        }
        fn finish(&mut self) {
            self.0.lock().unwrap().push("released".into());
        }
    }
    impl Policy for Peer {
        fn active(&self) -> bool {
            self.fresh.load(Ordering::SeqCst)
        }
        fn allowed(&self, peer: &str, kind: &str) -> bool {
            self.active()
                && peer == self.remote
                && matches!(kind, "presence" | "control")
                && self.trust.peer(peer).ok().flatten().is_some()
        }
        fn receiving(&self, kind: &str) -> bool {
            kind == "presence" || self.receiving.load(Ordering::SeqCst)
        }
        fn incoming(&self, peer: &str, kind: &str) -> anyhow::Result<TcpStream> {
            anyhow::ensure!(
                self.allowed(peer, kind) && self.receiving(kind),
                "Not available"
            );
            let (local, remote) = extend_computer_agent::relay::socket_pair()?;
            let identity = self.identity.clone();
            let trust = self.trust.clone();
            let expected = peer.to_owned();
            let events = self.events.clone();
            let receiving = self.receiving.load(Ordering::SeqCst);
            let kind = kind.to_owned();
            std::thread::spawn(move || {
                if kind == "presence" {
                    let _ = session::serve_presence_pinned(
                        remote,
                        &identity,
                        &trust,
                        &expected,
                        || Presence {
                            receiving,
                            port: 48177,
                            name: "Relay test peer".into(),
                        },
                    );
                } else {
                    let _ = session::serve_connection_with_cursor_pinned(
                        remote,
                        &identity,
                        &trust,
                        &expected,
                        &mut Sink(events),
                    );
                }
            });
            Ok(local)
        }
    }
    fn peer(identity: Arc<Identity>, other: &Identity, path: &std::path::Path) -> Arc<Peer> {
        let trust = TrustStore::open(path).unwrap();
        trust.remember(&other.fingerprint()).unwrap();
        Arc::new(Peer {
            identity,
            trust,
            remote: other.fingerprint(),
            fresh: AtomicBool::new(true),
            receiving: AtomicBool::new(true),
            events: Arc::new(Mutex::new(Vec::new())),
        })
    }
    async fn online(relay: &Relay) {
        let until = Instant::now() + Duration::from_secs(12);
        while !relay.online() {
            assert!(
                Instant::now() < until,
                "Local relay registration never became ready"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_relay_both_directions_pin_identity_receive_text_and_cancel_exact_channels() {
        let http = HttpServer::start_with_relay(true).await;
        let account = http.signup("relay-android@example.invalid").await;
        let android_identity = Arc::new(Identity::generate());
        let desktop_identity = Arc::new(Identity::generate());
        let android = http
            .register(&account, &android_identity, "android", "Android")
            .await;
        let desktop = http
            .register(&account, &desktop_identity, "macos", "Desktop")
            .await;
        for (device, identity) in [(&android, &android_identity), (&desktop, &desktop_identity)] {
            let options = http.options(device).await;
            http.verify(device, proof(identity, &options), StatusCode::NO_CONTENT)
                .await;
        }
        let temporary = tempfile::tempdir().unwrap();
        let a = peer(
            android_identity.clone(),
            &desktop_identity,
            &temporary.path().join("android"),
        );
        let d = peer(
            desktop_identity.clone(),
            &android_identity,
            &temporary.path().join("desktop"),
        );
        let ar = Arc::new(Relay::default());
        let dr = Arc::new(Relay::default());
        for (relay, device, policy) in [(&ar, &android, a.clone()), (&dr, &desktop, d.clone())] {
            relay
                .configure(Some(Credentials {
                    server: http.origin.clone(),
                    scope: "fixture/account".into(),
                    device: device.id.clone(),
                    token: device.token.clone().into(),
                }))
                .unwrap();
            relay.start(policy);
        }
        online(&ar).await;
        online(&dr).await;
        for (relay, source, target, device) in [
            (ar.clone(), a.clone(), d.clone(), desktop.id.clone()),
            (dr.clone(), d.clone(), a.clone(), android.id.clone()),
        ] {
            tokio::task::spawn_blocking(move || {
                let socket = relay
                    .tunnel(
                        &target.identity.fingerprint(),
                        &device,
                        "presence",
                        source.clone(),
                        false,
                        Arc::new(Cancellation::default()),
                    )
                    .unwrap();
                let status = session::query_presence(
                    socket,
                    &source.identity,
                    &target.identity.fingerprint(),
                )
                .unwrap()
                .unwrap();
                assert!(status.receiving);
                let wrong_pin = Identity::generate().fingerprint();
                let socket = relay
                    .tunnel(
                        &target.identity.fingerprint(),
                        &device,
                        "presence",
                        source.clone(),
                        false,
                        Arc::new(Cancellation::default()),
                    )
                    .unwrap();
                assert!(session::query_presence(socket, &source.identity, &wrong_pin).is_err());
                let socket = relay
                    .tunnel(
                        &target.identity.fingerprint(),
                        &device,
                        "control",
                        source.clone(),
                        false,
                        Arc::new(Cancellation::default()),
                    )
                    .unwrap();
                let mut client = EngineClient::connect(
                    socket,
                    &source.identity,
                    &source.trust,
                    None,
                    Some(&target.identity.fingerprint()),
                    |_, _| Decision::Deny,
                )
                .unwrap();
                client.request_control().unwrap();
                assert!(client.supports_text_input());
                let text = InputEvent::Text {
                    value: "漢字 👩‍💻 é fixture".into(),
                };
                client.queue_input(text.clone()).unwrap();
                client
                    .queue_input(InputEvent::Button {
                        button: 0,
                        down: true,
                        clicks: 1,
                    })
                    .unwrap();
                client.probe().unwrap();
                assert!(target
                    .events
                    .lock()
                    .unwrap()
                    .contains(&serde_json::to_string(&text).unwrap()));
                relay.cancel_peer(&target.identity.fingerprint());
                assert!(client.probe().is_err());
                let until = Instant::now() + Duration::from_secs(2);
                while !target
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e == "released")
                {
                    assert!(
                        Instant::now() < until,
                        "Held input not released after relay cancel"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .await
            .unwrap();
        }
        // Fresh membership controls routing independently of retained manual trust.
        a.fresh.store(false, Ordering::SeqCst);
        assert!(a.trust.peer(&d.identity.fingerprint()).unwrap().is_some());
        let relay = ar.clone();
        let source = a.clone();
        let target = d.identity.fingerprint();
        let target_id = desktop.id.clone();
        tokio::task::spawn_blocking(move || {
            assert!(relay
                .tunnel(
                    &target,
                    &target_id,
                    "control",
                    source,
                    false,
                    Arc::new(Cancellation::default())
                )
                .is_err())
        })
        .await
        .unwrap();
        a.fresh.store(true, Ordering::SeqCst);
        d.receiving.store(false, Ordering::SeqCst);
        let relay = ar.clone();
        let source = a.clone();
        let target = d.identity.fingerprint();
        let target_id = desktop.id.clone();
        tokio::task::spawn_blocking(move || {
            let socket = relay
                .tunnel(
                    &target,
                    &target_id,
                    "presence",
                    source.clone(),
                    false,
                    Arc::new(Cancellation::default()),
                )
                .unwrap();
            assert!(
                !session::query_presence(socket, &source.identity, &target)
                    .unwrap()
                    .unwrap()
                    .receiving
            );
        })
        .await
        .unwrap();
        ar.configure(None).unwrap();
        dr.configure(None).unwrap();
        assert!(!ar.online() && !dr.online());
        http.finish().await;
    }
}
