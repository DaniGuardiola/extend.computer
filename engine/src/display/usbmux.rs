//! iPhone USB carrier through Apple's usbmuxd. The iOS app listens on
//! `127.0.0.1:IOS_USB_PORT`; usbmuxd tunnels a desktop byte stream to it over
//! the cable. After a fixed host hello, that stream carries exactly the same
//! framed encrypted packets as the Android Open Accessory bulk pipe.
use anyhow::{ensure, Result};
use std::io::Read;

/// Device-side TCP port the iOS app listens on (loopback only). usbmuxd
/// reaches it over the cable; it is never exposed on a network interface.
pub const IOS_USB_PORT: u16 = 47_177;
/// USB vendor id the desktop USB inventory reports for usbmux devices.
pub const APPLE_USB_VENDOR: &str = "05ac";
/// First bytes the desktop writes on every usbmux carrier, followed by its
/// 64-hex identity fingerprint. AOA carries the same fingerprint in the
/// accessory serial descriptor; the device pins its paired peer with it.
pub const HOST_HELLO_MAGIC: [u8; 8] = *b"EXTUSB1\n";
pub const HOST_HELLO_LEN: usize = HOST_HELLO_MAGIC.len() + 64;

fn valid_fingerprint(value: &[u8]) -> bool {
    value.len() == 64
        && value
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub fn host_hello(fingerprint: &str) -> Result<[u8; HOST_HELLO_LEN]> {
    ensure!(
        valid_fingerprint(fingerprint.as_bytes()),
        "Invalid accessory host identity"
    );
    let mut hello = [0; HOST_HELLO_LEN];
    hello[..8].copy_from_slice(&HOST_HELLO_MAGIC);
    hello[8..].copy_from_slice(fingerprint.as_bytes());
    Ok(hello)
}

/// Device side: reads the host hello and returns the host fingerprint. Every
/// later byte on the stream belongs to the framed packet carrier.
pub fn read_host_hello(reader: &mut impl Read) -> Result<String> {
    let mut hello = [0; HOST_HELLO_LEN];
    reader.read_exact(&mut hello)?;
    ensure!(
        hello[..8] == HOST_HELLO_MAGIC && valid_fingerprint(&hello[8..]),
        "Invalid USB host hello"
    );
    Ok(String::from_utf8(hello[8..].to_vec())?)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use client::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod client {
    use anyhow::{bail, ensure, Context, Result};
    use plist::{Dictionary, Value};
    use std::{
        io::{self, Read, Write},
        os::unix::net::UnixStream,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU32, Ordering},
        time::Duration,
    };

    pub const SOCKET_PATH: &str = "/var/run/usbmuxd";
    const VERSION_PLIST: u32 = 1;
    const MESSAGE_PLIST: u32 = 8;
    const MAX_MESSAGE: usize = 1 << 20;
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
    static TAGS: AtomicU32 = AtomicU32::new(1);

    /// usbmuxd `Result` codes.
    pub const RESULT_OK: i64 = 0;
    pub const RESULT_BAD_DEVICE: i64 = 2;
    pub const RESULT_CONNECTION_REFUSED: i64 = 3;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MuxDevice {
        pub id: u32,
        /// Device UDID, as usbmuxd reports `SerialNumber`.
        pub udid: String,
        pub product_id: u16,
        /// `USB` or `Network` (Wi-Fi sync).
        pub connection: String,
        pub name: String,
    }
    impl MuxDevice {
        pub fn usb(&self) -> bool {
            self.connection == "USB"
        }
        fn from_record(id: Option<u64>, properties: &Dictionary) -> Option<Self> {
            let id = id.or_else(|| properties.get("DeviceID")?.as_unsigned_integer())?;
            let udid = properties.get("SerialNumber")?.as_string()?.to_owned();
            let product_id = properties
                .get("ProductID")
                .and_then(Value::as_unsigned_integer)
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(0);
            let connection = properties
                .get("ConnectionType")
                .and_then(Value::as_string)
                .unwrap_or("USB")
                .to_owned();
            Some(Self {
                id: u32::try_from(id).ok()?,
                udid,
                product_id,
                connection,
                name: product_name(product_id).into(),
            })
        }
    }
    /// usbmuxd does not report a user-visible device name without lockdown
    /// pairing records; the USB product id identifies the device family.
    pub fn product_name(product_id: u16) -> &'static str {
        match product_id {
            0x12a8 => "iPhone",
            0x12aa => "iPod touch",
            0x12ab => "iPad",
            _ => "Apple device",
        }
    }
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MuxEvent {
        Attached(MuxDevice),
        Detached(u32),
    }
    #[derive(Debug)]
    pub struct MuxError {
        pub code: i64,
    }
    impl std::fmt::Display for MuxError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self.code {
                RESULT_BAD_DEVICE => write!(f, "USB device is no longer connected"),
                RESULT_CONNECTION_REFUSED => write!(f, "Device refused the USB connection"),
                code => write!(f, "usbmuxd request failed ({code})"),
            }
        }
    }
    impl std::error::Error for MuxError {}

    pub fn write_message(stream: &mut impl Write, tag: u32, body: &Dictionary) -> Result<()> {
        let mut payload = Vec::new();
        Value::Dictionary(body.clone()).to_writer_xml(&mut payload)?;
        let length = u32::try_from(payload.len() + 16)?;
        let mut message = Vec::with_capacity(payload.len() + 16);
        for word in [length, VERSION_PLIST, MESSAGE_PLIST, tag] {
            message.extend_from_slice(&word.to_le_bytes());
        }
        message.extend_from_slice(&payload);
        stream.write_all(&message)?;
        Ok(())
    }
    pub fn read_message(stream: &mut impl Read) -> Result<(u32, Dictionary)> {
        let mut header = [0; 16];
        stream.read_exact(&mut header)?;
        let word = |i: usize| u32::from_le_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
        let length = word(0) as usize;
        ensure!(
            (16..=MAX_MESSAGE).contains(&length),
            "Invalid usbmuxd message length"
        );
        ensure!(
            word(1) == VERSION_PLIST && word(2) == MESSAGE_PLIST,
            "Unsupported usbmuxd message"
        );
        let mut payload = vec![0; length - 16];
        stream.read_exact(&mut payload)?;
        let body = Value::from_reader(io::Cursor::new(payload))?
            .into_dictionary()
            .context("usbmuxd message is not a dictionary")?;
        Ok((word(3), body))
    }
    fn request(kind: &str) -> Dictionary {
        let mut body = Dictionary::new();
        body.insert("MessageType".into(), kind.into());
        body.insert("ClientVersionString".into(), "extend.computer".into());
        body.insert("ProgName".into(), "extend.computer".into());
        body.insert("kLibUSBMuxVersion".into(), 3u64.into());
        body
    }
    fn result_code(body: &Dictionary) -> Option<i64> {
        (body.get("MessageType")?.as_string()? == "Result")
            .then(|| body.get("Number")?.as_signed_integer())
            .flatten()
    }

    pub struct Usbmux {
        path: PathBuf,
    }
    impl Default for Usbmux {
        fn default() -> Self {
            Self::at(SOCKET_PATH)
        }
    }
    impl Usbmux {
        pub fn at(path: impl AsRef<Path>) -> Self {
            Self {
                path: path.as_ref().into(),
            }
        }
        pub fn available(&self) -> bool {
            self.path.exists()
        }
        fn send(&self, body: &Dictionary) -> Result<(UnixStream, u32)> {
            let mut stream = UnixStream::connect(&self.path)
                .with_context(|| format!("Could not reach usbmuxd at {}", self.path.display()))?;
            stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
            stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
            let tag = TAGS.fetch_add(1, Ordering::Relaxed);
            write_message(&mut stream, tag, body)?;
            Ok((stream, tag))
        }
        fn reply(stream: &mut UnixStream, tag: u32) -> Result<Dictionary> {
            loop {
                let (reply_tag, body) = read_message(stream)?;
                if reply_tag == tag {
                    return Ok(body);
                }
            }
        }
        /// Every device usbmuxd currently knows, including Wi-Fi sync entries.
        pub fn devices(&self) -> Result<Vec<MuxDevice>> {
            let (mut stream, tag) = self.send(&request("ListDevices"))?;
            let body = Self::reply(&mut stream, tag)?;
            if let Some(code) = result_code(&body) {
                return Err(MuxError { code }.into());
            }
            let list = body
                .get("DeviceList")
                .and_then(Value::as_array)
                .context("usbmuxd returned no device list")?;
            Ok(list
                .iter()
                .filter_map(Value::as_dictionary)
                .filter_map(|record| {
                    let id = record.get("DeviceID").and_then(Value::as_unsigned_integer);
                    MuxDevice::from_record(id, record.get("Properties")?.as_dictionary()?)
                })
                .collect())
        }
        /// Cable-attached devices only.
        pub fn usb_devices(&self) -> Result<Vec<MuxDevice>> {
            let mut devices: Vec<_> = self.devices()?.into_iter().filter(MuxDevice::usb).collect();
            devices.sort_by(|a, b| a.udid.cmp(&b.udid));
            devices.dedup_by(|a, b| a.udid == b.udid);
            Ok(devices)
        }
        /// Subscribes to attach/detach events. usbmuxd first reports every
        /// device already attached.
        pub fn listen(&self) -> Result<Listener> {
            let (mut stream, tag) = self.send(&request("Listen"))?;
            let body = Self::reply(&mut stream, tag)?;
            match result_code(&body) {
                Some(RESULT_OK) => {}
                Some(code) => return Err(MuxError { code }.into()),
                None => bail!("usbmuxd did not accept Listen"),
            }
            // macOS rejects socket options once the peer has hung up; the
            // next read then reports that instead.
            let _ = stream.set_read_timeout(None);
            Ok(Listener { stream })
        }
        /// Opens a byte stream to `port` on the device's loopback interface.
        /// The returned socket is a raw tunnel; usbmuxd framing ends here.
        pub fn connect(&self, device_id: u32, port: u16) -> Result<UnixStream> {
            let mut body = request("Connect");
            body.insert("DeviceID".into(), u64::from(device_id).into());
            // usbmuxd expects the port in network byte order inside an integer.
            body.insert(
                "PortNumber".into(),
                u64::from(u16::from_ne_bytes(port.to_be_bytes())).into(),
            );
            let (mut stream, tag) = self.send(&body)?;
            let reply = Self::reply(&mut stream, tag)?;
            match result_code(&reply) {
                Some(RESULT_OK) => {}
                Some(code) => return Err(MuxError { code }.into()),
                None => bail!("usbmuxd did not answer Connect"),
            }
            let _ = stream.set_read_timeout(None);
            let _ = stream.set_write_timeout(None);
            Ok(stream)
        }
    }

    pub struct Listener {
        stream: UnixStream,
    }
    impl Listener {
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_read_timeout(timeout)
        }
        /// Blocks for the next attach or detach event; other notices
        /// (e.g. `Paired`) are skipped.
        pub fn next_event(&mut self) -> Result<MuxEvent> {
            loop {
                let (_, body) = read_message(&mut self.stream)?;
                let id = body.get("DeviceID").and_then(Value::as_unsigned_integer);
                match body.get("MessageType").and_then(Value::as_string) {
                    Some("Attached") => {
                        if let Some(device) = body
                            .get("Properties")
                            .and_then(Value::as_dictionary)
                            .and_then(|p| MuxDevice::from_record(id, p))
                        {
                            return Ok(MuxEvent::Attached(device));
                        }
                    }
                    Some("Detached") => {
                        if let Some(id) = id.and_then(|id| u32::try_from(id).ok()) {
                            return Ok(MuxEvent::Detached(id));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn host_hello_round_trips_and_rejects_malformed_identities() {
        let hello = host_hello(FINGERPRINT).unwrap();
        assert_eq!(hello.len(), HOST_HELLO_LEN);
        assert_eq!(read_host_hello(&mut &hello[..]).unwrap(), FINGERPRINT);
        assert!(host_hello(&FINGERPRINT.to_uppercase()).is_err());
        assert!(host_hello(&FINGERPRINT[1..]).is_err());
        let mut bad = hello;
        bad[0] ^= 1;
        assert!(read_host_hello(&mut &bad[..]).is_err());
        assert!(read_host_hello(&mut &hello[..40]).is_err());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    mod fake_usbmuxd {
        use super::super::*;
        use plist::{Dictionary, Value};
        use std::{
            io::{Read, Write},
            os::unix::net::{UnixListener, UnixStream},
            thread,
        };

        fn record(id: u64, udid: &str, product: u64, connection: &str) -> Value {
            let mut properties = Dictionary::new();
            properties.insert("DeviceID".into(), id.into());
            properties.insert("SerialNumber".into(), udid.into());
            properties.insert("ProductID".into(), product.into());
            properties.insert("ConnectionType".into(), connection.into());
            let mut record = Dictionary::new();
            record.insert("DeviceID".into(), id.into());
            record.insert("MessageType".into(), "Attached".into());
            record.insert("Properties".into(), properties.into());
            record.into()
        }
        fn result(number: i64) -> Dictionary {
            let mut body = Dictionary::new();
            body.insert("MessageType".into(), "Result".into());
            body.insert("Number".into(), number.into());
            body
        }
        /// One request per client connection, like real usbmuxd clients.
        fn serve(listener: UnixListener, clients: usize) -> thread::JoinHandle<Vec<Dictionary>> {
            thread::spawn(move || {
                let mut requests = Vec::new();
                for _ in 0..clients {
                    let (mut stream, _) = listener.accept().unwrap();
                    let (tag, body) = read_message(&mut stream).unwrap();
                    requests.push(body.clone());
                    match body.get("MessageType").unwrap().as_string().unwrap() {
                        "ListDevices" => {
                            let mut reply = Dictionary::new();
                            reply.insert(
                                "DeviceList".into(),
                                Value::Array(vec![
                                    record(7, "00008110-000269390E9A201E", 0x12a8, "USB"),
                                    record(8, "wifi-ipad", 0x12ab, "Network"),
                                ]),
                            );
                            write_message(&mut stream, tag, &reply).unwrap();
                        }
                        "Listen" => {
                            write_message(&mut stream, tag, &result(0)).unwrap();
                            let mut paired = Dictionary::new();
                            paired.insert("MessageType".into(), "Paired".into());
                            paired.insert("DeviceID".into(), 7u64.into());
                            write_message(&mut stream, 0, &paired).unwrap();
                            let Value::Dictionary(attached) = record(9, "ipad-udid", 0x12ab, "USB")
                            else {
                                unreachable!()
                            };
                            write_message(&mut stream, 0, &attached).unwrap();
                            let mut detached = Dictionary::new();
                            detached.insert("MessageType".into(), "Detached".into());
                            detached.insert("DeviceID".into(), 9u64.into());
                            write_message(&mut stream, 0, &detached).unwrap();
                        }
                        "Connect" => {
                            let port = body.get("PortNumber").unwrap().as_unsigned_integer();
                            let device = body.get("DeviceID").unwrap().as_unsigned_integer();
                            if device != Some(7) {
                                write_message(&mut stream, tag, &result(2)).unwrap();
                            } else if port
                                != Some(u64::from(u16::from_ne_bytes(IOS_USB_PORT.to_be_bytes())))
                            {
                                write_message(&mut stream, tag, &result(3)).unwrap();
                            } else {
                                write_message(&mut stream, tag, &result(0)).unwrap();
                                // Raw tunnel: echo bytes uppercased.
                                let mut bytes = [0; 5];
                                stream.read_exact(&mut bytes).unwrap();
                                stream.write_all(&bytes.to_ascii_uppercase()).unwrap();
                            }
                        }
                        other => panic!("unexpected request {other}"),
                    }
                }
                requests
            })
        }

        #[test]
        fn lists_usb_devices_and_tunnels_to_device_port_in_network_order() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("usbmuxd");
            let server = serve(UnixListener::bind(&path).unwrap(), 5);
            let mux = Usbmux::at(&path);
            assert!(mux.available());
            let all = mux.devices().unwrap();
            assert_eq!(all.len(), 2);
            let usb = mux.usb_devices().unwrap();
            assert_eq!(
                usb,
                vec![MuxDevice {
                    id: 7,
                    udid: "00008110-000269390E9A201E".into(),
                    product_id: 0x12a8,
                    connection: "USB".into(),
                    name: "iPhone".into(),
                }]
            );
            let mut tunnel: UnixStream = mux.connect(7, IOS_USB_PORT).unwrap();
            tunnel.write_all(b"hello").unwrap();
            let mut echoed = [0; 5];
            tunnel.read_exact(&mut echoed).unwrap();
            assert_eq!(&echoed, b"HELLO");
            let refused = mux.connect(7, IOS_USB_PORT + 1).unwrap_err();
            assert_eq!(
                refused.downcast_ref::<MuxError>().unwrap().code,
                RESULT_CONNECTION_REFUSED
            );
            let gone = mux.connect(99, IOS_USB_PORT).unwrap_err();
            assert_eq!(
                gone.downcast_ref::<MuxError>().unwrap().code,
                RESULT_BAD_DEVICE
            );
            let requests = server.join().unwrap();
            assert_eq!(
                requests[2].get("PortNumber").unwrap().as_unsigned_integer(),
                Some(0x49b8) // 47177 = 0xb849, byte-swapped
            );
            assert_eq!(
                requests[0]
                    .get("kLibUSBMuxVersion")
                    .unwrap()
                    .as_unsigned_integer(),
                Some(3)
            );
        }

        #[test]
        fn listen_reports_attach_and_detach_skipping_other_notices() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("usbmuxd");
            let server = serve(UnixListener::bind(&path).unwrap(), 1);
            let mut listener = Usbmux::at(&path).listen().unwrap();
            let MuxEvent::Attached(device) = listener.next_event().unwrap() else {
                panic!("expected attach");
            };
            assert_eq!((device.id, device.name.as_str()), (9, "iPad"));
            assert_eq!(listener.next_event().unwrap(), MuxEvent::Detached(9));
            server.join().unwrap();
            assert!(listener.next_event().is_err());
        }

        #[test]
        fn rejects_oversized_and_foreign_messages() {
            let mut header = Vec::new();
            for word in [u32::MAX, 1, 8, 1] {
                header.extend_from_slice(&word.to_le_bytes());
            }
            assert!(read_message(&mut &header[..]).is_err());
            let mut binary = Vec::new();
            for word in [16u32, 0, 1, 1] {
                binary.extend_from_slice(&word.to_le_bytes());
            }
            assert!(read_message(&mut &binary[..]).is_err());
            assert!(Usbmux::at("/nonexistent/usbmuxd").devices().is_err());
        }
    }
}
