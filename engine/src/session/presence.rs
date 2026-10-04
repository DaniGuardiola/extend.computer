//! Separate read-only protocol: no pairing, input, unpair, or consent operations.
use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub receiving: bool,
    pub port: u16,
    pub name: String,
}
impl Presence {
    fn validate(&self) -> Result<()> {
        validate_device_name(&self.name)?;
        ensure!(self.port != 0, "invalid receiving port");
        Ok(())
    }
}
pub fn serve_presence(
    mut stream: TcpStream,
    identity: &Identity,
    store: &TrustStore,
    status: impl FnOnce() -> Presence,
) -> Result<()> {
    wire::configure(&stream)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut hello = [0; 9];
    wire::read_exact_until(&mut stream, &mut hello, deadline)?;
    ensure!(
        &hello[..8] == MAGIC && hello[8] == b'H',
        EngineError::ProtocolIncompatible
    );
    let (mut channel, peer, _) = handshake(
        stream,
        identity,
        None,
        false,
        deadline,
        b"extend.computer/v1/presence",
        None,
    )?;
    if store.peer(&peer).ok().flatten().is_none() {
        channel.send(&Message::NotPaired)?;
        return Ok(());
    }
    channel.send(&Message::Ready)?;
    ensure!(
        matches!(channel.receive()?, Message::QueryPresence),
        EngineError::RequestRejected
    );
    ensure!(store.peer(&peer)?.is_some(), EngineError::RequestRejected);
    let status = status();
    status.validate()?;
    channel.send(&Message::Presence(status))
}
/// Only a pinned response can establish availability or change a cached endpoint.
/// None means an authenticated peer explicitly no longer recognizes this pairing.
pub fn query_presence(
    mut stream: TcpStream,
    identity: &Identity,
    expected: &str,
) -> Result<Option<Presence>> {
    wire::configure(&stream)?;
    stream.write_all(MAGIC)?;
    stream.write_all(b"H")?;
    let (mut channel, peer, _) = handshake(
        stream,
        identity,
        None,
        true,
        Instant::now() + Duration::from_secs(3),
        b"extend.computer/v1/presence",
        Some(expected),
    )?;
    ensure!(peer == expected, EngineError::PeerIdentityChanged);
    match channel.receive()? {
        Message::NotPaired => return Ok(None),
        Message::Ready => {}
        _ => bail!(EngineError::RequestRejected),
    }
    channel.send(&Message::QueryPresence)?;
    let Message::Presence(status) = channel.receive()? else {
        bail!(EngineError::RequestRejected)
    };
    status.validate()?;
    Ok(Some(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn exchange(known: bool, correct_pin: bool) {
        let server = Identity::generate();
        let client = Identity::generate();
        let root = tempfile::tempdir().unwrap();
        let store = TrustStore::open(root.path()).unwrap();
        if known {
            store.remember(&client.fingerprint()).unwrap();
        }
        let before = std::fs::read(root.path().join("trust.json")).ok();
        let expected = if correct_pin {
            server.fingerprint()
        } else {
            Identity::generate().fingerprint()
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            serve_presence(listener.accept().unwrap().0, &server, &store, || {
                assert!(known);
                Presence {
                    receiving: false,
                    port: 48177,
                    name: "Remote".into(),
                }
            })
        });
        let result = query_presence(TcpStream::connect(addr).unwrap(), &client, &expected);
        let _ = thread.join().unwrap();
        if !correct_pin {
            assert!(result.is_err());
        } else if known {
            let status = result.unwrap().unwrap();
            assert!(!status.receiving);
            assert_eq!(status.name, "Remote");
        } else {
            assert!(result.unwrap().is_none());
        }
        assert_eq!(before, std::fs::read(root.path().join("trust.json")).ok());
    }
    #[test]
    fn trusted_status_is_read_only() {
        exchange(true, true);
    }
    #[test]
    fn unknown_caller_gets_no_metadata() {
        exchange(false, true);
    }
    #[test]
    fn address_reuse_cannot_impersonate_peer() {
        exchange(true, false);
    }
    #[test]
    fn presence_rejects_control_pairing_and_unpair_requests() {
        for request in [
            Message::RequestProbe,
            Message::RequestControl {
                remembered_only: false,
            },
            Message::RequestUnpair,
        ] {
            let server = Identity::generate();
            let client = Identity::generate();
            let root = tempfile::tempdir().unwrap();
            let store = TrustStore::open(root.path()).unwrap();
            store.remember(&client.fingerprint()).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let worker = std::thread::spawn(move || {
                serve_presence(listener.accept().unwrap().0, &server, &store, || {
                    panic!("unauthorized operation reached metadata callback")
                })
            });
            let mut socket = TcpStream::connect(addr).unwrap();
            wire::configure(&socket).unwrap();
            socket.write_all(MAGIC).unwrap();
            socket.write_all(b"H").unwrap();
            let (mut channel, _, _) = handshake(
                socket,
                &client,
                None,
                true,
                Instant::now() + Duration::from_secs(3),
                b"extend.computer/v1/presence",
                None,
            )
            .unwrap();
            assert!(matches!(channel.receive().unwrap(), Message::Ready));
            channel.send(&request).unwrap();
            assert!(worker.join().unwrap().is_err());
        }
    }
}
