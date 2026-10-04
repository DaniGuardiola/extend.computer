//! Diagnostic integration driver. Uses an ephemeral key and never grants input/video access.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    identity::Identity,
    session::{Client, Decision},
    trust::TrustStore,
};
use std::{
    io,
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
use zeroize::Zeroizing;

fn main() -> Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .context("usage: peer_smoke HOST:PORT")?;
    let addresses: Vec<_> = endpoint.to_socket_addrs()?.collect();
    ensure!(!addresses.is_empty(), "no address");
    let dial = || {
        let mut last = None;
        for address in &addresses {
            match TcpStream::connect_timeout(address, Duration::from_secs(5)) {
                Ok(stream) => return Ok(stream),
                Err(error) => last = Some(error),
            }
        }
        Err(last.expect("nonempty addresses"))
    };
    let identity = Identity::generate();
    let dir = tempfile::tempdir()?;
    let store = TrustStore::open(dir.path())?;
    let mut code = Zeroizing::new(String::new());
    io::stdin().read_line(&mut code)?;
    println!("CLIENT_ID={}", identity.fingerprint());
    let mut client = Client::connect(
        dial()?,
        &identity,
        &store,
        Some(code.trim()),
        None,
        |_, _| Decision::AutomaticProbe,
    )?;
    let peer = client.peer().to_owned();
    let sustained = std::env::var_os("EXTEND_COMPUTER_SUSTAINED_PROBES").is_some();
    let count = if sustained { 1000 } else { 50 };
    let start = std::time::Instant::now();
    let mut timings = Vec::new();
    for index in 0..count {
        let rtt = client.probe()?.as_secs_f64() * 1000.0;
        if sustained && rtt > 25.0 {
            println!(
                "STALL index={index} elapsed_ms={:.1} rtt_ms={rtt:.1}",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        timings.push(rtt);
        std::thread::sleep(Duration::from_millis(if sustained { 8 } else { 50 }));
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "PAIRED_PROBES={} median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        count,
        timings[count / 2],
        timings[count * 95 / 100],
        timings[count - 1]
    );
    client.close()?;
    let mut client = Client::connect(dial()?, &identity, &store, None, Some(&peer), |_, _| {
        Decision::Deny
    })?;
    client.probe()?;
    println!("RECONNECTED_AND_PROBED=1");
    println!("WAIT_REVOKE");
    let mut ready = String::new();
    io::stdin().read_line(&mut ready)?;
    ensure!(ready.trim() == "revoked", "expected revocation signal");
    ensure!(client.probe().is_err(), "revoked live session still works");
    drop(client);
    ensure!(
        Client::connect(dial()?, &identity, &store, None, Some(&peer), |_, _| {
            Decision::AutomaticProbe
        })
        .is_err(),
        "revoked identity reconnected"
    );
    println!("PASS: pair, encrypted probes, reconnect, active revocation, rejected reconnect");
    Ok(())
}
