//! Best-effort pinned removal notification. Local revocation never waits for a peer.
use super::*;
use std::{
    collections::BTreeMap,
    net::Shutdown,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
static OPERATIONS: Mutex<BTreeMap<String, (u64, bool)>> = Mutex::new(BTreeMap::new());
static WORKERS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn record(peer: &str) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut operations = OPERATIONS.lock().unwrap();
    if operations.len() >= 256 && !operations.contains_key(peer) {
        if let Some(oldest) = operations.keys().next().cloned() {
            operations.remove(&oldest);
        }
    }
    operations.insert(peer.to_owned(), (id, false));
    id
}
fn current(peer: &str, id: u64) -> bool {
    OPERATIONS
        .lock()
        .unwrap()
        .get(peer)
        .is_some_and(|operation| operation.0 == id)
}
struct Worker;
impl Drop for Worker {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) fn notify(runtime: &Runtime, peer: String, address: String, id: u64) -> Result<()> {
    let address: SocketAddr = address.parse().context("No cached literal peer address")?;
    ensure!(
        address.port() != 0 && !address.ip().is_unspecified() && !address.ip().is_multicast(),
        "Invalid cached peer address"
    );
    ensure!(
        current(&peer, id) && runtime.trust.peer(&peer)?.is_none(),
        "Pairing changed during notification"
    );
    // A token can schedule at most one worker; duplicate JNI calls cannot flood sockets.
    {
        let mut operations = OPERATIONS.lock().unwrap();
        ensure!(
            operations.get(&peer) == Some(&(id, false)),
            "Stale unpair notification"
        );
        operations.insert(peer.clone(), (id, true));
    }
    if WORKERS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            (count < 4).then_some(count + 1)
        })
        .is_err()
    {
        return Ok(()); // Nonessential notification is dropped under load.
    }
    let permit = Worker;
    let identity = runtime.identity.clone();
    let trust = runtime.trust.clone();
    std::thread::Builder::new()
        .name("extend-unpair-hint".into())
        .spawn(move || {
            let _permit = permit;
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_millis(500))
            else {
                return;
            };
            let Ok(interrupt) = socket.try_clone() else {
                return;
            };
            let done = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let timer_done = done.clone();
            let timer = std::thread::spawn(move || {
                let (lock, changed) = &*timer_done;
                let finished = lock.lock().unwrap();
                let (finished, _) = changed
                    .wait_timeout_while(
                        finished,
                        deadline.saturating_duration_since(std::time::Instant::now()),
                        |finished| !*finished,
                    )
                    .unwrap();
                if !*finished {
                    let _ = interrupt.shutdown(Shutdown::Both);
                }
            });
            let _ =
                extend_computer_agent::session::notify_unpair_if(socket, &identity, &peer, || {
                    std::time::Instant::now() < deadline
                        && current(&peer, id)
                        && trust.peer(&peer).is_ok_and(|value| value.is_none())
                });
            *done.0.lock().unwrap() = true;
            done.1.notify_all();
            let _ = timer.join();
        })?;
    Ok(())
}
