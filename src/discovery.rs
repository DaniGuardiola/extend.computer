use anyhow::Result;
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use rand::{rngs::OsRng, RngCore};
use std::time::{Duration, Instant};

pub const PRESENCE_SERVICE: &str = "_extendpresence._tcp.local.";

pub const PAIRING_SERVICE: &str = "_extendpair._tcp.local.";

const SERVICE: &str = "_extendprobe._tcp.local.";

pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    pub fn start(port: u16) -> Result<Self> {
        Self::start_named(port, "extend.computer")
    }
    pub fn start_named(port: u16, device_name: &str) -> Result<Self> {
        Self::start_service(SERVICE, port, device_name, "probe", "")
    }
    pub fn start_presence(port: u16) -> Result<Self> {
        Self::start_service(PRESENCE_SERVICE, port, "", "presence", "")
    }
    pub fn start_pairing(port: u16, device_name: &str, discovery_id: &str) -> Result<Self> {
        Self::start_service(PAIRING_SERVICE, port, device_name, "pairing", discovery_id)
    }
    fn start_service(
        service: &str,
        port: u16,
        device_name: &str,
        capability: &str,
        discovery_id: &str,
    ) -> Result<Self> {
        let mut id = [0; 6];
        OsRng.fill_bytes(&mut id);
        let name = format!("extend-computer-{}", hex::encode(id));
        let info = ServiceInfo::new(
            service,
            &name,
            &format!("{name}.local."),
            "",
            port,
            &[
                ("version", "1"),
                ("capability", capability),
                ("discovery_id", discovery_id),
                ("name", device_name),
            ][..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        let daemon = ServiceDaemon::new()?;
        // This prototype listener is IPv4-only; do not advertise unreachable IPv6 endpoints.
        daemon.disable_interface(IfKind::IPv6)?;
        daemon.register(info)?;
        Ok(Self { daemon, fullname })
    }
}
impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

pub fn discover(duration: Duration) -> Result<()> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE)?;
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if let Ok(ServiceEvent::ServiceResolved(info)) =
            receiver.recv_timeout(Duration::from_millis(200))
        {
            // Discovery data is untrusted. Debug formatting escapes control characters.
            println!(
                "candidate {:?} addresses={:?} port={} (unverified)",
                info.get_fullname(),
                info.get_addresses(),
                info.get_port()
            );
        }
    }
    daemon.stop_browse(SERVICE)?;
    let _ = daemon.shutdown();
    Ok(())
}
