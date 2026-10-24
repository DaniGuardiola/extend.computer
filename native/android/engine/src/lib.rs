//! Android JNI boundary. Cryptography and trust use the shared desktop engine.
//! Java owns Keystore access; private seed bytes never cross the React bridge.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::session::{Client, Decision};
use extend_computer_agent::{identity::Identity, trust::TrustStore};
use jni::{
    objects::{JByteArray, JClass, JObject, JString},
    sys::jstring,
    JNIEnv,
};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use zeroize::{Zeroize, Zeroizing};

struct Runtime {
    identity: Arc<Identity>,
    trust: TrustStore,
    root: PathBuf,
    names: Arc<names::Names>,
    account: accounts::Membership,
    relay: Arc<extend_computer_agent::relay::Relay>,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
mod accounts;
mod control;
mod cursor_compositor;
mod device_info;
mod incoming;
mod manualpairing;
mod media;
mod names;
mod presence;
mod receiver;
mod relay;
mod sender;
mod unpair;
mod usb;
mod visualpairing;
static MANUAL_PAIRING: Mutex<Option<manualpairing::ManualPairing>> = Mutex::new(None);
static VISUAL_PAIRING: Mutex<Option<visualpairing::VisualPairing>> = Mutex::new(None);
static SENDER: Mutex<Option<Arc<sender::Sender>>> = Mutex::new(None);
static SENDER_START: Mutex<()> = Mutex::new(());
static INCOMING: Mutex<Option<incoming::Incoming>> = Mutex::new(None);
static RECEIVING: Mutex<Option<incoming::Incoming>> = Mutex::new(None);
static MEDIA: Mutex<Option<Arc<media::Server>>> = Mutex::new(None);
static CONTROL: Mutex<Option<Arc<control::Sender>>> = Mutex::new(None);
static CONTROL_START: Mutex<()> = Mutex::new(());
static PRESENCE: Mutex<Option<presence::Server>> = Mutex::new(None);

// Availability checks do not reserve a control/display session. Pairing UI and
// approval also recheck these native states; a running display is never stopped.
fn visual_pairing_idle() -> bool {
    pairing_session_idle()
        && MANUAL_PAIRING
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| !s.active())
}
fn pairing_session_idle() -> bool {
    fn idle(value: serde_json::Value) -> bool {
        matches!(
            value["state"].as_str(),
            Some("idle" | "closed" | "failed" | "disconnected")
        )
    }
    !receiver::shared().active()
        && CONTROL
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
        && SENDER
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
        && MEDIA
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| idle(s.snapshot()))
}

fn endpoint(input: &str) -> Result<SocketAddr> {
    let address: SocketAddr = input.parse().context("Enter an IP address and port")?;
    ensure!(
        address.port() != 0 && !address.ip().is_unspecified() && !address.ip().is_multicast(),
        "Invalid device address"
    );
    Ok(address)
}

fn pair_code(runtime: &Runtime, address: &str, code: &str) -> Result<String> {
    let code = normalize_pair_code(code)?;
    let stream = TcpStream::connect_timeout(&endpoint(address)?, Duration::from_secs(5))
        .context("Could not reach this device")?;
    // Explicit code entry authorizes this PAKE-authenticated pairing, as on desktop.
    let client = Client::connect(
        stream,
        &runtime.identity,
        &runtime.trust,
        Some(&code),
        None,
        |_, _| Decision::Remember,
    )?;
    let peer = client.peer().to_owned();
    client.close()?;
    Ok(peer)
}
fn normalize_pair_code(code: &str) -> Result<Zeroizing<String>> {
    ensure!(code.len() <= 128, "Invalid pairing code");
    let digits = Zeroizing::new(
        code.chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect::<String>(),
    );
    ensure!(
        digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
        "Enter the 16-character code shown on the other device"
    );
    Ok(Zeroizing::new(
        digits
            .as_bytes()
            .chunks(4)
            .map(|part| std::str::from_utf8(part).unwrap().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
    ))
}

fn probe_peer(runtime: &Runtime, address: &str, peer: &str) -> Result<String> {
    ensure!(runtime.trust.peer(peer)?.is_some(), "Unknown device");
    let stream = TcpStream::connect_timeout(&endpoint(address)?, Duration::from_secs(5))
        .context("Could not reach this device")?;
    let mut client = Client::connect(
        stream,
        &runtime.identity,
        &runtime.trust,
        None,
        Some(peer),
        |_, _| Decision::Deny,
    )?;
    let latency = client.probe()?.as_secs_f64() * 1000.0;
    client.close()?;
    Ok(serde_json::json!({"peer":peer,"rtt_ms":latency}).to_string())
}

fn resolve_presence(runtime: &Runtime, address: &str, peer: &str) -> Result<String> {
    ensure!(runtime.trust.peer(peer)?.is_some(), "Unknown device");
    let mut address = endpoint(address)?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .context("Could not reach this device's availability service")?;
    let status = extend_computer_agent::session::query_presence(stream, &runtime.identity, peer)?
        .context("This device no longer recognizes the pairing")?;
    ensure!(runtime.trust.peer(peer)?.is_some(), "Device was forgotten");
    // Only this pinned response establishes the current receiving endpoint.
    runtime.names.save(peer, &status.name)?;
    address.set_port(status.port);
    Ok(
        serde_json::json!({"peer":peer, "receiving":status.receiving,
        "name":status.name, "address":address.to_string()})
        .to_string(),
    )
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolvePresence(
    env: JNIEnv,
    _: JClass,
    address: JString,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let address: String = env.get_string(&address)?.into();
        let peer: String = env.get_string(&peer)?.into();
        resolve_presence(
            RUNTIME.get().context("Engine is not initialized")?,
            &address,
            &peer,
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_peerHosts(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        RUNTIME
            .get()
            .context("Engine is not initialized")?
            .names
            .hosts()
    })
}

fn public_identity(runtime: &Runtime) -> String {
    serde_json::json!({"fingerprint": runtime.identity.fingerprint(),
        "public_key": hex::encode(runtime.identity.public())})
    .to_string()
}

fn initialize(mut seed: Zeroizing<Vec<u8>>, root: PathBuf) -> Result<String> {
    ensure!(seed.len() == 32, "Invalid platform identity seed length");
    ensure!(
        root.is_absolute(),
        "Engine storage must be an absolute app-private path"
    );
    let mut bytes = [0; 32];
    bytes.copy_from_slice(&seed);
    let identity = Identity::from_secret(bytes);
    bytes.zeroize();
    seed.zeroize();
    let account = accounts::Membership::default();
    let trust = TrustStore::open(root.join("trust"))?.with_account_trust(account.trust.clone());
    let expected_root = root.clone();
    let expected_identity = identity.fingerprint();
    let runtime = Runtime {
        identity: Arc::new(identity),
        trust,
        names: Arc::new(names::Names::open(&root)?),
        account,
        relay: Arc::new(Default::default()),
        root,
    };
    let _ = RUNTIME.set(runtime);
    let current = RUNTIME.get().context("Engine initialization failed")?;
    ensure!(
        current.root == expected_root && current.identity.fingerprint() == expected_identity,
        "Engine identity or storage changed during process lifetime"
    );
    Ok(public_identity(current))
}

fn boundary(mut env: JNIEnv, action: impl FnOnce(&mut JNIEnv) -> Result<String>) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&mut env)));
    let value = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", error.to_string());
            return std::ptr::null_mut();
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "Native engine failed");
            return std::ptr::null_mut();
        }
    };
    match env.new_string(value) {
        Ok(value) => value.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "Native engine result unavailable",
            );
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_attachUsb(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        ensure!(
            peer.len() == 64 && peer.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid USB peer identity"
        );
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        ensure!(
            runtime.trust.peer(&peer)?.is_some(),
            "Pair this USB device before connecting"
        );
        Ok(serde_json::json!({"id":usb::attach(peer, runtime.identity.clone(), runtime.trust.clone(), runtime.names.clone())?}).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_usbState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| Ok(usb::snapshot().to_string()))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopUsb(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        usb::stop(&id);
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_receiveUsb(
    mut env: JNIEnv,
    _: JClass,
    id: JString,
    bytes: JByteArray,
    offset: jni::sys::jint,
    length: jni::sys::jint,
) -> jni::sys::jint {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<usize> {
        ensure!(
            offset >= 0
                && length >= 0
                && length <= 16384
                && offset
                    .checked_add(length)
                    .is_some_and(|end| end <= env.get_array_length(&bytes).unwrap_or(-1)),
            "Invalid USB read buffer"
        );
        let id: String = env.get_string(&id)?.into();
        let mut buffer = vec![0_i8; length as usize];
        env.get_byte_array_region(&bytes, offset, &mut buffer)?;
        let bytes: Vec<u8> = buffer.into_iter().map(|b| b as u8).collect();
        usb::receive(&id, &bytes)
    }));
    match result {
        Ok(Ok(count)) => count as jni::sys::jint,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            -1
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "USB engine failed");
            -1
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextUsbFrame(
    mut env: JNIEnv,
    _: JClass,
    id: JString,
) -> jni::sys::jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<jni::sys::jbyteArray> {
            let id: String = env.get_string(&id)?.into();
            Ok(match usb::outgoing(&id)? {
                Some(bytes) => env.byte_array_from_slice(&bytes)?.into_raw(),
                None => std::ptr::null_mut(),
            })
        },
    ));
    match result {
        Ok(Ok(array)) => array,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            std::ptr::null_mut()
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "USB engine failed");
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_initialize(
    env: JNIEnv,
    _: JClass,
    seed: JByteArray,
    root: JString,
    context: JObject,
) -> jstring {
    boundary(env, |env| {
        let root: String = env.get_string(&root)?.into();
        let root = PathBuf::from(root);
        let seed = Zeroizing::new(env.convert_byte_array(&seed)?);
        if let Some(current) = RUNTIME.get() {
            ensure!(
                current.root == root,
                "Engine storage changed during process lifetime"
            );
            ensure!(
                seed.as_slice() == current.identity.secret(),
                "Engine identity changed during process lifetime"
            );
            return Ok(public_identity(current));
        }
        extend_computer_agent::device_info::set_local_kind(device_info::detect(env, &context));
        initialize(seed, root)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_accountProof(
    env: JNIEnv,
    _: JClass,
    server_key: JString,
    challenge: JString,
) -> jstring {
    boundary(env, |env| {
        let key: String = env.get_string(&server_key)?.into();
        let challenge: String = env.get_string(&challenge)?.into();
        RUNTIME
            .get()
            .context("Engine is not initialized")?
            .identity
            .account_proof(&key, &challenge)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pairedPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        let records = RUNTIME
            .get()
            .context("Engine is not initialized")?
            .trust
            .load()?;
        Ok(serde_json::to_string(
            &records.peers.keys().collect::<Vec<_>>(),
        )?)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pairCode(
    env: JNIEnv,
    _: JClass,
    address: JString,
    code: JString,
) -> jstring {
    boundary(env, |env| {
        let address: String = env.get_string(&address)?.into();
        let code = Zeroizing::new(String::from(env.get_string(&code)?));
        ensure!(
            visual_pairing_idle(),
            "Stop the current connection before pairing"
        );
        pair_code(
            RUNTIME.get().context("Engine is not initialized")?,
            &address,
            &code,
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startManualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
    address: JString,
    code: JString,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let address: String = env.get_string(&address)?.into();
        let code = Zeroizing::new(String::from(env.get_string(&code)?));
        let name: String = env.get_string(&name)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let address = endpoint(&address)?;
        let code = normalize_pair_code(&code)?;
        ensure!(
            pairing_session_idle(),
            "Stop the current connection before pairing"
        );
        // Keep the same lock order for manual/visual starts. The manual worker
        // uses only pairing_session_idle, never these operation registry locks.
        let visual = VISUAL_PAIRING.lock().unwrap();
        ensure!(
            !visual.as_ref().is_some_and(|s| s.active()),
            "Pairing already in progress"
        );
        let mut slot = MANUAL_PAIRING.lock().unwrap();
        ensure!(
            !slot.as_ref().is_some_and(|s| s.active()),
            "Pairing already in progress"
        );
        // Reap the previous interrupted attempt on this async startup thread,
        // not on the synchronous UI cancel path. No per-attempt backlog.
        drop(slot.take());
        let operation = manualpairing::ManualPairing::start(
            id,
            runtime.identity.clone(),
            runtime.trust.clone(),
            address,
            code,
            name,
            runtime.names.clone(),
            Duration::from_secs(120),
            Arc::new(pairing_session_idle),
        )?;
        let snapshot = operation.snapshot().to_string();
        *slot = Some(operation);
        Ok(snapshot)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_manualPairingState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let slot = MANUAL_PAIRING.lock().unwrap();
        let operation = slot
            .as_ref()
            .filter(|s| s.id == id)
            .context("Pairing operation changed")?;
        Ok(operation.snapshot().to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopManualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        if let Some(operation) = MANUAL_PAIRING
            .lock()
            .unwrap()
            .as_ref()
            .filter(|s| s.id == id)
        {
            operation.stop();
        }
        Ok("{}".into())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startVisualPairing(
    env: JNIEnv,
    _: JClass,
    address: JString,
    name: JString,
    local_name: JString,
) -> jstring {
    boundary(env, |env| {
        let address: String = env.get_string(&address)?.into();
        let name: String = env.get_string(&name)?.into();
        let local_name: String = env.get_string(&local_name)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let address = endpoint(&address)?;
        ensure!(
            visual_pairing_idle(),
            "Stop the current connection before pairing"
        );
        let mut slot = VISUAL_PAIRING.lock().unwrap();
        ensure!(
            MANUAL_PAIRING
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|s| !s.active()),
            "Pairing already in progress"
        );
        ensure!(
            !slot.as_ref().is_some_and(|s| s.active()),
            "Pairing already in progress"
        );
        let operation = visualpairing::VisualPairing::start(
            runtime.identity.clone(),
            runtime.trust.clone(),
            address,
            name,
            local_name,
            runtime.names.clone(),
            Duration::from_secs(120),
        )?;
        let snapshot = operation.snapshot().to_string();
        *slot = Some(operation);
        Ok(snapshot)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_visualPairingState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let slot = VISUAL_PAIRING.lock().unwrap();
        let operation = slot
            .as_ref()
            .filter(|s| s.id == id)
            .context("Pairing operation changed")?;
        Ok(operation.snapshot().to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_respondVisualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
    request: JString,
    allow: jni::sys::jboolean,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let request: String = env.get_string(&request)?.into();
        let slot = VISUAL_PAIRING.lock().unwrap();
        let operation = slot
            .as_ref()
            .filter(|s| s.id == id)
            .context("Pairing operation changed")?;
        operation.respond(&request, allow != 0)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopVisualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let mut slot = VISUAL_PAIRING.lock().unwrap();
        let operation = slot
            .as_mut()
            .filter(|s| s.id == id)
            .context("Pairing operation changed")?;
        operation.stop();
        Ok(operation.snapshot().to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probePeer(
    env: JNIEnv,
    _: JClass,
    address: JString,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let address: String = env.get_string(&address)?.into();
        let peer: String = env.get_string(&peer)?.into();
        probe_peer(
            RUNTIME.get().context("Engine is not initialized")?,
            &address,
            &peer,
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_forgetPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        unpair_local(runtime, &peer)?;
        Ok(serde_json::json!({"operation":unpair::record(&peer).to_string()}).to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_notifyUnpair(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
    operation: JString,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let address: String = env.get_string(&address)?.into();
        let operation: String = env.get_string(&operation)?.into();
        unpair::notify(
            RUNTIME.get().context("Engine is not initialized")?,
            peer,
            address,
            operation.parse()?,
        )?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_updateLocalDeviceName(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let name: String = env.get_string(&name)?.into();
        RUNTIME
            .get()
            .context("Engine is not initialized")?
            .names
            .set_local(&name)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_removedPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let mut removed = runtime.names.removals()?;
        removed.retain(|peer, _| {
            runtime
                .trust
                .peer(peer)
                .map_or(false, |value| value.is_none())
        });
        Ok(serde_json::to_string(&removed)?)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_dismissRemovedPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        ensure!(
            runtime.trust.peer(&peer)?.is_none(),
            "Device is paired again"
        );
        runtime.names.dismiss_removal(&peer)?;
        Ok("{}".into())
    })
}

fn unpair_local(runtime: &Runtime, peer: &str) -> Result<()> {
    ensure!(
        peer.len() == 64 && hex::decode(peer)?.len() == 32,
        "Expected full device fingerprint"
    );
    // Commit revocation first. Account directory refresh must not restore access.
    runtime.trust.complete_unpair(peer)?;
    accounts::cancel_retired(
        runtime,
        std::collections::BTreeMap::from([(peer.to_owned(), String::new())]),
    )?;
    let accessory = usb::snapshot();
    if accessory["peer"].as_str() == Some(peer) {
        if let Some(id) = accessory["id"].as_str() {
            usb::stop(id);
        }
    }
    runtime.names.forget(peer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{pairing::PairingWindow, session};
    use std::net::TcpListener;
    use std::thread;

    fn runtime(root: PathBuf) -> Runtime {
        Runtime {
            account: accounts::Membership::default(),
            relay: Arc::new(Default::default()),
            identity: Arc::new(Identity::generate()),
            trust: TrustStore::open(root.join("trust")).unwrap(),
            names: Arc::new(names::Names::open(&root).unwrap()),
            root,
        }
    }

    #[test]
    fn local_unpair_removes_manual_and_blocks_account_refresh_without_touching_other_peer() {
        let files = tempfile::tempdir().unwrap();
        let mut android = runtime(files.path().join("android"));
        android.trust = android
            .trust
            .clone()
            .with_account_trust(android.account.trust.clone());
        let manual = Identity::generate().fingerprint();
        let account = Identity::generate().fingerprint();
        let retained = Identity::generate().fingerprint();
        android.trust.remember(&manual).unwrap();
        android.trust.remember(&retained).unwrap();
        android.account.trust.replace(
            "server/account",
            &[account.clone()],
            Duration::from_secs(90),
        );
        for peer in [&manual, &account, &retained] {
            android.names.save(peer, "Paired device").unwrap();
            android
                .names
                .save_host(peer, "127.0.0.1".parse().unwrap())
                .unwrap();
        }
        assert!(unpair_local(&android, "invalid-peer").is_err());
        assert!(android.trust.peer(&manual).unwrap().is_some());
        unpair_local(&android, &manual).unwrap();
        assert!(android.trust.peer(&manual).unwrap().is_none());
        assert!(android.trust.peer(&account).unwrap().is_some());
        unpair_local(&android, &account).unwrap();
        android.account.trust.replace(
            "server/account",
            &[account.clone()],
            Duration::from_secs(90),
        );
        assert!(
            android.trust.peer(&account).unwrap().is_none(),
            "Fresh directory must not undo unpair"
        );
        assert!(android.trust.peer(&retained).unwrap().is_some());
        let names: serde_json::Value =
            serde_json::from_str(&android.names.snapshot().unwrap()).unwrap();
        let hosts: serde_json::Value =
            serde_json::from_str(&android.names.hosts().unwrap()).unwrap();
        assert!(names.get(&manual).is_none() && names.get(&account).is_none());
        assert!(hosts.get(&manual).is_none() && hosts.get(&account).is_none());
        assert_eq!(names[&retained], "Paired device");
        assert_eq!(hosts[&retained], "127.0.0.1");
        let restored = TrustStore::open(android.root.join("trust"))
            .unwrap()
            .with_account_trust(android.account.trust.clone());
        assert!(
            restored.peer(&account).unwrap().is_none(),
            "Account block survives reopen"
        );
        assert!(restored.peer(&retained).unwrap().is_some());
    }

    #[test]
    fn authenticated_unpair_hint_removes_remote_metadata_and_keeps_listener() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        let own = android.identity.fingerprint();
        let retained = Identity::generate().fingerprint();
        android.trust.remember(&peer).unwrap();
        remote.trust.remember(&own).unwrap();
        remote.trust.remember(&retained).unwrap();
        remote.names.record_removal(&retained).unwrap();
        assert!(
            remote.names.removals().unwrap().is_empty(),
            "Unknown metadata must not create unsolicited notices"
        );
        remote.names.save(&own, "Android tablet").unwrap();
        remote
            .names
            .save_host(&own, "127.0.0.1".parse().unwrap())
            .unwrap();
        let mut listener = incoming::Incoming::named(
            remote.identity.clone(),
            remote.trust.clone(),
            None,
            "Mac".into(),
            remote.names.clone(),
        )
        .unwrap();
        let address = format!(
            "127.0.0.1:{}",
            listener.snapshot()["port"].as_u64().unwrap()
        );
        unpair_local(&android, &peer).unwrap();
        let id = unpair::record(&peer);
        unpair::notify(&android, peer.clone(), address, id).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while (!remote.names.removals().unwrap().contains_key(&own)
            || remote.names.snapshot().unwrap().contains(&own)
            || remote.names.hosts().unwrap().contains(&own))
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(remote.trust.peer(&own).unwrap().is_none());
        assert_eq!(
            remote
                .names
                .removals()
                .unwrap()
                .get(&own)
                .map(String::as_str),
            Some("Android tablet")
        );
        assert!(!remote.names.snapshot().unwrap().contains(&own));
        assert!(!remote.names.hosts().unwrap().contains(&own));
        assert!(remote.trust.peer(&retained).unwrap().is_some());
        assert_ne!(listener.snapshot()["state"], "closed");
        let restored = names::Names::open(&remote.root).unwrap();
        assert!(restored.removals().unwrap().contains_key(&own));
        // A new authenticated name after pairing clears the old removal notice.
        remote.trust.remember(&own).unwrap();
        remote.names.save(&own, "Android tablet").unwrap();
        assert!(!remote.names.removals().unwrap().contains_key(&own));
        listener.stop();
    }

    #[test]
    fn unpair_hint_rejects_stale_and_repaired_operations() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let peer = Identity::generate().fingerprint();
        let old = unpair::record(&peer);
        let newer = unpair::record(&peer);
        assert!(unpair::notify(&android, peer.clone(), "127.0.0.1:9".into(), old).is_err());
        android.trust.remember(&peer).unwrap();
        assert!(unpair::notify(&android, peer.clone(), "127.0.0.1:9".into(), newer).is_err());
        assert!(android.trust.peer(&peer).unwrap().is_some());
    }

    #[test]
    fn unpair_hint_deadline_closes_stalled_peer() {
        use std::io::Read;
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let peer = Identity::generate().fingerprint();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0u8; 4096];
            loop {
                match socket.read(&mut bytes) {
                    Ok(0) => {
                        tx.send(()).unwrap();
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => panic!("Unpair deadline did not close socket: {error}"),
                }
            }
        });
        let start = std::time::Instant::now();
        unpair::notify(&android, peer.clone(), address, unpair::record(&peer)).unwrap();
        rx.recv_timeout(Duration::from_secs(4)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(4));
        server.join().unwrap();
    }

    #[test]
    fn local_name_updates_live_authenticated_exchange_without_restarting_listener() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let mut listener = incoming::Incoming::named(
            remote.identity.clone(),
            remote.trust.clone(),
            None,
            "Original".into(),
            remote.names.clone(),
        )
        .unwrap();
        let port = listener.snapshot()["port"].as_u64().unwrap();
        remote.names.set_local("Updated tablet").unwrap();
        assert!(remote.names.set_local("\n").is_err());
        let name = session::exchange_device_name(
            TcpStream::connect(("127.0.0.1", port as u16)).unwrap(),
            &android.identity,
            &peer,
            "Client",
        )
        .unwrap();
        assert_eq!(name, "Updated tablet");
        assert_eq!(listener.snapshot()["port"].as_u64().unwrap(), port);
        listener.stop();
    }

    #[test]
    fn presence_resolution_pins_identity_repairs_port_and_preserves_trust_on_failure() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let _ = session::serve_presence(
                    listener.accept().unwrap().0,
                    &remote.identity,
                    &remote.trust,
                    || session::Presence {
                        receiving: true,
                        port: 54321,
                        name: "Authenticated Desktop".into(),
                    },
                );
            }
        });
        let status: serde_json::Value =
            serde_json::from_str(&resolve_presence(&android, &address, &peer).unwrap()).unwrap();
        assert_eq!(status["address"], "127.0.0.1:54321");
        assert_eq!(status["name"], "Authenticated Desktop");
        let wrong = Identity::generate().fingerprint();
        android.trust.remember(&wrong).unwrap();
        assert!(resolve_presence(&android, &address, &wrong).is_err());
        server.join().unwrap();
        let names: serde_json::Value =
            serde_json::from_str(&android.names.snapshot().unwrap()).unwrap();
        assert!(names.get(&wrong).is_none());
        assert!(android.trust.peer(&peer).unwrap().is_some());
        assert!(android.trust.peer(&wrong).unwrap().is_some());
        android
            .names
            .save_host(&peer, "127.0.0.1".parse().unwrap())
            .unwrap();
        android.names.forget(&peer).unwrap();
        assert_eq!(android.names.hosts().unwrap(), "{}");
        assert_eq!(
            names::Names::open(&android.root).unwrap().hosts().unwrap(),
            "{}"
        );
        android.trust.forget(&peer).unwrap();
        assert!(resolve_presence(&android, &address, &peer).is_err());
    }

    #[test]
    fn presence_resolution_preserves_ipv6_scope_when_repairing_port() {
        let files = tempfile::tempdir().unwrap();
        let android = runtime(files.path().join("android"));
        let remote = runtime(files.path().join("remote"));
        let peer = remote.identity.fingerprint();
        android.trust.remember(&peer).unwrap();
        remote
            .trust
            .remember(&android.identity.fingerprint())
            .unwrap();
        let listener = TcpListener::bind("[::1]:0").unwrap();
        let mut address = listener.local_addr().unwrap();
        let SocketAddr::V6(ref mut v6) = address else {
            panic!("IPv6 listener expected")
        };
        v6.set_scope_id(7);
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            let socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "Presence client did not connect"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Presence accept failed: {error}"),
                }
            };
            session::serve_presence(socket, &remote.identity, &remote.trust, || {
                session::Presence {
                    receiving: true,
                    port: 54322,
                    name: "Scoped Desktop".into(),
                }
            })
            .unwrap();
        });
        let result = resolve_presence(&android, &address.to_string(), &peer);
        server.join().unwrap();
        let status: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(status["address"], "[::1%7]:54322");
        assert_eq!(status["name"], "Scoped Desktop");
        assert!(android.trust.peer(&peer).unwrap().is_some());
    }

    #[test]
    fn real_pair_reconnect_wrong_code_wrong_identity_and_forget() {
        let scratch = tempfile::tempdir().unwrap();
        let android = runtime(scratch.path().join("android"));
        let desktop = runtime(scratch.path().join("desktop"));
        let server_peer = desktop.identity.fingerprint();
        let tablet_peer = android.identity.fingerprint();
        let server_store = desktop.trust.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let window = PairingWindow::new(Duration::from_secs(120));
        let code = window.code().unwrap().to_string();
        let server = thread::spawn(move || {
            let mut window = Some(window);
            let mut accepted = vec![];
            for _ in 0..3 {
                let (socket, _) = listener.accept().unwrap();
                accepted.push(
                    session::serve_connection(
                        socket,
                        &desktop.identity,
                        &desktop.trust,
                        &mut window,
                        |_, _| Decision::Remember,
                    )
                    .is_ok(),
                );
            }
            accepted
        });
        let peer = pair_code(
            &android,
            &address,
            &code.replace('-', "").to_ascii_uppercase(),
        )
        .unwrap();
        assert_eq!(peer, server_peer);
        assert!(server_store.peer(&tablet_peer).unwrap().is_some());
        assert!(probe_peer(&android, &address, &peer).is_ok());
        let wrong_peer = Identity::generate().fingerprint();
        android.trust.remember(&wrong_peer).unwrap();
        assert!(probe_peer(&android, &address, &wrong_peer).is_err());
        android.trust.forget(&peer).unwrap();
        assert!(probe_peer(&android, &address, &peer).is_err());
        assert_eq!(server.join().unwrap(), [true, true, false]);

        let rejected = runtime(scratch.path().join("rejected"));
        let remote = runtime(scratch.path().join("wrong-code-remote"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut window = Some(PairingWindow::new(Duration::from_secs(120)));
        let server = thread::spawn(move || {
            session::serve_connection(
                listener.accept().unwrap().0,
                &remote.identity,
                &remote.trust,
                &mut window,
                |_, _| Decision::Remember,
            )
            .is_err()
        });
        assert!(pair_code(&rejected, &address, "ffff-ffff-ffff-ffff").is_err());
        assert!(rejected.trust.load().unwrap().peers.is_empty());
        assert!(server.join().unwrap());
    }
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startIncoming(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let name: String = env.get_string(&name)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        ensure!(
            visual_pairing_idle(),
            "Stop the current connection before pairing"
        );
        let mut slot = INCOMING
            .lock()
            .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?;
        if let Some(mut previous) = slot.take() {
            previous.stop();
        }
        let listener = incoming::Incoming::named(
            runtime.identity.clone(),
            runtime.trust.clone(),
            Some(Duration::from_secs(120)),
            name,
            runtime.names.clone(),
        )?;
        let snapshot = listener.snapshot().to_string();
        *slot = Some(listener);
        Ok(snapshot)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_peerNames(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        RUNTIME
            .get()
            .context("Engine is not initialized")?
            .names
            .snapshot()
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startReceiving(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let name: String = env.get_string(&name)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let mut slot = RECEIVING
            .lock()
            .map_err(|_| anyhow::anyhow!("Receiving unavailable"))?;
        if slot.is_none() {
            *slot = Some(incoming::Incoming::receiving(
                runtime.identity.clone(),
                runtime.trust.clone(),
                name,
                runtime.names.clone(),
                receiver::shared(),
            )?);
        }
        Ok(slot.as_ref().unwrap().snapshot().to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_receivingState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        Ok(RECEIVING
            .lock()
            .map_err(|_| anyhow::anyhow!("Receiving unavailable"))?
            .as_ref()
            .map(|listener| listener.snapshot())
            .unwrap_or_else(|| serde_json::json!({"state":"closed","port":0}))
            .to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopReceiving(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        if let Some(runtime) = RUNTIME.get() {
            runtime.relay.cancel_kind("control");
        }
        if let Some(mut listener) = RECEIVING
            .lock()
            .map_err(|_| anyhow::anyhow!("Receiving unavailable"))?
            .take()
        {
            listener.stop();
        }
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_incomingState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        let slot = INCOMING
            .lock()
            .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?;
        Ok(slot.as_ref().map(|listener| listener.snapshot()).unwrap_or_else(|| serde_json::json!({"state":"closed","port":0,"code":null,"secondsRemaining":0,"request":null})).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_respondIncoming(
    env: JNIEnv,
    _: JClass,
    id: JString,
    allow: jni::sys::jboolean,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        INCOMING
            .lock()
            .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?
            .as_ref()
            .context("Pairing closed")?
            .respond(&id, allow != 0)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopIncoming(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        if let Some(mut listener) = INCOMING
            .lock()
            .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?
            .take()
        {
            listener.stop();
        }
        Ok("{}".into())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startPresence(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let name: String = env.get_string(&name)?.into();
        extend_computer_agent::session::validate_device_name(&name)?;
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let mut slot = PRESENCE
            .lock()
            .map_err(|_| anyhow::anyhow!("Presence unavailable"))?;
        if slot.is_none() {
            let names = runtime.names.clone();
            *slot = Some(presence::Server::start(
                runtime.identity.clone(),
                runtime.trust.clone(),
                move || {
                    let snapshot = RECEIVING
                        .lock()
                        .ok()
                        .and_then(|slot| slot.as_ref().map(|listener| listener.snapshot()));
                    let port = snapshot
                        .as_ref()
                        .and_then(|s| s["port"].as_u64())
                        .filter(|port| *port != 0)
                        .unwrap_or(48177) as u16;
                    extend_computer_agent::session::Presence {
                        receiving: snapshot.is_some_and(|s| s["state"] != "closed"),
                        port,
                        name: names.local(&name),
                    }
                },
            )?);
        }
        Ok(serde_json::json!({"port":slot.as_ref().unwrap().port()}).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopPresence(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        if let Some(mut server) = PRESENCE
            .lock()
            .map_err(|_| anyhow::anyhow!("Presence unavailable"))?
            .take()
        {
            server.stop();
        }
        Ok("{}".into())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startDisplayReceiver(
    env: JNIEnv,
    _: JClass,
    capabilities: JString,
) -> jstring {
    boundary(env, |env| {
        let caps: String = env.get_string(&capabilities)?.into();
        ensure!(caps.len() <= 8192, "Decoder capabilities too large");
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let mut slot = MEDIA.lock().unwrap();
        if slot.is_none() {
            *slot = Some(Arc::new(media::Server::start(
                runtime.identity.clone(),
                runtime.trust.clone(),
                serde_json::from_str(&caps)?,
            )?));
        }
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopDisplayReceiver(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        let server = MEDIA.lock().unwrap().take();
        if let Some(server) = server {
            server.stop();
        }
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        Ok(MEDIA
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
            .to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayReceiverSettings(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        Ok(media_server()?.receiver_settings(&id)?.to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_requestDisplayReceiverSettings(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jni::sys::jlong,
    settings: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let settings: String = env.get_string(&settings)?.into();
        ensure!(revision >= 0, "Invalid display revision");
        let request = media_server()?.request_receiver_settings(&id, revision as u64, &settings)?;
        Ok(serde_json::json!({"id":request}).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_attachDisplay(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        media_server()?.attach(&id)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayInputState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        Ok(media_server()?.input_state(&id)?.to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_setDisplayInputEnabled(
    env: JNIEnv,
    _: JClass,
    id: JString,
    enabled: jni::sys::jboolean,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        media_server()?.set_input_enabled(&id, enabled != 0)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayInputCommand(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jni::sys::jlong,
    command: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let command: String = env.get_string(&command)?.into();
        ensure!(revision >= 0, "Invalid display revision");
        media_server()?.input_command(&id, revision as u64, &command)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayConfigured(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jni::sys::jlong,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        ensure!(revision >= 0, "Invalid display revision");
        media_server()?.configured(&id, revision as u64)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayRendered(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jni::sys::jlong,
    presentation_us: jni::sys::jlong,
    device_ns: jni::sys::jlong,
    callback_device_ns: jni::sys::jlong,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        ensure!(
            revision >= 0 && presentation_us >= 0 && device_ns >= 0 && callback_device_ns >= 0,
            "Invalid rendered timestamp"
        );
        let timestamp = (presentation_us as u64)
            .checked_mul(1000)
            .context("Rendered timestamp overflow")?;
        media_server()?.rendered(
            &id,
            revision as u64,
            timestamp,
            device_ns as u64,
            callback_device_ns as u64,
        )?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_cancelDisplay(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let server = MEDIA.lock().unwrap().clone();
        if let Some(server) = server {
            server.cancel(Some(&id));
        }
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextDisplayFrame(
    mut env: JNIEnv,
    _: JClass,
    id: JString,
) -> jni::sys::jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<jni::sys::jbyteArray> {
            let id: String = env.get_string(&id)?.into();
            let frame = media_server()?.take_frame(&id)?;
            Ok(match frame {
                Some(bytes) => env.byte_array_from_slice(&bytes)?.into_raw(),
                None => std::ptr::null_mut(),
            })
        },
    ));
    match result {
        Ok(Ok(array)) => array,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            std::ptr::null_mut()
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "Display engine failed");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextDisplayCursor(
    mut env: JNIEnv,
    _: JClass,
    id: JString,
) -> jni::sys::jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<jni::sys::jbyteArray> {
            let id: String = env.get_string(&id)?.into();
            Ok(match media_server()?.take_cursor(&id)? {
                Some(bytes) => env.byte_array_from_slice(&bytes)?.into_raw(),
                None => std::ptr::null_mut(),
            })
        },
    ));
    match result {
        Ok(Ok(array)) => array,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            std::ptr::null_mut()
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "Display cursor engine failed",
            );
            std::ptr::null_mut()
        }
    }
}

fn media_server() -> Result<Arc<media::Server>> {
    MEDIA.lock().unwrap().clone().context("Receiving disabled")
}

fn control_sender() -> Result<Arc<control::Sender>> {
    CONTROL
        .lock()
        .unwrap()
        .clone()
        .context("Control is not active")
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startControl(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
) -> jstring {
    boundary(env, |env| {
        let _start = CONTROL_START.lock().unwrap();
        let peer: String = env.get_string(&peer)?.into();
        let address: String = env.get_string(&address)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let old = CONTROL.lock().unwrap().take();
        drop(old);
        let sender = Arc::new(control::Sender::start_routed(
            runtime.identity.clone(),
            runtime.trust.clone(),
            peer,
            address,
            runtime.relay.clone(),
        )?);
        let snapshot = sender.snapshot();
        *CONTROL.lock().unwrap() = Some(sender);
        Ok(snapshot.to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_controlState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        Ok(CONTROL
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
            .to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_controlEvent(
    env: JNIEnv,
    _: JClass,
    id: JString,
    event: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let event: String = env.get_string(&event)?.into();
        control_sender()?.enqueue(&id, &event)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopControl(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        control_sender()?.cancel(&id);
        Ok("{}".into())
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureTextReceiver(
    env: JNIEnv,
    _: JClass,
    enabled: jni::sys::jboolean,
) -> jstring {
    boundary(env, |_| {
        receiver::shared().configure_text(enabled != 0);
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureInputKeyboardAvailable(
    env: JNIEnv,
    _: JClass,
    enabled: jni::sys::jboolean,
) -> jstring {
    boundary(env, |_| {
        receiver::shared().configure_keyboard(enabled != 0);
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureInputReceiver(
    env: JNIEnv,
    _: JClass,
    enabled: jni::sys::jboolean,
    width: jni::sys::jdouble,
    height: jni::sys::jdouble,
) -> jstring {
    boundary(env, |_| {
        receiver::shared().configure(enabled != 0, width, height)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pollInputReceiver(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| Ok(receiver::shared().drain().to_string()))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startDisplaySender(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
    config: JString,
) -> jstring {
    boundary(env, |env| {
        let _start = SENDER_START.lock().unwrap();
        let peer: String = env.get_string(&peer)?.into();
        let address: String = env.get_string(&address)?.into();
        let config: String = env.get_string(&config)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let old = SENDER.lock().unwrap().take();
        drop(old);
        let sender = Arc::new(sender::Sender::start_routed(
            runtime.identity.clone(),
            runtime.trust.clone(),
            peer,
            address,
            serde_json::from_str(&config)?,
        )?);
        let state = sender.snapshot();
        *SENDER.lock().unwrap() = Some(sender);
        Ok(state.to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureDisplaySender(
    env: JNIEnv,
    _: JClass,
    id: JString,
    config: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        let config: String = env.get_string(&config)?.into();
        let sender = SENDER
            .lock()
            .unwrap()
            .clone()
            .context("Capture is not active")?;
        sender.configure(&id, serde_json::from_str(&config)?)?;
        Ok(sender.snapshot().to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displaySenderState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        Ok(SENDER
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
            .to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pushDisplayFrame(
    env: JNIEnv,
    _: JClass,
    id: JString,
    bytes: JByteArray,
    timestamp: jni::sys::jlong,
    keyframe: jni::sys::jboolean,
) -> jstring {
    boundary(env, |env| {
        ensure!(timestamp > 0, "Invalid capture timestamp");
        ensure!(
            env.get_array_length(&bytes)? <= 8 * 1024 * 1024,
            "Encoded frame too large"
        );
        let id: String = env.get_string(&id)?.into();
        let sender = SENDER
            .lock()
            .unwrap()
            .clone()
            .context("Capture is not active")?;
        sender.enqueue(
            &id,
            extend_computer_agent::display::Frame {
                timestamp: timestamp as u64,
                keyframe: keyframe != 0,
                bytes: env.convert_byte_array(&bytes)?,
            },
        )?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopDisplaySender(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let id: String = env.get_string(&id)?.into();
        if let Some(sender) = SENDER.lock().unwrap().as_ref() {
            sender.cancel(&id);
        }
        Ok("{}".into())
    })
}
