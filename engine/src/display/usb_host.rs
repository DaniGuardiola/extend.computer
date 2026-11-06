//! macOS native accessory host lifecycle. The libusb helper (Android) or
//! usbmuxd (iPhone) carries raw framed encrypted media over a private local
//! socket, never ADB or a network tunnel.
use super::{
    usb_link::{self, UsbHub},
    usbmux, Transport,
};
use crate::session::display::VideoCodec;
use anyhow::{ensure, Context, Result};
use std::{
    io::Write,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct NativeUsbHost {
    hub: Arc<UsbHub>,
    stopped: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
    worker: Option<JoinHandle<()>>,
    _helper: Option<Helper>,
    _directory: Option<tempfile::TempDir>,
}
struct Helper(Child);
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl NativeUsbHost {
    pub fn connect(
        helper: &Path,
        vendor_hex: &str,
        serial: &str,
        local_fingerprint: &str,
    ) -> Result<Self> {
        ensure!(
            local_fingerprint.len() == 64
                && local_fingerprint
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid accessory host identity"
        );
        let directory = tempfile::Builder::new()
            .prefix("extend-usb-")
            .tempdir_in("/tmp")?;
        let path = directory.path().join("carrier.sock");
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let mut helper = Helper(
            Command::new(helper)
                .args([vendor_hex, serial, local_fingerprint])
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .context("Could not start native USB helper")?,
        );
        let accepted = (|| -> Result<_> {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = helper.0.try_wait()? {
                    anyhow::bail!("USB accessory helper exited: {status}");
                }
                match listener.accept() {
                    Ok((stream, _)) => return Ok(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
                ensure!(
                    Instant::now() < deadline,
                    "USB accessory connection timed out"
                );
                thread::sleep(Duration::from_millis(20));
            }
        })();
        Self::start(accepted?, Some(helper), Some(directory))
    }
    /// iPhone carrier: usbmuxd tunnels to the app's loopback listener on
    /// `IOS_USB_PORT`. The host hello hands the device this host's pinned
    /// identity, as the AOA serial descriptor does for Android.
    pub fn connect_usbmux(udid: &str, local_fingerprint: &str) -> Result<Self> {
        Self::connect_usbmux_at(&usbmux::Usbmux::default(), udid, local_fingerprint)
    }
    pub fn connect_usbmux_at(
        mux: &usbmux::Usbmux,
        udid: &str,
        local_fingerprint: &str,
    ) -> Result<Self> {
        let hello = usbmux::host_hello(local_fingerprint)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            let device = mux
                .usb_devices()?
                .into_iter()
                .find(|device| device.udid == udid)
                .context("Exact USB device unavailable")?;
            // The app may still be starting its listener; refusals retry briefly.
            let error = match mux.connect(device.id, usbmux::IOS_USB_PORT) {
                Ok(stream) => break stream,
                Err(error) => error,
            };
            let refused = error
                .downcast_ref::<usbmux::MuxError>()
                .is_some_and(|e| e.code == usbmux::RESULT_CONNECTION_REFUSED);
            ensure!(
                refused,
                error.context("Could not reach the iPhone over USB")
            );
            ensure!(
                Instant::now() < deadline,
                "Open extend.computer on the iPhone, then try again"
            );
            thread::sleep(Duration::from_millis(250));
        };
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(&hello)?;
        stream.set_write_timeout(None)?;
        Self::start(stream, None, None)
    }
    fn start(
        stream: UnixStream,
        helper: Option<Helper>,
        directory: Option<tempfile::TempDir>,
    ) -> Result<Self> {
        stream.set_nonblocking(true)?;
        let writer = stream.try_clone()?;
        let (hub, driver) = UsbHub::new("192.0.2.1".parse()?, "192.0.2.2".parse()?)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = stopped.clone();
        let error = Arc::new(Mutex::new(String::new()));
        let worker_error = error.clone();
        let worker = thread::Builder::new()
            .name("extend-usb-carrier".into())
            .spawn(move || {
                let result = usb_link::run_io(driver, stream, writer, &worker_stop);
                if let Err(error) = result {
                    if !worker_stop.load(Ordering::Acquire) {
                        *worker_error.lock().unwrap() = error.to_string();
                    }
                }
                worker_stop.store(true, Ordering::Release);
            })?;
        Ok(Self {
            hub,
            stopped,
            error,
            worker: Some(worker),
            _helper: helper,
            _directory: directory,
        })
    }
    pub fn hub(&self) -> Arc<UsbHub> {
        self.hub.clone()
    }
    pub fn transport(&self, codec: VideoCodec) -> Result<Transport> {
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "USB accessory worker stopped"
        );
        Transport::with_packet_link(codec, Box::new(self.hub.endpoint()?))
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let (sent, received, unregistered) = self.hub.counters();
        serde_json::json!({"state":if self.stopped.load(Ordering::Acquire) {"stopped"} else {"connected"},
            "error":*self.error.lock().unwrap(),"sent_packets":sent,"received_packets":received,"unregistered_packets":unregistered,"queue_metrics":self.hub.queue_metrics()})
    }
}
impl Drop for NativeUsbHost {
    fn drop(&mut self) {
        self.hub.close();
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{usb_stream::UsbService, usbmux::*};
    use plist::{Dictionary, Value};
    use std::io::Read;

    fn result(number: i64) -> Dictionary {
        let mut body = Dictionary::new();
        body.insert("MessageType".into(), "Result".into());
        body.insert("Number".into(), number.into());
        body
    }

    #[test]
    fn usbmux_carrier_sends_host_hello_then_shares_the_accessory_framing() {
        const HOST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usbmuxd");
        let listener = UnixListener::bind(&path).unwrap();
        let (device_hub, driver) =
            UsbHub::new("192.0.2.2".parse().unwrap(), "192.0.2.1".parse().unwrap()).unwrap();
        device_hub.enable_streams().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let device_stop = stop.clone();
        // Fake usbmuxd: one refusal while the app starts, then the tunnel's
        // far end behaves like the iOS app's loopback listener.
        let device = thread::spawn(move || {
            let mut refused = false;
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let (tag, body) = read_message(&mut stream).unwrap();
                match body["MessageType"].as_string().unwrap() {
                    "ListDevices" => {
                        let mut properties = Dictionary::new();
                        properties.insert("SerialNumber".into(), "iphone-udid".into());
                        properties.insert("ProductID".into(), 0x12a8u64.into());
                        properties.insert("ConnectionType".into(), "USB".into());
                        let mut record = Dictionary::new();
                        record.insert("DeviceID".into(), 3u64.into());
                        record.insert("Properties".into(), properties.into());
                        let mut reply = Dictionary::new();
                        reply.insert("DeviceList".into(), Value::Array(vec![record.into()]));
                        write_message(&mut stream, tag, &reply).unwrap();
                    }
                    "Connect" if !refused => {
                        refused = true;
                        write_message(&mut stream, tag, &result(RESULT_CONNECTION_REFUSED))
                            .unwrap();
                    }
                    "Connect" => {
                        write_message(&mut stream, tag, &result(RESULT_OK)).unwrap();
                        let peer = read_host_hello(&mut stream).unwrap();
                        stream.set_nonblocking(true).unwrap();
                        let writer = stream.try_clone().unwrap();
                        let _ = usb_link::run_io(driver, stream, writer, &device_stop);
                        return peer;
                    }
                    other => panic!("unexpected {other}"),
                }
            }
        });
        let host =
            NativeUsbHost::connect_usbmux_at(&Usbmux::at(&path), "iphone-udid", HOST).unwrap();
        let hub = host.hub();
        hub.enable_streams().unwrap();
        let mut opened = hub
            .open_stream(
                UsbService::Presence,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(3),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut accepted = loop {
            if let Some((service, stream)) = device_hub.accept_stream().unwrap() {
                assert_eq!(service, UsbService::Presence);
                break stream;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        };
        accepted
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        opened.write_all(b"over-usbmux").unwrap();
        let mut bytes = [0; 11];
        accepted.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"over-usbmux");
        assert_eq!(host.snapshot()["state"], "connected");
        drop(host);
        stop.store(true, Ordering::Release);
        device_hub.close();
        assert_eq!(device.join().unwrap(), HOST);
    }

    #[test]
    fn usbmux_carrier_requires_the_exact_attached_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usbmuxd");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (tag, _) = read_message(&mut stream).unwrap();
            let mut reply = Dictionary::new();
            reply.insert("DeviceList".into(), Value::Array(vec![]));
            write_message(&mut stream, tag, &reply).unwrap();
        });
        let error = NativeUsbHost::connect_usbmux_at(
            &Usbmux::at(&path),
            "missing",
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("Exact USB device unavailable"));
        server.join().unwrap();
    }
}
