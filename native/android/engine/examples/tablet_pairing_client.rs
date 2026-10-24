//! Ephemeral desktop-side client for the tablet's real incoming pairing UI.
use anyhow::{Context, Result};
use extend_computer_agent::{
    identity::Identity,
    session::{self, Client, Decision},
    trust::TrustStore,
};
use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("Private fixture JSON path required")?;
    let result_path = std::env::args()
        .nth(2)
        .context("Private receipt path required")?;
    let fixture: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let address: SocketAddr = fixture["address"]
        .as_str()
        .context("Address missing")?
        .parse()?;
    let code = fixture["code"].as_str().context("Code missing")?;
    let peer = fixture["peer"]
        .as_str()
        .context("Expected tablet identity missing")?;
    let scratch = tempfile::tempdir()?;
    let identity = Arc::new(Identity::generate());
    let trust = TrustStore::open(scratch.path())?;
    fs::write(
        &result_path,
        serde_json::json!({"status":"waiting","clientPeer":identity.fingerprint()}).to_string(),
    )?;
    let mut client = Client::connect(
        TcpStream::connect_timeout(&address, Duration::from_secs(5))?,
        &identity,
        &trust,
        Some(code),
        Some(peer),
        |_, _| Decision::Remember,
    )?;
    client.probe()?;
    client.close()?;
    let mut client = Client::connect(
        TcpStream::connect_timeout(&address, Duration::from_secs(5))?,
        &identity,
        &trust,
        None,
        Some(peer),
        |_, _| Decision::Deny,
    )?;
    client.probe()?;
    client.close()?;
    let local_name = "Incoming Test Mac";
    let tablet_name = session::exchange_device_name(
        TcpStream::connect_timeout(&address, Duration::from_secs(5))?,
        &identity,
        peer,
        local_name,
    )?;
    // Temporary return-route servers use the same pinned desktop protocols.
    // Ephemeral presence port exercises discovery, not just the default-port hint.
    let _return_advertisement = if fixture["receivingAddress"].is_string() {
        let receiver = TcpListener::bind("0.0.0.0:0")?;
        let receiver_port = receiver.local_addr()?.port();
        let receiver_identity = identity.clone();
        let receiver_trust = trust.clone();
        std::thread::spawn(move || {
            for socket in receiver.incoming().flatten() {
                let _ = socket.set_nonblocking(false);
                let _ = session::serve_connection(
                    socket,
                    &receiver_identity,
                    &receiver_trust,
                    &mut None,
                    |_, _| Decision::Deny,
                );
            }
        });
        let presence = TcpListener::bind("0.0.0.0:0")?;
        let presence_port = presence.local_addr()?.port();
        let presence_identity = identity.clone();
        let presence_trust = trust.clone();
        std::thread::spawn(move || {
            for socket in presence.incoming().flatten() {
                let _ = socket.set_nonblocking(false);
                let _ =
                    session::serve_presence(socket, &presence_identity, &presence_trust, || {
                        session::Presence {
                            receiving: true,
                            port: receiver_port,
                            name: "Incoming Test Mac".into(),
                        }
                    });
            }
        });
        Some(extend_computer_agent::discovery::Advertisement::start_presence(presence_port)?)
    } else {
        None
    };
    let mut checks = vec![
        "incoming-code-pair",
        "pinned-probe",
        "pinned-reconnect",
        "authenticated-name-exchange",
    ];
    if let Some(receiving) = fixture["receivingAddress"].as_str() {
        fs::write(&result_path, serde_json::json!({"status":"paired","clientPeer":identity.fingerprint(),"clientName":local_name,"tabletName":tablet_name}).to_string())?;
        println!("Paired. Waiting for background reconnect check.");
        let mut signal = String::new();
        std::io::stdin().read_line(&mut signal)?;
        anyhow::ensure!(
            signal.trim() == "probe",
            "Expected explicit background check signal"
        );
        let presence_address: SocketAddr = receiving.parse()?;
        let status = session::query_presence(
            TcpStream::connect_timeout(&presence_address, Duration::from_secs(5))?,
            &identity,
            peer,
        )?
        .context("Pinned availability missing")?;
        anyhow::ensure!(status.receiving, "Tablet receiving is off");
        let receiving = SocketAddr::new(presence_address.ip(), status.port);
        let mut client = Client::connect(
            TcpStream::connect_timeout(&receiving, Duration::from_secs(5))?,
            &identity,
            &trust,
            None,
            Some(peer),
            |_, _| Decision::Deny,
        )?;
        client.probe()?;
        client.close()?;
        let background_name = session::exchange_device_name(
            TcpStream::connect_timeout(&receiving, Duration::from_secs(5))?,
            &identity,
            peer,
            local_name,
        )?;
        anyhow::ensure!(
            background_name == tablet_name,
            "Tablet name changed unexpectedly"
        );
        anyhow::ensure!(
            session::exchange_device_name(
                TcpStream::connect_timeout(&receiving, Duration::from_secs(5))?,
                &Identity::generate(),
                peer,
                "Spoofed Mac"
            )
            .is_err(),
            "Unknown peer name was accepted"
        );
        anyhow::ensure!(
            session::query_presence(
                TcpStream::connect_timeout(&presence_address, Duration::from_secs(5))?,
                &Identity::generate(),
                peer
            )?
            .is_none(),
            "Unknown presence peer was accepted"
        );
        checks.extend([
            "background-pinned-reconnect",
            "background-authenticated-name",
            "unknown-peer-rejected",
        ]);
        fs::write(&result_path, serde_json::json!({"status":"background-passed","clientPeer":identity.fingerprint(),"clientName":local_name,"tabletName":tablet_name}).to_string())?;
        signal.clear();
        std::io::stdin().read_line(&mut signal)?;
        let restarted_presence: SocketAddr = signal.trim().parse()?;
        let repaired = session::query_presence(
            TcpStream::connect_timeout(&restarted_presence, Duration::from_secs(5))?,
            &identity,
            peer,
        )?
        .context("Restarted availability missing")?;
        anyhow::ensure!(repaired.receiving, "Restarted receiver is off");
        let restarted = SocketAddr::new(restarted_presence.ip(), repaired.port);
        let mut client = Client::connect(
            TcpStream::connect_timeout(&restarted, Duration::from_secs(5))?,
            &identity,
            &trust,
            None,
            Some(peer),
            |_, _| Decision::Deny,
        )?;
        client.probe()?;
        client.close()?;
        checks.push("pinned-reconnect-after-tablet-restart");
        checks.push("pinned-presence-port-repair");
        fs::write(&result_path, serde_json::json!({"status":"return-route-ready","clientPeer":identity.fingerprint(),"clientName":local_name,"tabletName":tablet_name}).to_string())?;
        signal.clear();
        std::io::stdin().read_line(&mut signal)?;
        anyhow::ensure!(
            signal.trim() == "route-checked",
            "Expected physical UI return-route check"
        );
        checks.push("android-ui-return-route-repair");
    }
    fs::write(result_path, serde_json::json!({"status":"passed","clientPeer":identity.fingerprint(),"clientName":local_name,"tabletName":tablet_name,"checks":checks}).to_string())?;
    println!("PASS: tablet incoming pairing and pinned reconnect.");
    Ok(())
}
