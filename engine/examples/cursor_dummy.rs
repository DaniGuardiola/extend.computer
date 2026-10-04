//! Autonomous encrypted cursor-path diagnostic. Never invokes native input APIs.
use anyhow::{Context, Result};
use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{self, Client, CursorSink, Decision},
    timing::Trace,
    trust::TrustStore,
};
use std::{
    io,
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
struct Dummy {
    trace: Trace,
    count: u64,
}
impl CursorSink for Dummy {
    fn approve(&mut self, _: &str) -> Result<bool> {
        Ok(true)
    }
    fn move_to(&mut self, _: f64, _: f64) -> Result<()> {
        self.trace.record("dummy_receive", self.count, 0);
        self.count += 1;
        Ok(())
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).context("usage: cursor_dummy serve|IP:PORT")?;
    let low_jitter = std::env::var_os("EXTEND_COMPUTER_TEST_LOW_JITTER").is_some();
    let dir = tempfile::tempdir()?;
    let store = TrustStore::open(dir.path())?;
    let identity = Identity::generate();
    if mode == "serve" {
        let listener = TcpListener::bind("0.0.0.0:48177")?;
        let window = PairingWindow::new(Duration::from_secs(120));
        eprintln!(
            "Pair code: {} (temporary dummy receiver)",
            window.code().unwrap()
        );
        listener.set_nonblocking(true)?;
        let accept_deadline = Instant::now() + Duration::from_secs(20);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        && Instant::now() < accept_deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => {
                    return Err(error).context("dummy receiver accept deadline or socket error")
                }
            }
        };
        // BSD/macOS accept can inherit O_NONBLOCK from the listener. The framed
        // protocol uses blocking reads with explicit deadlines.
        stream.set_nonblocking(false)?;
        eprintln!("dummy: TCP accepted");
        let peer = stream.peer_addr()?.ip();
        session::serve_connection_with_cursor(
            stream,
            &identity,
            &store,
            &mut Some(window),
            |_, _| Decision::Once,
            &mut extend_computer_agent::low_jitter::ManagedCursor::new(
                Dummy {
                    trace: Trace::from_env(),
                    count: 0,
                },
                peer,
                low_jitter,
            ),
        )?;
    } else {
        let mut code = Zeroizing::new(String::new());
        io::stdin().read_line(&mut code)?;
        let mut client = Client::connect(
            TcpStream::connect_timeout(&mode.parse()?, Duration::from_secs(5))
                .context("connect TCP")?,
            &identity,
            &store,
            Some(code.trim()),
            None,
            |_, _| Decision::Once,
        )?;
        eprintln!("dummy: authenticated");
        client
            .request_cursor()
            .context("request cursor capability")?;
        let _lease =
            extend_computer_agent::low_jitter::start_for_peer(client.address().ip(), low_jitter);
        let trace = Trace::from_env();
        let start = Instant::now();
        let mut count = 0;
        let mut pending = 0;
        println!("Synthetic cursor diagnostic active. No physical cursor movement.");
        while start.elapsed() < Duration::from_secs(15) {
            if pending == 4 {
                trace.record("ack_start", count, 0);
                client.probe()?;
                trace.record("ack_end", count, 0);
                pending = 0;
            }
            let phase = start.elapsed().as_secs_f64() * 3.0;
            trace.record("send_start", count, 0);
            client.queue_cursor(0.5 + 0.4 * phase.cos(), 0.5 + 0.4 * phase.sin())?;
            trace.record("send_end", count, 0);
            count += 1;
            pending += 1;
            std::thread::sleep(Duration::from_millis(4));
        }
        client.probe()?;
        client.close()?;
        println!("cursor_updates={count}");
    }
    Ok(())
}
