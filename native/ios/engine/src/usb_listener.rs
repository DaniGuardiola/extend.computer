//! iPhone USB carrier. The desktop reaches the app through usbmuxd, which
//! tunnels to this loopback listener on `IOS_USB_PORT`. Each connection starts
//! with the fixed host hello (the host's pinned fingerprint, like the Android
//! accessory serial), then carries the same framed packets as the Android
//! accessory. Only one carrier is attached at a time; a newer connection from
//! a paired host replaces the previous one (cable re-plug, desktop restart).
use anyhow::{ensure, Context, Result};
use extend_android_engine::api;
use extend_computer_agent::display::usbmux;
use std::{
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

struct Listener {
    port: u16,
    stop: Arc<AtomicBool>,
    bound: Arc<AtomicBool>,
}
struct Carrier {
    id: String,
    peer: String,
    stream: TcpStream,
}
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);
static CARRIER: Mutex<Option<Carrier>> = Mutex::new(None);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn note(error: impl std::fmt::Display) {
    *LAST_ERROR.lock().unwrap() = error.to_string();
}

/// Starts listening on `127.0.0.1:port` (idempotent). Returns `{"port":..}`.
/// The app and its screen sharing extension hand the port to each other, so a
/// port still held by the other process is retried until it frees up or
/// `stop` is called; `state` reports `bound` once the socket is listening.
pub fn start(port: u16) -> Result<String> {
    let mut slot = LISTENER.lock().unwrap();
    if let Some(listener) = slot.as_ref() {
        if listener.port == port && !listener.stop.load(Ordering::Acquire) {
            return Ok(serde_json::json!({ "port": port }).to_string());
        }
        listener.stop.store(true, Ordering::Release);
    }
    ensure!(port != 0, "Invalid USB port");
    let socket = match bind(port) {
        Ok(socket) => Some(socket),
        Err(error) if error.kind() == ErrorKind::AddrInUse => None,
        Err(error) => {
            return Err(error).with_context(|| format!("Couldn't listen for USB on port {port}"))
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let bound = Arc::new(AtomicBool::new(socket.is_some()));
    let worker_stop = stop.clone();
    let worker_bound = bound.clone();
    thread::Builder::new()
        .name("extend-usbmux-accept".into())
        .spawn(move || {
            let socket = match socket {
                Some(socket) => socket,
                None => loop {
                    thread::sleep(BIND_RETRY);
                    if worker_stop.load(Ordering::Acquire) {
                        return;
                    }
                    match bind(port) {
                        Ok(socket) => break socket,
                        Err(error) if error.kind() == ErrorKind::AddrInUse => {}
                        Err(error) => {
                            note(format!("Couldn't listen for USB on port {port}: {error}"));
                            release(&worker_stop);
                            return;
                        }
                    }
                },
            };
            worker_bound.store(true, Ordering::Release);
            accept_loop(socket, worker_stop)
        })?;
    *slot = Some(Listener { port, stop, bound });
    Ok(serde_json::json!({ "port": port }).to_string())
}

const BIND_RETRY: Duration = Duration::from_millis(100);

fn bind(port: u16) -> std::io::Result<TcpListener> {
    // usbmuxd connects to the device's loopback interface; never expose the
    // carrier on Wi-Fi.
    let socket = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// Stops listening and detaches the current carrier. Safe when stopped.
pub fn stop() -> Result<String> {
    if let Some(listener) = LISTENER.lock().unwrap().take() {
        listener.stop.store(true, Ordering::Release);
    }
    if let Some(carrier) = CARRIER.lock().unwrap().take() {
        retire(&carrier);
    }
    Ok("{}".into())
}

/// `{"listening":bool,"bound":bool,"port":..,"carrier":{"id","peer"}|null,"error":".."}`.
pub fn state() -> Result<String> {
    let (port, bound) = LISTENER
        .lock()
        .unwrap()
        .as_ref()
        .map_or((None, false), |l| {
            (Some(l.port), l.bound.load(Ordering::Acquire))
        });
    let carrier = CARRIER
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| serde_json::json!({ "id": c.id, "peer": c.peer }));
    Ok(serde_json::json!({
        "listening": port.is_some(),
        "bound": bound,
        "port": port.unwrap_or(0),
        "carrier": carrier,
        "error": *LAST_ERROR.lock().unwrap(),
    })
    .to_string())
}

fn retire(carrier: &Carrier) {
    let _ = api::stop_usb(&carrier.id);
    let _ = carrier.stream.shutdown(Shutdown::Both);
}

fn accept_loop(socket: TcpListener, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match socket.accept() {
            Ok((stream, _)) => {
                let _ = thread::Builder::new()
                    .name("extend-usbmux-carrier".into())
                    .spawn(move || {
                        if let Err(error) = serve(stream) {
                            note(format!("{error:#}"));
                        }
                    });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                // iOS can revoke the socket while suspended; the app restarts
                // the listener when it becomes active again.
                note(format!("USB listener stopped: {error}"));
                break;
            }
        }
    }
    release(&stop);
}

fn release(stop: &Arc<AtomicBool>) {
    let mut slot = LISTENER.lock().unwrap();
    if slot.as_ref().is_some_and(|l| Arc::ptr_eq(&l.stop, stop)) {
        *slot = None;
    }
}

fn serve(mut stream: TcpStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    // A stray local connection must not hold a worker forever.
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let peer = usbmux::read_host_hello(&mut stream)?;
    stream.set_read_timeout(None)?;
    // Unknown hosts are refused before any carrier exists (attach checks trust).
    let previous = CARRIER.lock().unwrap().take();
    if let Some(previous) = previous {
        retire(&previous);
    }
    let attached: serde_json::Value = serde_json::from_str(&api::attach_usb(&peer)?)?;
    let id = attached["id"]
        .as_str()
        .context("USB carrier unavailable")?
        .to_owned();
    *CARRIER.lock().unwrap() = Some(Carrier {
        id: id.clone(),
        peer,
        stream: stream.try_clone()?,
    });
    note("");
    let writer = stream.try_clone()?;
    let write_id = id.clone();
    let write = thread::Builder::new()
        .name("extend-usbmux-write".into())
        .spawn(move || write_loop(writer, &write_id))?;
    let result = read_loop(&mut stream, &id);
    finish(&id);
    let _ = write.join();
    result
}

fn read_loop(stream: &mut TcpStream, id: &str) -> Result<()> {
    let mut bytes = [0; api::USB_READ_LIMIT];
    loop {
        let length = match stream.read(&mut bytes) {
            Ok(0) => return Ok(()), // host closed the tunnel
            Ok(length) => length,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        let mut offset = 0;
        while offset < length {
            let count = api::receive_usb(id, &bytes[offset..length])?;
            ensure!(count <= length - offset, "USB framing failed");
            if count == 0 {
                thread::sleep(Duration::from_millis(1));
            } else {
                offset += count;
            }
        }
    }
}

fn write_loop(mut stream: TcpStream, id: &str) {
    loop {
        match api::next_usb_frame(id) {
            Ok(Some(frame)) => {
                if stream.write_all(&frame).is_err() {
                    break;
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(1)),
            Err(_) => break, // carrier retired
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn finish(id: &str) {
    let mut slot = CARRIER.lock().unwrap();
    if slot.as_ref().is_some_and(|c| c.id == id) {
        if let Some(carrier) = slot.take() {
            retire(&carrier);
        }
    } else {
        // Already replaced: still stop this epoch so its writer exits.
        let _ = api::stop_usb(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listener is process-global.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn unknown_host_hello_is_rejected_before_attaching() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Engine not initialized: attach must fail and leave no carrier.
        let port = {
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        start(port).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let fingerprint = "ab".repeat(32);
        client
            .write_all(&usbmux::host_hello(&fingerprint).unwrap())
            .unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut byte = [0; 1];
        // The device closes the tunnel instead of attaching.
        assert!(matches!(client.read(&mut byte), Ok(0) | Err(_)));
        assert!(CARRIER.lock().unwrap().is_none());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let state = loop {
            let state: serde_json::Value = serde_json::from_str(&state().unwrap()).unwrap();
            if !state["error"].as_str().unwrap().is_empty() || std::time::Instant::now() > deadline
            {
                break state;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(state["listening"], true);
        assert!(!state["error"].as_str().unwrap().is_empty());
        stop().unwrap();
        let state: serde_json::Value = serde_json::from_str(&super::state().unwrap()).unwrap();
        assert_eq!(state["listening"], false);
    }

    #[test]
    fn a_port_held_by_the_other_process_is_taken_once_released() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The app's listener while the extension starts (or the reverse).
        let holder = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        start(port).unwrap();
        let bound = || {
            let state: serde_json::Value = serde_json::from_str(&state().unwrap()).unwrap();
            assert_eq!(state["listening"], true);
            state["bound"].as_bool().unwrap()
        };
        thread::sleep(BIND_RETRY * 3);
        assert!(!bound());
        drop(holder);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !bound() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
        stop().unwrap();
        // Stopping while still waiting gives the port up without binding it.
        let holder = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        start(port).unwrap();
        stop().unwrap();
        drop(holder);
        thread::sleep(BIND_RETRY * 3);
        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn malformed_hello_is_dropped() {
        let mut bad = usbmux::host_hello(&"cd".repeat(32)).unwrap();
        bad[0] ^= 1;
        assert!(usbmux::read_host_hello(&mut &bad[..]).is_err());
    }
}
