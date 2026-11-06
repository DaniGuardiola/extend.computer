//! Platform-neutral engine API. Each function backs one platform export (JNI on
//! Android, the C ABI on iOS): plain strings in, the same JSON strings out.
//! Platform shims only convert arguments and report errors.
use super::*;
pub use extend_computer_agent::device_info::DeviceKind;

pub use crate::accounts::{account_peer_names, clear_account_peers, replace_account_peers};
pub use crate::relay::{configure_account_relay, probe_account_peer, resolve_account_presence};
pub use crate::usb::{probe_usb_peer, resolve_usb_presence};

/// Largest accessory read accepted per call.
pub const USB_READ_LIMIT: usize = 16384;
/// Largest encoded capture frame accepted per call.
pub const FRAME_LIMIT: usize = 8 * 1024 * 1024;

fn runtime() -> Result<&'static Runtime> {
    RUNTIME.get().context("Engine is not initialized")
}

/// Starts the engine once per process. `kind` runs only on first initialization.
pub fn initialize(
    seed: Zeroizing<Vec<u8>>,
    root: &str,
    kind: impl FnOnce() -> DeviceKind,
) -> Result<String> {
    let root = PathBuf::from(root);
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
    extend_computer_agent::device_info::set_local_kind(kind());
    super::initialize(seed, root)
}

pub fn account_proof(server_key: &str, challenge: &str) -> Result<String> {
    runtime()?.identity.account_proof(server_key, challenge)
}

pub fn resolve_presence(address: &str, peer: &str) -> Result<String> {
    super::resolve_presence(runtime()?, address, peer)
}

pub fn peer_hosts() -> Result<String> {
    runtime()?.names.hosts()
}

pub fn attach_usb(peer: &str) -> Result<String> {
    ensure!(
        peer.len() == 64 && peer.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid USB peer identity"
    );
    let runtime = runtime()?;
    ensure!(
        runtime.trust.peer(peer)?.is_some(),
        "Pair this USB device before connecting"
    );
    Ok(serde_json::json!({"id":usb::attach(peer.to_owned(), runtime.identity.clone(), runtime.trust.clone(), runtime.names.clone())?}).to_string())
}
pub fn usb_state() -> Result<String> {
    Ok(usb::snapshot().to_string())
}
pub fn stop_usb(id: &str) -> Result<String> {
    usb::stop(id);
    Ok("{}".into())
}
/// Feeds accessory bytes read by the platform; returns the accepted count.
pub fn receive_usb(id: &str, bytes: &[u8]) -> Result<usize> {
    ensure!(bytes.len() <= USB_READ_LIMIT, "Invalid USB read buffer");
    usb::receive(id, bytes)
}
/// Next accessory frame to write, or `None` when nothing is queued.
pub fn next_usb_frame(id: &str) -> Result<Option<Vec<u8>>> {
    usb::outgoing(id)
}

pub fn paired_peers() -> Result<String> {
    let records = runtime()?.trust.load()?;
    Ok(serde_json::to_string(
        &records.peers.keys().collect::<Vec<_>>(),
    )?)
}

/// Returns the paired peer fingerprint as a plain (non-JSON) string.
pub fn pair_code(address: &str, code: &str) -> Result<String> {
    ensure!(
        visual_pairing_idle(),
        "Stop the current connection before pairing"
    );
    super::pair_code(runtime()?, address, code)
}

pub fn start_manual_pairing(id: &str, address: &str, code: &str, name: &str) -> Result<String> {
    let runtime = runtime()?;
    let address = endpoint(address)?;
    let code = normalize_pair_code(code)?;
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
        id.to_owned(),
        runtime.identity.clone(),
        runtime.trust.clone(),
        address,
        code,
        name.to_owned(),
        runtime.names.clone(),
        Duration::from_secs(120),
        Arc::new(pairing_session_idle),
    )?;
    let snapshot = operation.snapshot().to_string();
    *slot = Some(operation);
    Ok(snapshot)
}
pub fn manual_pairing_state(id: &str) -> Result<String> {
    let slot = MANUAL_PAIRING.lock().unwrap();
    let operation = slot
        .as_ref()
        .filter(|s| s.id == id)
        .context("Pairing operation changed")?;
    Ok(operation.snapshot().to_string())
}
pub fn stop_manual_pairing(id: &str) -> Result<String> {
    if let Some(operation) = MANUAL_PAIRING
        .lock()
        .unwrap()
        .as_ref()
        .filter(|s| s.id == id)
    {
        operation.stop();
    }
    Ok("{}".into())
}

pub fn start_visual_pairing(address: &str, name: &str, local_name: &str) -> Result<String> {
    let runtime = runtime()?;
    let address = endpoint(address)?;
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
        name.to_owned(),
        local_name.to_owned(),
        runtime.names.clone(),
        Duration::from_secs(120),
    )?;
    let snapshot = operation.snapshot().to_string();
    *slot = Some(operation);
    Ok(snapshot)
}
pub fn visual_pairing_state(id: &str) -> Result<String> {
    let slot = VISUAL_PAIRING.lock().unwrap();
    let operation = slot
        .as_ref()
        .filter(|s| s.id == id)
        .context("Pairing operation changed")?;
    Ok(operation.snapshot().to_string())
}
pub fn respond_visual_pairing(id: &str, request: &str, allow: bool) -> Result<String> {
    let slot = VISUAL_PAIRING.lock().unwrap();
    let operation = slot
        .as_ref()
        .filter(|s| s.id == id)
        .context("Pairing operation changed")?;
    operation.respond(request, allow)?;
    Ok("{}".into())
}
pub fn stop_visual_pairing(id: &str) -> Result<String> {
    let mut slot = VISUAL_PAIRING.lock().unwrap();
    let operation = slot
        .as_mut()
        .filter(|s| s.id == id)
        .context("Pairing operation changed")?;
    operation.stop();
    Ok(operation.snapshot().to_string())
}

pub fn probe_peer(address: &str, peer: &str) -> Result<String> {
    super::probe_peer(runtime()?, address, peer)
}

pub fn forget_peer(peer: &str) -> Result<String> {
    unpair_local(runtime()?, peer)?;
    Ok(serde_json::json!({"operation":unpair::record(peer).to_string()}).to_string())
}

pub fn notify_unpair(peer: &str, address: &str, operation: &str) -> Result<String> {
    unpair::notify(
        runtime()?,
        peer.to_owned(),
        address.to_owned(),
        operation.parse()?,
    )?;
    Ok("{}".into())
}
pub fn update_local_device_name(name: &str) -> Result<String> {
    runtime()?.names.set_local(name)?;
    Ok("{}".into())
}
pub fn removed_peers() -> Result<String> {
    let runtime = runtime()?;
    let mut removed = runtime.names.removals()?;
    removed.retain(|peer, _| {
        runtime
            .trust
            .peer(peer)
            .map_or(false, |value| value.is_none())
    });
    Ok(serde_json::to_string(&removed)?)
}
pub fn dismiss_removed_peer(peer: &str) -> Result<String> {
    let runtime = runtime()?;
    ensure!(
        runtime.trust.peer(peer)?.is_none(),
        "Device is paired again"
    );
    runtime.names.dismiss_removal(peer)?;
    Ok("{}".into())
}

pub fn start_incoming(name: &str) -> Result<String> {
    let runtime = runtime()?;
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
        name.to_owned(),
        runtime.names.clone(),
    )?;
    let snapshot = listener.snapshot().to_string();
    *slot = Some(listener);
    Ok(snapshot)
}

pub fn peer_names() -> Result<String> {
    runtime()?.names.snapshot()
}
pub fn start_receiving(name: &str) -> Result<String> {
    let runtime = runtime()?;
    let mut slot = RECEIVING
        .lock()
        .map_err(|_| anyhow::anyhow!("Receiving unavailable"))?;
    if slot.is_none() {
        *slot = Some(incoming::Incoming::receiving(
            runtime.identity.clone(),
            runtime.trust.clone(),
            name.to_owned(),
            runtime.names.clone(),
            receiver::shared(),
        )?);
    }
    Ok(slot.as_ref().unwrap().snapshot().to_string())
}
pub fn receiving_state() -> Result<String> {
    Ok(RECEIVING
        .lock()
        .map_err(|_| anyhow::anyhow!("Receiving unavailable"))?
        .as_ref()
        .map(|listener| listener.snapshot())
        .unwrap_or_else(|| serde_json::json!({"state":"closed","port":0}))
        .to_string())
}
pub fn stop_receiving() -> Result<String> {
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
}
pub fn incoming_state() -> Result<String> {
    let slot = INCOMING
        .lock()
        .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?;
    Ok(slot.as_ref().map(|listener| listener.snapshot()).unwrap_or_else(|| serde_json::json!({"state":"closed","port":0,"code":null,"secondsRemaining":0,"request":null})).to_string())
}
pub fn respond_incoming(id: &str, allow: bool) -> Result<String> {
    INCOMING
        .lock()
        .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?
        .as_ref()
        .context("Pairing closed")?
        .respond(id, allow)?;
    Ok("{}".into())
}
pub fn stop_incoming() -> Result<String> {
    if let Some(mut listener) = INCOMING
        .lock()
        .map_err(|_| anyhow::anyhow!("Pairing unavailable"))?
        .take()
    {
        listener.stop();
    }
    Ok("{}".into())
}

pub fn start_presence(name: &str) -> Result<String> {
    extend_computer_agent::session::validate_device_name(name)?;
    let runtime = runtime()?;
    let mut slot = PRESENCE
        .lock()
        .map_err(|_| anyhow::anyhow!("Presence unavailable"))?;
    if slot.is_none() {
        let names = runtime.names.clone();
        let name = name.to_owned();
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
}
pub fn stop_presence() -> Result<String> {
    if let Some(mut server) = PRESENCE
        .lock()
        .map_err(|_| anyhow::anyhow!("Presence unavailable"))?
        .take()
    {
        server.stop();
    }
    Ok("{}".into())
}

pub fn start_display_receiver(capabilities: &str) -> Result<String> {
    ensure!(capabilities.len() <= 8192, "Decoder capabilities too large");
    let runtime = runtime()?;
    let mut slot = MEDIA.lock().unwrap();
    if slot.is_none() {
        *slot = Some(Arc::new(media::Server::start(
            runtime.identity.clone(),
            runtime.trust.clone(),
            serde_json::from_str(capabilities)?,
        )?));
    }
    Ok("{}".into())
}
pub fn stop_display_receiver() -> Result<String> {
    let server = MEDIA.lock().unwrap().take();
    if let Some(server) = server {
        server.stop();
    }
    Ok("{}".into())
}
pub fn display_state() -> Result<String> {
    Ok(MEDIA
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.snapshot())
        .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
        .to_string())
}
pub fn display_receiver_settings(id: &str) -> Result<String> {
    Ok(media_server()?.receiver_settings(id)?.to_string())
}
pub fn request_display_receiver_settings(
    id: &str,
    revision: i64,
    settings: &str,
) -> Result<String> {
    ensure!(revision >= 0, "Invalid display revision");
    let request = media_server()?.request_receiver_settings(id, revision as u64, settings)?;
    Ok(serde_json::json!({"id":request}).to_string())
}
pub fn attach_display(id: &str) -> Result<String> {
    media_server()?.attach(id)?;
    Ok("{}".into())
}
pub fn display_input_state(id: &str) -> Result<String> {
    Ok(media_server()?.input_state(id)?.to_string())
}
pub fn set_display_input_enabled(id: &str, enabled: bool) -> Result<String> {
    media_server()?.set_input_enabled(id, enabled)?;
    Ok("{}".into())
}
pub fn display_input_command(id: &str, revision: i64, command: &str) -> Result<String> {
    ensure!(revision >= 0, "Invalid display revision");
    media_server()?.input_command(id, revision as u64, command)?;
    Ok("{}".into())
}
pub fn display_configured(id: &str, revision: i64) -> Result<String> {
    ensure!(revision >= 0, "Invalid display revision");
    media_server()?.configured(id, revision as u64)?;
    Ok("{}".into())
}
pub fn display_rendered(
    id: &str,
    revision: i64,
    presentation_us: i64,
    device_ns: i64,
    callback_device_ns: i64,
) -> Result<String> {
    ensure!(
        revision >= 0 && presentation_us >= 0 && device_ns >= 0 && callback_device_ns >= 0,
        "Invalid rendered timestamp"
    );
    let timestamp = (presentation_us as u64)
        .checked_mul(1000)
        .context("Rendered timestamp overflow")?;
    media_server()?.rendered(
        id,
        revision as u64,
        timestamp,
        device_ns as u64,
        callback_device_ns as u64,
    )?;
    Ok("{}".into())
}
pub fn cancel_display(id: &str) -> Result<String> {
    let server = MEDIA.lock().unwrap().clone();
    if let Some(server) = server {
        server.cancel(Some(id));
    }
    Ok("{}".into())
}
/// Next received display frame, or `None` when nothing is queued.
pub fn next_display_frame(id: &str) -> Result<Option<Vec<u8>>> {
    media_server()?.take_frame(id)
}
/// Next received cursor update, or `None` when nothing is queued.
pub fn next_display_cursor(id: &str) -> Result<Option<Vec<u8>>> {
    media_server()?.take_cursor(id)
}

pub fn start_control(peer: &str, address: &str) -> Result<String> {
    let _start = CONTROL_START.lock().unwrap();
    let runtime = runtime()?;
    let old = CONTROL.lock().unwrap().take();
    drop(old);
    let sender = Arc::new(control::Sender::start_routed(
        runtime.identity.clone(),
        runtime.trust.clone(),
        peer.to_owned(),
        address.to_owned(),
        runtime.relay.clone(),
    )?);
    let snapshot = sender.snapshot();
    *CONTROL.lock().unwrap() = Some(sender);
    Ok(snapshot.to_string())
}
pub fn control_state() -> Result<String> {
    Ok(CONTROL
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.snapshot())
        .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
        .to_string())
}
pub fn control_event(id: &str, event: &str) -> Result<String> {
    control_sender()?.enqueue(id, event)?;
    Ok("{}".into())
}
pub fn stop_control(id: &str) -> Result<String> {
    control_sender()?.cancel(id);
    Ok("{}".into())
}

pub fn configure_text_receiver(enabled: bool) -> Result<String> {
    receiver::shared().configure_text(enabled);
    Ok("{}".into())
}
pub fn configure_input_keyboard_available(enabled: bool) -> Result<String> {
    receiver::shared().configure_keyboard(enabled);
    Ok("{}".into())
}
pub fn configure_input_receiver(enabled: bool, width: f64, height: f64) -> Result<String> {
    receiver::shared().configure(enabled, width, height)?;
    Ok("{}".into())
}
pub fn poll_input_receiver() -> Result<String> {
    Ok(receiver::shared().drain().to_string())
}

pub fn start_display_sender(peer: &str, address: &str, config: &str) -> Result<String> {
    let _start = SENDER_START.lock().unwrap();
    let runtime = runtime()?;
    let old = SENDER.lock().unwrap().take();
    drop(old);
    let sender = Arc::new(sender::Sender::start_routed(
        runtime.identity.clone(),
        runtime.trust.clone(),
        peer.to_owned(),
        address.to_owned(),
        serde_json::from_str(config)?,
    )?);
    let state = sender.snapshot();
    *SENDER.lock().unwrap() = Some(sender);
    Ok(state.to_string())
}
pub fn configure_display_sender(id: &str, config: &str) -> Result<String> {
    let sender = SENDER
        .lock()
        .unwrap()
        .clone()
        .context("Capture is not active")?;
    sender.configure(id, serde_json::from_str(config)?)?;
    Ok(sender.snapshot().to_string())
}
pub fn display_sender_state() -> Result<String> {
    Ok(SENDER
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.snapshot())
        .unwrap_or_else(|| serde_json::json!({"state":"idle"}))
        .to_string())
}
/// Validates a capture frame before its bytes are copied across the boundary.
pub fn check_display_frame(timestamp: i64, len: usize) -> Result<()> {
    ensure!(timestamp > 0, "Invalid capture timestamp");
    ensure!(len <= FRAME_LIMIT, "Encoded frame too large");
    Ok(())
}
pub fn push_display_frame(
    id: &str,
    bytes: Vec<u8>,
    timestamp: i64,
    keyframe: bool,
) -> Result<String> {
    check_display_frame(timestamp, bytes.len())?;
    let sender = SENDER
        .lock()
        .unwrap()
        .clone()
        .context("Capture is not active")?;
    sender.enqueue(
        id,
        extend_computer_agent::display::Frame {
            timestamp: timestamp as u64,
            keyframe,
            bytes,
        },
    )?;
    Ok("{}".into())
}
pub fn stop_display_sender(id: &str) -> Result<String> {
    if let Some(sender) = SENDER.lock().unwrap().as_ref() {
        sender.cancel(id);
    }
    Ok("{}".into())
}
