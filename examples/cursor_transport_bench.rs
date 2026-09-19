//! Deterministic transport comparison with an 8 ms response delay; no native input.
use anyhow::Result;
use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{serve_connection_with_cursor, Client, CursorSink, Decision},
    trust::TrustStore,
    wire,
};
use std::{
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
struct Sink;
impl CursorSink for Sink {
    fn approve(&mut self, _: &str) -> Result<bool> {
        Ok(true)
    }
    fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
        Ok(())
    }
}
fn run(streamed: bool) -> Result<Duration> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let server_address = listener.local_addr()?;
    let window = PairingWindow::new(Duration::from_secs(30));
    let code = window.code().unwrap().to_owned();
    let server = thread::spawn(move || -> Result<()> {
        let dir = tempfile::tempdir()?;
        serve_connection_with_cursor(
            listener.accept()?.0,
            &Identity::generate(),
            &TrustStore::open(dir.path())?,
            &mut Some(window),
            |_, _| Decision::Once,
            &mut Sink,
        )
    });
    let relay = TcpListener::bind("127.0.0.1:0")?;
    let address = relay.local_addr()?;
    let proxy = thread::spawn(move || -> Result<()> {
        let (mut downstream, _) = relay.accept()?;
        let mut upstream = TcpStream::connect(server_address)?;
        wire::configure(&downstream)?;
        wire::configure(&upstream)?;
        let mut request = downstream.try_clone()?;
        let mut destination = upstream.try_clone()?;
        let forward = thread::spawn(move || {
            let _ = std::io::copy(&mut request, &mut destination);
            let _ = destination.shutdown(std::net::Shutdown::Write);
        });
        while let Ok(frame) = wire::read_frame(&mut upstream) {
            thread::sleep(Duration::from_millis(8));
            wire::write_frame(&mut downstream, &frame)?;
        }
        let _ = downstream.shutdown(std::net::Shutdown::Both);
        forward.join().unwrap();
        Ok(())
    });
    let dir = tempfile::tempdir()?;
    let mut client = Client::connect(
        TcpStream::connect(address)?,
        &Identity::generate(),
        &TrustStore::open(dir.path())?,
        Some(&code),
        None,
        |_, _| Decision::Once,
    )?;
    client.request_cursor()?;
    let start = Instant::now();
    for i in 0..80 {
        if streamed {
            client.queue_cursor(i as f64 / 80.0, 0.5)?;
            if i % 4 == 3 {
                client.probe()?;
            }
        } else {
            client.move_cursor(i as f64 / 80.0, 0.5)?;
        }
    }
    let elapsed = start.elapsed();
    client.close()?;
    server.join().unwrap()?;
    proxy.join().unwrap()?;
    Ok(elapsed)
}
fn main() -> Result<()> {
    let baseline = run(false)?;
    let streamed = run(true)?;
    println!(
        "simulated_response_delay_ms=8 updates=80 stop_wait_ms={:.1} window4_ms={:.1} ratio={:.2}",
        baseline.as_secs_f64() * 1000.0,
        streamed.as_secs_f64() * 1000.0,
        baseline.as_secs_f64() / streamed.as_secs_f64()
    );
    Ok(())
}
