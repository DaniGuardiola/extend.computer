//! macOS native accessory host lifecycle. The libusb helper carries raw framed
//! encrypted media over a private local socket, never ADB or a network tunnel.
use super::{
    usb_link::{self, UsbHub},
    Transport,
};
use crate::session::display::VideoCodec;
use anyhow::{ensure, Context, Result};
use std::{
    os::unix::{fs::PermissionsExt, net::UnixListener},
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
    _helper: Helper,
    _directory: tempfile::TempDir,
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
        let stream = accepted?;
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
