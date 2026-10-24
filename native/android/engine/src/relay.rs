//! Android account relay policy. Account routing remains separate from LAN hints.
use super::*;
use extend_computer_agent::relay::{self, Cancellation, Credentials, Policy};
use extend_computer_agent::session::{self, Presence};

struct AndroidPolicy;
fn runtime() -> Result<&'static Runtime> {
    RUNTIME.get().context("Engine is not initialized")
}
fn current_device(runtime: &Runtime, peer: &str) -> Result<String> {
    let routes = runtime.account.routes.lock().unwrap();
    ensure!(routes.fresh(), "Account device access expired");
    let id = routes
        .devices
        .get(peer)
        .cloned()
        .context("Device is not signed in to this account")?;
    drop(routes);
    ensure!(runtime.trust.peer(peer)?.is_some(), "Device access expired");
    Ok(id)
}
fn receiving_port() -> Option<u16> {
    RECEIVING.lock().unwrap().as_ref().and_then(|listener| {
        let snapshot = listener.snapshot();
        (snapshot["state"] != "closed")
            .then(|| snapshot["port"].as_u64().map(|p| p as u16))
            .flatten()
    })
}
impl Policy for AndroidPolicy {
    fn active(&self) -> bool {
        runtime().is_ok_and(|r| r.account.routes.lock().unwrap().fresh())
    }
    fn allowed(&self, peer: &str, kind: &str) -> bool {
        matches!(kind, "presence" | "control")
            && runtime().is_ok_and(|r| current_device(r, peer).is_ok())
    }
    fn receiving(&self, kind: &str) -> bool {
        kind == "presence" || receiving_port().is_some()
    }
    fn incoming(&self, peer: &str, kind: &str) -> Result<TcpStream> {
        let r = runtime()?;
        current_device(r, peer)?;
        if kind == "control" {
            return RECEIVING
                .lock()
                .unwrap()
                .as_ref()
                .context("Receiving is off")?
                .relay_stream(r.identity.clone(), r.trust.clone(), r.names.clone(), peer);
        }
        ensure!(kind == "presence", "Invalid relay connection kind");
        // No public listener survives notification Stop. This private socket pair
        // serves read-only pinned presence, including the authoritative off state.
        let (local, remote) = relay::socket_pair()?;
        let expected = peer.to_owned();
        std::thread::Builder::new()
            .name("extend-relay-presence".into())
            .spawn(move || {
                let _ = session::serve_presence_pinned(
                    remote,
                    &r.identity,
                    &r.trust,
                    &expected,
                    &|| Presence {
                        receiving: receiving_port().is_some(),
                        port: receiving_port().unwrap_or(48177),
                        name: r.names.local("Android device"),
                    },
                );
            })?;
        Ok(local)
    }
}
fn tunnel(
    r: &Runtime,
    peer: &str,
    kind: &str,
    refresh: bool,
    cancel: Arc<Cancellation>,
) -> Result<TcpStream> {
    let device = current_device(r, peer)?;
    r.relay
        .tunnel(
            peer,
            &device,
            kind,
            Arc::new(AndroidPolicy),
            refresh,
            cancel,
        )
        .map_err(|_| anyhow::anyhow!("Could not reach this device over the internet"))
}
pub(super) fn outgoing(
    peer: &str,
    address: Option<SocketAddr>,
    relay: &Arc<relay::Relay>,
    cancel: Arc<Cancellation>,
) -> Result<(TcpStream, &'static str)> {
    let r = runtime()?;
    ensure!(Arc::ptr_eq(relay, &r.relay), "Account route replaced");
    if let Some(address) = address {
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
            return Ok((socket, "lan"));
        }
    }
    Ok((tunnel(r, peer, "control", true, cancel)?, "internet"))
}
pub(super) fn endpoint_optional(address: &str) -> Result<Option<SocketAddr>> {
    if address.is_empty() {
        Ok(None)
    } else {
        Ok(Some(super::endpoint(address)?))
    }
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureAccountRelay(
    env: JNIEnv,
    _: JClass,
    server: JString,
    scope: JString,
    device: JString,
    token: JString,
    fingerprint: JString,
) -> jstring {
    boundary(env, |env| {
        let server: String = env.get_string(&server)?.into();
        let scope: String = env.get_string(&scope)?.into();
        let device: String = env.get_string(&device)?.into();
        let token = Zeroizing::new(String::from(env.get_string(&token)?));
        let fingerprint: String = env.get_string(&fingerprint)?.into();
        let r = runtime()?;
        ensure!(
            fingerprint == r.identity.fingerprint(),
            "Registered account identity mismatch"
        );
        {
            let routes = r.account.routes.lock().unwrap();
            ensure!(
                routes.fresh() && routes.scope == scope,
                "Account directory changed"
            );
        }
        r.relay.configure(Some(Credentials {
            server,
            scope,
            device,
            token,
        }))?;
        r.relay.start(Arc::new(AndroidPolicy));
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolveAccountPresence(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    refresh: jni::sys::jboolean,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let r = runtime()?;
        let socket = tunnel(
            r,
            &peer,
            "presence",
            refresh != 0,
            Arc::new(Default::default()),
        )?;
        let status = session::query_presence(socket, &r.identity, &peer)?
            .context("This device no longer recognizes the pairing")?;
        current_device(r, &peer)?;
        r.names.save(&peer, &status.name)?;
        // Never persist loopback as a discovered LAN address.
        Ok(serde_json::json!({"peer":peer,"name":status.name,"receiving":status.receiving,"route":"internet"}).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probeAccountPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let r = runtime()?;
        let socket = tunnel(r, &peer, "control", true, Arc::new(Default::default()))?;
        let mut client =
            Client::connect(socket, &r.identity, &r.trust, None, Some(&peer), |_, _| {
                Decision::Deny
            })?;
        let latency = client.probe()?.as_secs_f64() * 1000.;
        client.close()?;
        current_device(r, &peer)?;
        Ok(serde_json::json!({"peer":peer,"rtt_ms":latency,"route":"internet"}).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_account_route_cannot_be_kept_alive_by_manual_trust_and_block_is_authoritative() {
        let dir = tempfile::tempdir().unwrap();
        let peer = Identity::generate().fingerprint();
        let account = accounts::Membership::default();
        let r = Runtime {
            identity: Arc::new(Identity::generate()),
            trust: TrustStore::open(dir.path().join("trust"))
                .unwrap()
                .with_account_trust(account.trust.clone()),
            root: dir.path().into(),
            names: Arc::new(names::Names::open(dir.path()).unwrap()),
            account,
            relay: Arc::new(Default::default()),
        };
        r.trust.remember(&peer).unwrap();
        let device = "3".repeat(64);
        *r.account.routes.lock().unwrap() = accounts::Routes {
            scope: "local/account".into(),
            until: Some(std::time::Instant::now() + Duration::from_secs(60)),
            devices: std::collections::BTreeMap::from([(peer.clone(), device.clone())]),
        };
        assert_eq!(current_device(&r, &peer).unwrap(), device);
        r.account.routes.lock().unwrap().until = Some(std::time::Instant::now());
        assert!(r.trust.peer(&peer).unwrap().is_some());
        assert!(current_device(&r, &peer).is_err());
        r.account.routes.lock().unwrap().until =
            Some(std::time::Instant::now() + Duration::from_secs(60));
        r.trust.complete_unpair(&peer).unwrap();
        assert!(current_device(&r, &peer).is_err());
        assert!(endpoint_optional("").unwrap().is_none());
        assert!(endpoint_optional("127.0.0.1:7").unwrap().is_some());
        assert!(endpoint_optional("localhost:7").is_err());
    }
}
