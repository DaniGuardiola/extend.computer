//! Bounded racing of read-only, pinned presence checks. Control sessions stay singular.
use super::*;
use std::{collections::BTreeSet, sync::mpsc};

pub(super) fn race<T: Send>(
    addresses: impl IntoIterator<Item = SocketAddr>,
    cancelled: impl Fn() -> bool + Sync,
    verify: impl Fn(TcpStream) -> Result<T> + Sync,
) -> Result<(SocketAddr, T, Option<&'static str>)> {
    let interfaces = wired::interfaces();
    race_ranked(
        addresses,
        cancelled,
        verify,
        |address| {
            interfaces
                .iter()
                .any(|interface| interface.contains(address.ip()))
        },
        |socket| {
            let local = socket.local_addr().ok()?.ip();
            interfaces
                .iter()
                .any(|interface| std::net::IpAddr::V4(interface.ip) == local)
                .then_some("usb")
        },
    )
}

fn race_ranked<T: Send>(
    addresses: impl IntoIterator<Item = SocketAddr>,
    cancelled: impl Fn() -> bool + Sync,
    verify: impl Fn(TcpStream) -> Result<T> + Sync,
    preferred: impl Fn(&SocketAddr) -> bool + Sync,
    route: impl Fn(&TcpStream) -> Option<&'static str> + Sync,
) -> Result<(SocketAddr, T, Option<&'static str>)> {
    let mut seen = BTreeSet::new();
    let mut addresses: Vec<_> = addresses
        .into_iter()
        .filter(|a| a.port() != 0 && !a.ip().is_unspecified() && seen.insert(*a))
        .take(32)
        .map(|address| (address, preferred(&address)))
        .collect();
    // Cable candidates get workers first. Discovery remains only a hint: a
    // preferred result still must pass the same pinned identity verification.
    addresses.sort_by_key(|(_, preferred)| !preferred);
    let mut preferred_left = addresses.iter().filter(|(_, preferred)| *preferred).count();
    let pending = Mutex::new(addresses.into_iter());
    // The same lock protects stop and registration: a socket can never escape cleanup.
    let sockets = Mutex::new((false, Vec::<TcpStream>::new()));
    let deadline = Instant::now() + Duration::from_secs(2);
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        for _ in 0..4 {
            let tx = tx.clone();
            let pending = &pending;
            let sockets = &sockets;
            let cancelled = &cancelled;
            let verify = &verify;
            let route = &route;
            scope.spawn(move || loop {
                if cancelled() || Instant::now() >= deadline || sockets.lock().unwrap().0 {
                    break;
                }
                let Some((address, preferred)) = pending.lock().unwrap().next() else {
                    break;
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let result = extend_computer_agent::local_network::connect_timeout(
                    &address,
                    remaining.min(Duration::from_millis(400)),
                )
                .map_err(anyhow::Error::from)
                .and_then(|socket| {
                    let mut state = sockets.lock().unwrap();
                    if state.0 || cancelled() || Instant::now() >= deadline {
                        let _ = socket.shutdown(Shutdown::Both);
                        anyhow::bail!("Route check cancelled");
                    }
                    state.1.push(socket.try_clone()?);
                    drop(state);
                    let selected = route(&socket);
                    verify(socket).map(|status| (status, selected))
                });
                if tx.send((address, preferred, result)).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut result = Err(anyhow::anyhow!(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "No authenticated local route is available"
        )));
        let mut fallback = None;
        let mut fallback_until = None;
        while !cancelled() && Instant::now() < deadline {
            if fallback_until.is_some_and(|until| Instant::now() >= until) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok((address, preferred, response)) => {
                    if preferred {
                        preferred_left = preferred_left.saturating_sub(1);
                    }
                    match response {
                        Ok((status, route)) => {
                            if route == Some("usb") || preferred_left == 0 {
                                result = Ok((address, status, route));
                                break;
                            }
                            if fallback.is_none() {
                                fallback = Some((address, status, route));
                                // A stale cable hint cannot hold up an authenticated
                                // Wi-Fi route for the full discovery deadline.
                                fallback_until = Some(Instant::now() + Duration::from_millis(400));
                            }
                        }
                        Err(error) => {
                            if std::env::var_os("EXTEND_ROUTE_DIAGNOSTICS").is_some() {
                                eprintln!("Authenticated route check {address} failed: {error:#}");
                            }
                            // A pinned protocol-version response is actionable. Unauthenticated
                            // addresses must not override availability or trust state.
                            let incompatible = error.downcast_ref::<extend_computer_agent::error::EngineError>()
                                == Some(&extend_computer_agent::error::EngineError::ProtocolIncompatible);
                            if incompatible {
                                result = Err(error);
                            }
                        }
                    }
                    if preferred_left == 0 && fallback.is_some() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        if result.is_err() && !cancelled() {
            if let Some(verified) = fallback {
                result = Ok(verified);
            }
        }
        let mut state = sockets.lock().unwrap();
        state.0 = true;
        for socket in state.1.drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
        drop(state);
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener};

    #[test]
    fn stalled_first_route_cannot_block_verified_winner_and_loser_is_closed() {
        let slow = TcpListener::bind("127.0.0.1:0").unwrap();
        let fast = TcpListener::bind("127.0.0.1:0").unwrap();
        let addresses = [slow.local_addr().unwrap(), fast.local_addr().unwrap()];
        let start = Instant::now();
        let connected = std::sync::Barrier::new(2);
        let result = race(
            addresses,
            || false,
            |mut socket| {
                connected.wait();
                if socket.peer_addr()? == addresses[0] {
                    let mut byte = [0];
                    socket.read_exact(&mut byte)?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result.0, addresses[1]);
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut loser = slow.accept().unwrap().0;
        loser
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(loser.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn cancellation_closes_in_flight_verification() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let start = Instant::now();
        let result = race(
            [address],
            || start.elapsed() > Duration::from_millis(100),
            |mut s| {
                s.read_exact(&mut [0])?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn stalled_verification_obeys_total_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        let result = race(
            [listener.local_addr().unwrap()],
            || false,
            |mut socket| {
                socket.read_exact(&mut [0])?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(start.elapsed() >= Duration::from_secs(2));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn authenticated_usb_wins_over_faster_wifi() {
        let wifi = TcpListener::bind("127.0.0.1:0").unwrap();
        let usb = TcpListener::bind("127.0.0.1:0").unwrap();
        let wifi_address = wifi.local_addr().unwrap();
        let usb_address = usb.local_addr().unwrap();
        // The cable answers only after Wi-Fi has verified, so Wi-Fi is always
        // faster no matter how busy the machine running the test is.
        let wifi_verified = std::sync::atomic::AtomicBool::new(false);
        let result = race_ranked(
            [wifi_address, usb_address],
            || false,
            |socket| {
                if socket.peer_addr()? == usb_address {
                    while !wifi_verified.load(std::sync::atomic::Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                } else {
                    wifi_verified.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            },
            |address| *address == usb_address,
            |socket| (socket.peer_addr().unwrap() == usb_address).then_some("usb"),
        )
        .unwrap();
        assert_eq!(result.0, usb_address);
        assert_eq!(result.2, Some("usb"));
    }

    #[test]
    fn unverified_usb_cannot_override_wifi_or_hold_it_for_full_deadline() {
        for stalled in [false, true] {
            let wifi = TcpListener::bind("127.0.0.1:0").unwrap();
            let usb = TcpListener::bind("127.0.0.1:0").unwrap();
            let wifi_address = wifi.local_addr().unwrap();
            let usb_address = usb.local_addr().unwrap();
            let start = Instant::now();
            let result = race_ranked(
                [wifi_address, usb_address],
                || false,
                |mut socket| {
                    if socket.peer_addr()? == usb_address {
                        if stalled {
                            socket.read_exact(&mut [0])?;
                        }
                        anyhow::bail!("Wrong pinned identity");
                    }
                    Ok(())
                },
                |address| *address == usb_address,
                |socket| (socket.peer_addr().unwrap() == usb_address).then_some("usb"),
            )
            .unwrap();
            assert_eq!(result.0, wifi_address);
            assert_eq!(result.2, None);
            assert!(start.elapsed() < Duration::from_secs(1));
        }
    }
}
