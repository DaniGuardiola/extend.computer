//! Mac-side test peer for the iPhone app. Runs the same mobile engine API the
//! phone uses, with a disposable identity kept in `<state>` (never a desktop
//! user's identity or trust). Auto-approves every pairing request it sees and
//! prints the verification symbols so they can be compared with the phone.
//!
//! cargo run -p extend-ios-engine --example ios_peer -- <state> <command> ...
//!   identity                     print this peer's fingerprint
//!   incoming [secs]              open a pairing window (code + visual), advertised as _extendpair
//!   pair <addr> <code>           code-pair to the phone's incoming window
//!   visual <addr> [secs]         visual-pair to the phone's incoming window
//!   serve [secs] [bind-ip]       receiving + presence (_extendpresence); with bind-ip also
//!                                serves presence on <bind-ip>:48178 (SO_REUSEADDR) when the
//!                                wildcard port is held by another app
//!   presence <addr> <peer>       pinned presence query (addr host:port of the phone's presence)
//!   probe <addr> <peer>          pinned control probe
//!   peers                        paired peers + names
//!   forget <peer>                local unpair
//!   usb <udid> <peer>            connect through usbmuxd and query presence over the carrier
use anyhow::{bail, Context, Result};
use extend_android_engine::api;
use extend_computer_agent::{
    discovery::Advertisement,
    identity::Identity,
    session::{self, Presence},
    trust::TrustStore,
};
use std::{
    fs,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const NAME: &str = "Test Mac Peer";

fn seed(state: &Path) -> Result<Zeroizing<Vec<u8>>> {
    fs::create_dir_all(state)?;
    let path = state.join("seed");
    if let Ok(bytes) = fs::read(&path) {
        return Ok(Zeroizing::new(bytes));
    }
    let identity = Identity::generate();
    fs::write(&path, identity.secret())?;
    Ok(Zeroizing::new(identity.secret().to_vec()))
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let state = PathBuf::from(args.first().context("state dir required")?);
    let command = args.get(1).map(String::as_str).unwrap_or("identity");
    let arg = |i: usize| args.get(i + 2).cloned().context("missing argument");
    let secs = |i: usize, default: u64| args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(default);
    let secret = seed(&state)?;
    let root = fs::canonicalize(&state)?.join("engine");
    fs::create_dir_all(&root)?;
    let identity = json(&api::initialize(secret.clone(), root.to_str().unwrap(), || {
        api::DeviceKind::Laptop
    })?);
    let fingerprint = identity["fingerprint"].as_str().unwrap().to_owned();
    api::update_local_device_name(NAME)?;
    println!("PEER {fingerprint}");
    match command {
        "identity" => {}
        "incoming" => {
            let snapshot = json(&api::start_incoming(NAME)?);
            let port = snapshot["port"].as_u64().context("port")? as u16;
            println!("CODE {} PORT {port}", snapshot["code"].as_str().unwrap_or(""));
            let _ad = Advertisement::start_pairing(port, NAME, &fingerprint)
                .map_err(|e| eprintln!("advertise failed: {e:#}"))
                .ok();
            let deadline = Instant::now() + Duration::from_secs(secs(0, 120));
            let mut last = String::new();
            while Instant::now() < deadline {
                let state = json(&api::incoming_state()?);
                let line = state.to_string();
                if line != last {
                    println!("INCOMING {line}");
                    last = line;
                }
                if let Some(request) = state["request"].as_object() {
                    if let Some(id) = request.get("id").and_then(|v| v.as_str()) {
                        println!("ALLOW {} symbols={:?}", id, request.get("symbols"));
                        let _ = api::respond_incoming(id, true);
                    }
                }
                if state["state"] != "waiting" {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
            println!("NAMES {}", api::peer_names()?);
        }
        "pair" => {
            let peer = api::pair_code(&arg(0)?, &arg(1)?)?;
            println!("PAIRED {peer}");
        }
        "visual" => {
            let op = json(&api::start_visual_pairing(&arg(0)?, "iPhone", NAME)?);
            let id = op["id"].as_str().context("id")?.to_owned();
            let deadline = Instant::now() + Duration::from_secs(secs(1, 120));
            let mut answered = String::new();
            while Instant::now() < deadline {
                let state = json(&api::visual_pairing_state(&id)?);
                if let Some(request) = state["request"].as_object() {
                    let rid = request["id"].as_str().unwrap_or("").to_owned();
                    if rid != answered {
                        println!("ALLOW {rid} symbols={:?}", request.get("symbols"));
                        api::respond_visual_pairing(&id, &rid, true)?;
                        answered = rid;
                    }
                }
                let phase = state["state"].as_str().unwrap_or("").to_owned();
                if !["connecting", "verify"].contains(&phase.as_str()) {
                    println!("VISUAL {state}");
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
        "serve" => {
            let receiving = json(&api::start_receiving(NAME)?);
            let presence = json(&api::start_presence(NAME)?);
            let port = presence["port"].as_u64().context("port")? as u16;
            let receive_port = receiving["port"].as_u64().unwrap_or(48177) as u16;
            println!("RECEIVING {receiving} PRESENCE_PORT {port}");
            let _ad = Advertisement::start_presence(port)
                .map_err(|e| eprintln!("advertise failed: {e:#}"))
                .ok();
            if let Some(ip) = args.get(3) {
                let listener = reuse_listener(SocketAddr::new(ip.parse()?, 48178))?;
                let me = Identity::from_secret(secret.as_slice().try_into()?);
                let trust = TrustStore::open(root.join("trust"))?;
                println!("PRESENCE_ALSO {ip}:48178");
                thread::spawn(move || {
                    for socket in listener.incoming().flatten() {
                        let result = session::serve_presence(socket, &me, &trust, || Presence {
                            receiving: true,
                            port: receive_port,
                            name: NAME.into(),
                        });
                        println!("PRESENCE_SERVED {result:?}");
                    }
                });
            }
            thread::sleep(Duration::from_secs(secs(0, 300)));
            api::stop_presence()?;
            api::stop_receiving()?;
        }
        "presence" => println!("PRESENCE {}", api::resolve_presence(&arg(0)?, &arg(1)?)?),
        "probe" => println!("PROBE {}", api::probe_peer(&arg(0)?, &arg(1)?)?),
        "peers" => println!("PEERS {} NAMES {}", api::paired_peers()?, api::peer_names()?),
        "forget" => println!("FORGET {}", api::forget_peer(&arg(0)?)?),
        "usb" => usb(&arg(0)?, &arg(1)?, &secret)?,
        other => bail!("unknown command {other}"),
    }
    Ok(())
}

fn reuse_listener(address: SocketAddr) -> Result<TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

#[cfg(target_os = "macos")]
fn usb(udid: &str, peer: &str, secret: &[u8]) -> Result<()> {
    use extend_computer_agent::display::{usb_host::NativeUsbHost, usb_stream::UsbService};
    use std::sync::atomic::AtomicBool;
    let me = Identity::from_secret(secret.try_into()?);
    let host = NativeUsbHost::connect_usbmux(udid, &me.fingerprint())?;
    let hub = host.hub();
    hub.enable_streams()?;
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let socket = hub.open_stream(UsbService::Presence, &stop, Instant::now() + Duration::from_secs(5))?;
    let reply = session::query_presence_with_device_info(socket, &me, peer)?;
    let status = reply.as_ref().map(|r| &r.status);
    println!(
        "USB_PRESENCE receiving={:?} name={:?} port={:?} in {:?}",
        status.map(|s| s.receiving),
        status.map(|s| s.name.clone()),
        status.map(|s| s.port),
        started.elapsed()
    );
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn usb(_: &str, _: &str, _: &[u8]) -> Result<()> {
    bail!("usbmux host is macOS only")
}
