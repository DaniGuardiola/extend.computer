//! Explicit development check for overlapping, bounded low-jitter leases.
use anyhow::{Context, Result};
use std::time::{Duration, Instant};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 3, "usage: lease_soak PEER_IP SECONDS");
    let seconds: u64 = args[2].parse()?;
    anyhow::ensure!(
        (1..=300).contains(&seconds),
        "duration must be 1..300 seconds"
    );
    let mut lease = extend_computer_agent::low_jitter::start_for_peer(args[1].parse()?, true)
        .context("eligible Wi-Fi lease required")?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        lease.maintain()?;
        std::thread::sleep(Duration::from_millis(250));
    }
    drop(lease);
    println!("PASS lease maintained for {seconds}s and released");
    Ok(())
}
