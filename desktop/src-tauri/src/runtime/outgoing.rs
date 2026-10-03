//! Initiated pairing and authenticated control connections.
use super::*;
use extend_computer_agent::session::{Client, Decision};
use zeroize::Zeroizing;

fn normalize_pairing_code(input: &str) -> Result<Zeroizing<String>> {
    let digits = Zeroizing::new(
        input
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect::<String>(),
    );
    ensure!(
        digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
        "Enter the 16-character code shown on the other device."
    );
    Ok(Zeroizing::new(
        digits
            .as_bytes()
            .chunks(4)
            .map(|part| std::str::from_utf8(part).unwrap().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
    ))
}

impl Desktop {
    pub fn pair_nearby(self: &Arc<Self>, device: Device) -> Result<()> {
        peers::validate(&device)?;
        let (id, cancelled) = self.reserve(SessionKind::Pair, None)?;
        let app = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                extend_computer_agent::low_jitter::require_ready_for_peer(
                    device.address.parse::<SocketAddr>()?.ip(),
                )?;
                let identity = app.identity()?;
                ensure!(!cancelled.load(Ordering::SeqCst), "Pairing cancelled");
                let socket =
                    TcpStream::connect_timeout(&device.address.parse()?, Duration::from_secs(5))
                        .context(errors::PairingStartError)?;
                app.set_socket(id, &socket)?;
                let mut verification_started = false;
                let peer = extend_computer_agent::session::pair_visually(
                    socket,
                    &identity,
                    &app.store()?,
                    |peer, symbols| {
                        verification_started = true;
                        app.stage(id, Phase::Approval, Some(peer));
                        app.approvals
                            .ask_verified("verify", peer, Some(*symbols), || {
                                app.cancelled(id, &cancelled)
                            })
                            == Answer::Remember
                    },
                )
                .map_err(|error| {
                    if !verification_started && is_connection_error(&error) {
                        error.context(errors::PairingStartError)
                    } else {
                        error
                    }
                })?;
                app.save_device(&peer, device.clone())?;
                let _ = app.sync_peer_name(&peer, &device, &identity, Some(id));
                Ok(())
            })();
            app.finish(id, result);
        });
        Ok(())
    }
    pub fn close_pairing(&self) {
        let cancel = {
            let mut inner = self.inner.lock().unwrap();
            inner.code = None;
            inner.pairing_open = false;
            inner.advertisement = None;
            inner.job.as_ref().is_some_and(|j| {
                j.view.kind == SessionKind::Pair
                    || (j.view.kind == SessionKind::Incoming && j.view.phase != Phase::Connected)
            })
        };
        if cancel {
            self.disconnect();
        }
    }

    pub fn pair(self: &Arc<Self>, device: Device, code: String) -> Result<()> {
        peers::validate(&device)?;
        ensure!(
            !code.trim().is_empty() && code.len() <= 128,
            "Enter the code shown on the other device."
        );
        let code = normalize_pairing_code(&code)?;
        let (id, cancelled) = self.reserve(SessionKind::Pair, None)?;
        let app = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                extend_computer_agent::low_jitter::require_ready_for_peer(
                    device.address.parse::<SocketAddr>()?.ip(),
                )?;
                let identity = app.identity()?;
                ensure!(!cancelled.load(Ordering::SeqCst), "Pairing cancelled");
                let address = device.address.parse()?;
                let socket = TcpStream::connect_timeout(&address, Duration::from_secs(5)).context(
                    "Could not reach this device. Open extend.computer there and show its pairing code.",
                )?;
                app.set_socket(id, &socket)?;
                let client = Client::connect(
                    socket,
                    &identity,
                    &app.store()?,
                    Some(code.trim()),
                    None,
                    |_, _| Decision::Remember,
                )?;
                let peer = client.peer().to_owned();
                app.save_device(&peer, device.clone())?;
                client.close()?;
                let _ = app.sync_peer_name(&peer, &device, &identity, Some(id));
                Ok(())
            })();
            app.finish(id, result);
        });
        Ok(())
    }
    pub fn connect(self: &Arc<Self>, peer: String, mut device: Device) -> Result<()> {
        // Discovery may have repaired the address since the GUI snapshot.
        if let Some(saved) = self.devices.lock().unwrap().get(&peer) {
            device.address = saved.address.clone();
        }
        self.save_device(&peer, device.clone())?;
        self.refresh_permissions()?;
        let permissions = self.inner.lock().unwrap().permissions.clone();
        ensure!(
            permissions.available,
            "extend.computer's input helper is missing. Reinstall the app."
        );
        ensure!(
            permissions.listen && permissions.post,
            "Allow Input Monitoring and Accessibility in Permissions, then try again."
        );
        let (id, cancelled) = self.reserve(SessionKind::Outgoing, Some(peer.clone()))?;
        let app = self.clone();
        std::thread::spawn(move || {
            let mut connected_once = false;
            let mut retries = 0;
            let result = loop {
                let result = (|| -> Result<()> {
                    if let Some(saved) = app.devices.lock().unwrap().get(&peer) {
                        device = saved.clone();
                    }
                    let identity = app.identity()?;
                    ensure!(!cancelled.load(Ordering::SeqCst), "Connection cancelled");
                    let address = device.address.parse::<SocketAddr>()?;
                    let socket = if address.ip().is_unspecified() {
                        app.account_tunnel(&peer, "control")?
                    } else {
                        extend_computer_agent::low_jitter::require_ready_for_peer(address.ip())?;
                        TcpStream::connect_timeout(&address, Duration::from_millis(700))
                            .map_err(anyhow::Error::from)
                            .or_else(|_| app.account_tunnel(&peer, "control"))?
                    };
                    app.set_socket(id, &socket)?;
                    let client = match Client::connect(
                        socket,
                        &identity,
                        &app.store()?,
                        None,
                        Some(&peer),
                        |_, _| Decision::Once,
                    ) {
                        Ok(client) => client,
                        Err(error)
                            if error
                                .downcast_ref::<extend_computer_agent::error::EngineError>()
                                == Some(
                                    &extend_computer_agent::error::EngineError::PeerUnpaired,
                                ) =>
                        {
                            app.apply_peer_unpaired(&peer, id)?;
                            return Ok(());
                        }
                        Err(error) => return Err(error),
                    };
                    extend_computer_agent::control::send_session_with_ready(
                        client,
                        &app.helper,
                        true,
                        &device.edge,
                        0.,
                        None,
                        retries > 0,
                        || {
                            connected_once = true;
                            app.stage(id, Phase::Connected, None);
                        },
                        || {
                            app.devices
                                .lock()
                                .unwrap()
                                .get(&peer)
                                .map(|device| device.edge.clone())
                        },
                    )
                })();
                match result {
                    Err(error)
                        if connected_once
                            && retries < 3
                            && is_connection_error(&error)
                            && !cancelled.load(Ordering::SeqCst) =>
                    {
                        retries += 1;
                        if !app.prepare_reconnect(id) {
                            break Ok(());
                        }
                        let wait = Duration::from_secs(1 << (retries - 1));
                        eprintln!(
                            "Control connection lost: {error:#}. Reconnecting in {}s.",
                            wait.as_secs()
                        );
                        let until = Instant::now() + wait;
                        while Instant::now() < until && !app.cancelled(id, &cancelled) {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        if app.cancelled(id, &cancelled) {
                            break Ok(());
                        }
                    }
                    result => break result,
                }
            };
            app.finish(id, result);
        });
        Ok(())
    }
}
