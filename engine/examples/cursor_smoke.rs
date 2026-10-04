//! Opt-in, 30-second edge-control driver for the two-Mac development harness.
use anyhow::{Context, Result};
use extend_computer_agent::{
    cursor,
    identity::Identity,
    session::{Client, Decision},
    trust::TrustStore,
};
use std::{
    io,
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        (3..=6).contains(&args.len()),
        "usage: cursor_smoke ADDRESS HELPER [left|right] [offset-y] [input]"
    );
    let mut code = Zeroizing::new(String::new());
    io::stdin().read_line(&mut code)?;
    let stream = args[1]
        .to_socket_addrs()?
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).ok())
        .context("connect peer")?;
    let state = tempfile::tempdir()?;
    let client = Client::connect(
        stream,
        &Identity::generate(),
        &TrustStore::open(state.path())?,
        Some(code.trim()),
        None,
        |_, _| Decision::Once,
    )?;
    let edge = args.get(3).map(String::as_str).unwrap_or("left");
    let offset = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(0.0);
    if let Some(seconds) = args.get(5).and_then(|s| s.strip_prefix("session:")) {
        let seconds: u64 = seconds.parse()?;
        extend_computer_agent::control::send_session(
            client,
            Path::new(&args[2]),
            true,
            edge,
            offset,
            (seconds != 0).then(|| Duration::from_secs(seconds)),
        )
    } else {
        let send = if args.get(5).is_some_and(|a| a == "input") {
            extend_computer_agent::control::send
        } else {
            cursor::send_with_policy
        };
        send(client, Path::new(&args[2]), true, edge, offset)
    }
}
