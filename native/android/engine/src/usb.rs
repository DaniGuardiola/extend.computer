//! Native accessory routes. A carrier is selected before any LAN hint is parsed;
//! its private streams still authenticate the exact paired Noise identity.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::{packet_link::PacketDriver, usb_link::UsbHub, usb_stream::UsbService, Transport},
    identity::Identity,
    session::{self, display::VideoCodec, Client, Decision, Presence},
    trust::TrustStore,
};
use std::{
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
static IDS: AtomicU64 = AtomicU64::new(1);
static CONNECTION: Mutex<Option<Connection>> = Mutex::new(None);
struct Connection {
    route: Route,
    driver: PacketDriver,
}
#[derive(Clone)]
pub(super) struct Route {
    pub(super) id: String,
    pub(super) peer: String,
    hub: Arc<UsbHub>,
    trust: TrustStore,
    commit: Arc<Mutex<()>>,
}
impl Route {
    pub fn current(&self) -> bool {
        let same = CONNECTION.lock().unwrap().as_ref().is_some_and(|c| {
            c.route.id == self.id
                && c.route.peer == self.peer
                && Arc::ptr_eq(&c.route.hub, &self.hub)
        });
        same && self.trust.peer(&self.peer).ok().flatten().is_some()
    }
    pub fn open(
        &self,
        service: UsbService,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<TcpStream> {
        ensure!(self.current(), "USB connection changed");
        let socket = self.hub.open_stream(service, stop, deadline)?;
        ensure!(
            self.current() && !stop.load(Ordering::Acquire),
            "USB connection changed"
        );
        Ok(socket)
    }
    pub fn transport(&self, codec: VideoCodec) -> Result<Transport> {
        ensure!(self.current(), "USB connection changed");
        Ok(Transport::with_packet_link(
            codec,
            Box::new(self.hub.endpoint()?),
        )?)
    }
    fn save_name(&self, names: &crate::names::Names, name: &str) -> Result<()> {
        // Retirement waits for this metadata-only barrier before unpair cleanup.
        // Neither carrier I/O nor a network operation holds it.
        let _commit = self.commit.lock().unwrap();
        ensure!(self.current(), "USB connection changed");
        names.save(&self.peer, name)
    }
}
pub fn route(peer: &str) -> Result<Option<Route>> {
    let route = CONNECTION
        .lock()
        .unwrap()
        .as_ref()
        .filter(|c| c.route.peer == peer)
        .map(|c| c.route.clone());
    if let Some(route) = &route {
        ensure!(route.current(), "Device access expired");
    }
    Ok(route)
}
pub fn selected(peer: &str, address: &str) -> Result<Option<Route>> {
    let route = route(peer)?;
    if let Some(expected) = address.strip_prefix("usb:") {
        ensure!(
            !expected.is_empty()
                && expected.len() <= 20
                && expected.bytes().all(|b| b.is_ascii_digit()),
            "Invalid USB carrier"
        );
        ensure!(
            route.as_ref().is_some_and(|route| route.id == expected),
            "USB connection changed"
        );
    }
    Ok(route)
}
pub fn attach(
    peer: String,
    identity: Arc<Identity>,
    trust: TrustStore,
    names: Arc<crate::names::Names>,
) -> Result<String> {
    ensure!(
        trust.peer(&peer)?.is_some(),
        "Pair this USB device before connecting"
    );
    let mut connection = CONNECTION.lock().unwrap();
    ensure!(connection.is_none(), "USB accessory is already connected");
    let (hub, driver) = UsbHub::new("192.0.2.2".parse()?, "192.0.2.1".parse()?)?;
    hub.enable_streams()?;
    let id = IDS.fetch_add(1, Ordering::Relaxed).to_string();
    let route = Route {
        id: id.clone(),
        peer,
        hub,
        trust: trust.clone(),
        commit: Arc::new(Mutex::new(())),
    };
    *connection = Some(Connection {
        route: route.clone(),
        driver,
    });
    drop(connection);
    let worker_route = route.clone();
    if let Err(error) = thread::Builder::new()
        .name("extend-usb-services".into())
        .spawn(move || {
            while worker_route.current() {
                match worker_route.hub.accept_stream() {
                    Ok(Some((service, socket))) => {
                        if worker_route.current() {
                            let _ = incoming(
                                socket,
                                service,
                                &worker_route,
                                identity.clone(),
                                trust.clone(),
                                names.clone(),
                            );
                        }
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(2)),
                    Err(_) => break,
                }
            }
            stop(&worker_route.id);
        })
    {
        stop(&id);
        return Err(error.into());
    }
    Ok(id)
}
fn receiving() -> bool {
    crate::RECEIVING
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|s| s.snapshot()["state"] != "closed")
}
fn incoming(
    socket: TcpStream,
    service: UsbService,
    route: &Route,
    identity: Arc<Identity>,
    trust: TrustStore,
    names: Arc<crate::names::Names>,
) -> Result<()> {
    ensure!(route.current(), "USB connection changed");
    let expected = route.peer.clone();
    match service {
        UsbService::Presence => {
            let guard = route.clone();
            thread::Builder::new()
                .name("extend-usb-presence".into())
                .spawn(move || {
                    if guard.current() {
                        let _ = session::serve_presence_pinned(
                            socket,
                            &identity,
                            &trust,
                            &expected,
                            &|| Presence {
                                receiving: guard.current() && receiving(),
                                port: 48177,
                                name: names.local("Android device"),
                            },
                        );
                    }
                })?;
        }
        UsbService::Control => {
            crate::RECEIVING
                .lock()
                .unwrap()
                .as_ref()
                .context("Receiving is off")?
                .accept_pinned(socket, identity, trust, names, &expected)?;
        }
        UsbService::Display => {
            crate::MEDIA
                .lock()
                .unwrap()
                .clone()
                .context("Receiving is off")?
                .accept_usb(socket, identity, trust, route.clone())?;
        }
    }
    Ok(())
}
pub fn stop(id: &str) {
    let retired = {
        let mut connection = CONNECTION.lock().unwrap();
        if connection.as_ref().is_some_and(|c| c.route.id == id) {
            connection.take()
        } else {
            None
        }
    };
    if let Some(c) = retired {
        // Remove the carrier first, then drain any already admitted name save.
        // A later save sees the retired epoch without blocking packet I/O.
        drop(c.route.commit.lock().unwrap());
        c.route.hub.close();
    }
}
pub fn receive(id: &str, bytes: &[u8]) -> Result<usize> {
    let mut connection = CONNECTION.lock().unwrap();
    let connection = connection
        .as_mut()
        .filter(|c| c.route.id == id)
        .context("USB accessory disconnected")?;
    Ok(connection.driver.receive_bytes(bytes)?)
}
pub fn outgoing(id: &str) -> Result<Option<Vec<u8>>> {
    let mut connection = CONNECTION.lock().unwrap();
    let connection = connection
        .as_mut()
        .filter(|c| c.route.id == id)
        .context("USB accessory disconnected")?;
    Ok(connection.driver.take_outgoing()?)
}
pub fn snapshot() -> serde_json::Value {
    let connection = CONNECTION.lock().unwrap();
    match connection.as_ref() {
        None => serde_json::json!({"state":"disconnected"}),
        Some(c) => {
            let (sent, received, unregistered) = c.route.hub.counters();
            serde_json::json!({"state":"connected","id":c.route.id,"peer":c.route.peer,"streams_ready":c.route.hub.streams_ready(),"sent_packets":sent,"received_packets":received,"unregistered_packets":unregistered,"queue_metrics":c.route.hub.queue_metrics()})
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolveUsbPresence(
    env: jni::JNIEnv,
    _: jni::objects::JClass,
    peer: jni::objects::JString,
    id: jni::objects::JString,
) -> jni::sys::jstring {
    crate::boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let id: String = env.get_string(&id)?.into();
        let r = crate::RUNTIME.get().context("Engine is not initialized")?;
        let route = route(&peer)?.context("USB signaling unavailable")?;
        ensure!(route.id == id, "USB connection changed");
        let socket = route.open(
            UsbService::Presence,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )?;
        let status = session::query_presence(socket, &r.identity, &peer)?
            .context("This device no longer recognizes the pairing")?;
        route.save_name(&r.names, &status.name)?;
        Ok(serde_json::json!({"peer":peer,"name":status.name,"receiving":status.receiving,"route":"usb","carrier_id":route.id}).to_string())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probeUsbPeer(
    env: jni::JNIEnv,
    _: jni::objects::JClass,
    peer: jni::objects::JString,
    id: jni::objects::JString,
) -> jni::sys::jstring {
    crate::boundary(env, |env| {
        let peer: String = env.get_string(&peer)?.into();
        let id: String = env.get_string(&id)?.into();
        let r = crate::RUNTIME.get().context("Engine is not initialized")?;
        let route = route(&peer)?.context("USB signaling unavailable")?;
        ensure!(route.id == id, "USB connection changed");
        let socket = route.open(
            UsbService::Control,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(3),
        )?;
        let mut client =
            Client::connect(socket, &r.identity, &r.trust, None, Some(&peer), |_, _| {
                Decision::Deny
            })?;
        let latency = client.probe()?.as_secs_f64() * 1000.;
        client.close()?;
        ensure!(route.current(), "USB connection changed");
        Ok(
            serde_json::json!({"peer":peer,"rtt_ms":latency,"route":"usb","carrier_id":route.id})
                .to_string(),
        )
    })
}
#[cfg(test)]
#[path = "usb_tests.rs"]
mod tests;
