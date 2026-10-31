//! Authenticated display sessions use a separate listener from native input.
use super::*;
use extend_computer_agent::{
    display::{DisplayEvent, Transport},
    session::display::{Config, DisplaySignal},
};
use std::net::IpAddr;
mod bitrate;
mod helper;
mod input;
mod receiver;
pub(super) struct SettingsUpdate {
    config: Config,
    suspend_input: bool,
    reply: std::sync::mpsc::Sender<Result<Config>>,
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn usb_checkpoint_retires_local_intent_even_after_unobserved_short_outage() {
        let now = Instant::now();
        let mut receiver = receiver::Receiver::new(true, true, true);
        let revision = receiver.state().revision;
        receiver
            .native(
                receiver::NativeEvent::Focus {
                    active: true,
                    revision,
                },
                now,
            )
            .unwrap();
        assert!(receiver.state().focused);
        let mut checkpoint = (0, None);
        let completed = (1, Some(now));
        observe_recovery_input(completed, &mut checkpoint, None, Some(&mut receiver)).unwrap();
        assert!(input_quarantined(checkpoint, now));
        let interrupted_revision = receiver.state().revision;
        observe_recovery_input(completed, &mut checkpoint, None, Some(&mut receiver)).unwrap();
        assert_eq!(receiver.state().revision, interrupted_revision);

        let interrupted = viewer_state_for_transport(&receiver, true);
        assert!(!interrupted.available);
        assert!(!interrupted.focused);
        assert!(!interrupted.enabled);
        assert!(!interrupted.text_supported);
        assert!(interrupted.shortcut_policy.is_none());
        assert!(interrupted.revision > revision);

        // A native event queued before the outage cannot request a new lease.
        receiver
            .native(
                receiver::NativeEvent::Focus {
                    active: true,
                    revision,
                },
                now,
            )
            .unwrap();
        let after_cooldown = now + Duration::from_secs(2);
        assert!(!input_quarantined(checkpoint, after_cooldown));
        let restored = viewer_state_for_transport(&receiver, false);
        assert!(restored.available);
        assert!(!restored.focused);
        assert!(!restored.enabled);
        assert!(receiver.message(now).is_none());
    }
}

fn physical_usb_peer(local: IpAddr, peer: IpAddr) -> bool {
    wired::interfaces()
        .iter()
        .any(|interface| IpAddr::V4(interface.ip) == local && interface.contains(peer))
}

fn viewer_state_for_transport(receiver: &receiver::Receiver, recovering: bool) -> receiver::State {
    let mut state = receiver.state();
    if recovering {
        // Retire this endpoint's native queue through the original receiver
        // request lifetime. Local old revisions cannot rearm input.
        state.available = false;
        state.enabled = false;
        state.focused = false;
        state.text_supported = false;
        state.shortcut_policy = None;
    }
    state
}

fn input_quarantined(checkpoint: (u64, Option<Instant>), now: Instant) -> bool {
    checkpoint.0 != 0
        && checkpoint
            .1
            .is_none_or(|ended| now < ended + receiver::TIMEOUT)
}

fn observe_recovery_input(
    checkpoint: (u64, Option<Instant>),
    previous: &mut (u64, Option<Instant>),
    worker: Option<&input::Worker>,
    receiver: Option<&mut receiver::Receiver>,
) -> Result<()> {
    if checkpoint != *previous && checkpoint.0 != 0 {
        // Release only this endpoint's local holds. The remote source retains
        // its existing bounded lease deadman until a Stop/Revoked is delivered.
        if let Some(worker) = worker {
            worker.transport_recovery(checkpoint.0, checkpoint.1)?;
        }
        if checkpoint.0 != previous.0 {
            if let Some(receiver) = receiver {
                receiver.reconfigure(Instant::now())?;
            }
        }
    }
    *previous = checkpoint;
    Ok(())
}

fn apply_source_settings(
    signal: &mut DisplaySignal,
    settings: &std::sync::mpsc::Sender<SettingsUpdate>,
    requested: Config,
) -> Result<Config> {
    requested.validate()?;
    let (reply, result) = std::sync::mpsc::channel();
    settings.send(SettingsUpdate {
        config: requested.clone(),
        reply,
        suspend_input: true,
    })?;
    result.recv_timeout(Duration::from_secs(6))??;
    let config = signal.configure(&requested)?;
    let (reply, result) = std::sync::mpsc::channel();
    settings.send(SettingsUpdate {
        config,
        reply,
        suspend_input: false,
    })?;
    result.recv_timeout(Duration::from_secs(8))?
}

fn validate_settings(previous: &Config, next: &Config) -> Result<()> {
    next.validate()?;
    ensure!(
        previous.mode == next.mode
            && previous.codec == next.codec
            && previous.separate_cursor == next.separate_cursor,
        "Display settings cannot change the connection mode"
    );
    Ok(())
}

impl Desktop {
    pub fn update_display(&self, session: u64, mut config: Config) -> Result<()> {
        #[cfg(windows)]
        {
            config.codec = extend_computer_agent::session::display::VideoCodec::H264;
            config.separate_cursor = false;
        }
        if config.auto_size && config.mode == extend_computer_agent::session::display::Mode::Mirror
        {
            helper::Helper::screen_size(&self.display_helper()?, &mut config)?;
            config.auto_size = false;
        }
        let sender = {
            let inner = self.inner.lock().unwrap();
            let job = inner
                .job
                .as_ref()
                .filter(|j| {
                    j.view.id == session
                        && j.view.kind == SessionKind::Outgoing
                        && j.view.phase == Phase::Connected
                })
                .context("This display connection is no longer active")?;
            validate_settings(
                job.view
                    .display_config
                    .as_ref()
                    .context("Display settings are not ready")?,
                &config,
            )?;
            job.display_updates
                .clone()
                .context("Display settings are not ready")?
        };
        let (reply, result) = std::sync::mpsc::channel();
        sender
            .send(SettingsUpdate {
                config,
                reply,
                suspend_input: false,
            })
            .context("Display connection ended")?;
        result
            .recv_timeout(Duration::from_secs(12))
            .context("Display settings did not respond")??;
        Ok(())
    }
    /// Finite, explicit developer test entry point. Uses the regular account,
    /// routing, authentication, and cancellation paths rather than a test bypass.
    #[cfg(any(debug_assertions, feature = "display-benchmark"))]
    pub fn start_display_benchmark(
        self: &Arc<Self>,
        mut peer: String,
        config: Config,
        seconds: u64,
    ) {
        let app = self.clone();
        std::thread::spawn(move || {
            let ready_until = Instant::now() + Duration::from_secs(60);
            let result = (|| -> Result<()> {
                config.validate()?;
                ensure!((30..=3600).contains(&seconds), "Invalid benchmark duration");
                loop {
                    ensure!(
                        Instant::now() < ready_until,
                        "Benchmark account peer not ready"
                    );
                    if peer == "account" {
                        let accounts = app.account_devices.lock().unwrap();
                        if accounts.len() == 1 {
                            peer = accounts.keys().next().unwrap().clone();
                        } else {
                            drop(accounts);
                            std::thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    }
                    if app.store()?.is_account_peer(&peer)? {
                        let device = app.devices.lock().unwrap().get(&peer).cloned();
                        if let Some(device) = device {
                            app.connect_display(peer.clone(), device, config.clone(), Vec::new())?;
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                // Explicit development-only control for unattended settings
                // transition tests. Uses the same validated update path as IPC.
                let settings_file =
                    std::env::var_os("EXTEND_DISPLAY_BENCHMARK_SETTINGS").map(PathBuf::from);
                let benchmark_session = app
                    .inner
                    .lock()
                    .unwrap()
                    .job
                    .as_ref()
                    .filter(|job| job.view.peer.as_deref() == Some(&peer))
                    .map(|job| job.view.id)
                    .context("Missing benchmark session")?;
                let deadline = Instant::now() + Duration::from_secs(seconds);
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Request {
                    sequence: u64,
                    config: Config,
                }
                // A restarted benchmark must not replay the previous run's mode.
                let mut previous_sequence = settings_file
                    .as_ref()
                    .and_then(|path| std::fs::read(path).ok())
                    .filter(|bytes| bytes.len() <= 4096)
                    .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok())
                    .map(|request| request.sequence);
                while Instant::now() < deadline && !app.closing.load(Ordering::SeqCst) {
                    if app
                        .inner
                        .lock()
                        .unwrap()
                        .job
                        .as_ref()
                        .map(|job| job.view.id)
                        != Some(benchmark_session)
                    {
                        break;
                    }
                    if let Some(path) = &settings_file {
                        if let Ok(bytes) = std::fs::read(path) {
                            if bytes.len() <= 4096 {
                                if let Ok(request) = serde_json::from_slice::<Request>(&bytes) {
                                    if previous_sequence.is_none_or(|old| request.sequence > old) {
                                        let session = app
                                            .inner
                                            .lock()
                                            .unwrap()
                                            .job
                                            .as_ref()
                                            .filter(|job| {
                                                job.view.id == benchmark_session
                                                    && job.view.phase == Phase::Connected
                                            })
                                            .map(|job| job.view.id);
                                        if let Some(session) = session {
                                            let outcome =
                                                app.update_display(session, request.config);
                                            let applied =
                                                app.inner.lock().unwrap().job.as_ref().and_then(
                                                    |job| job.view.display_config.clone(),
                                                );
                                            let ack = serde_json::json!({"sequence":request.sequence,"ok":outcome.is_ok(),
                                                "error":outcome.err().map(|error|format!("{error:#}")),"configuration":applied});
                                            std::fs::write(
                                                path.with_extension("ack.json"),
                                                serde_json::to_vec(&ack)?,
                                            )?;
                                            previous_sequence = Some(request.sequence);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                if app
                    .inner
                    .lock()
                    .unwrap()
                    .job
                    .as_ref()
                    .map(|job| job.view.id)
                    == Some(benchmark_session)
                {
                    app.disconnect();
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("Display benchmark failed: {error:#}");
            }
        });
    }
    fn display_helper(&self) -> Result<PathBuf> {
        crate::platform::helper(&self.helper, "Display")
    }
    pub fn start_display_listener(self: &Arc<Self>) -> Result<()> {
        let mut listener =
            wired::Listeners::bind(48179).context("Display receiving port is in use")?;
        listener.set_nonblocking(true)?;
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            let Some(app) = weak.upgrade() else { break };
            if app.closing.load(Ordering::SeqCst) {
                break;
            }
            if let Ok((socket, _)) = listener.accept() {
                if !app.inner.lock().unwrap().receiving_enabled {
                    continue;
                }
                if let Ok((id, cancelled)) = app.reserve(SessionKind::Incoming, None) {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        let result = app.receive_display(id, cancelled, socket, None);
                        app.finish(id, result);
                    });
                }
            }
            drop(app);
            std::thread::sleep(Duration::from_millis(30));
        });
        Ok(())
    }
    pub(super) fn serve_usb_display(
        self: &Arc<Self>,
        socket: TcpStream,
        route: &usb::Route,
    ) -> Result<()> {
        ensure!(route.current(self), "USB connection was cancelled");
        ensure!(
            self.inner.lock().unwrap().receiving_enabled,
            "Incoming connections are disabled"
        );
        let (id, cancelled) = self.reserve(SessionKind::Incoming, Some(route.peer.clone()))?;
        self.set_display_route(id, "usb");
        let result = self.receive_display(id, cancelled, socket, Some(route));
        self.finish(id, result);
        Ok(())
    }
    fn receive_display(
        self: &Arc<Self>,
        id: u64,
        cancelled: Arc<AtomicBool>,
        socket: TcpStream,
        usb_route: Option<&usb::Route>,
    ) -> Result<()> {
        socket.set_nonblocking(false)?;
        extend_computer_agent::local_network::bind_stream_to_local_interface(&socket)?;
        self.set_socket(id, &socket)?;
        if usb_route.is_some() {
            self.set_display_route(id, "usb");
        }
        let shutdown = socket.try_clone()?;
        let remote_ip = socket.peer_addr()?.ip();
        let ip = socket.local_addr()?.ip();
        let cable_route = wired::socket_route(&socket);
        let physical_usb =
            usb_route.is_none() && cable_route == Some("usb") && physical_usb_peer(ip, remote_ip);
        let identity = self.identity()?;
        let store = self.store()?;
        let mut media = None;
        let mut configuration = None;
        let mut prepared_viewer = None;
        let answer = |peer: &str, config: &mut Config, offer: &str, features| {
            if let Some(route) = usb_route {
                ensure!(route.current(self), "USB connection was cancelled");
            }
            let permissions =
                self.refresh_permissions_for_route(usb_route.is_some() || physical_usb)?;
            ensure!(
                if usb_route.is_some() || cable_route == Some("usb") {
                    permissions.available && permissions.post
                } else {
                    permissions.can_receive()
                },
                "Incoming permissions are not ready"
            );
            ensure!(
                self.inner.lock().unwrap().receiving_enabled && !self.cancelled(id, &cancelled),
                "Incoming connections are disabled"
            );
            if config.auto_size {
                let path = self.display_helper()?;
                helper::Helper::screen_size(&path, config)?;
                let viewer = helper::Helper::start(&path, config, false, false, &[])?;
                viewer.fit_viewport(config)?;
                viewer.configure(config)?;
                prepared_viewer = Some(viewer);
            }
            self.set_display_mode(id, config.mode);
            self.stage(id, Phase::Connecting, Some(peer));
            let (mut transport, route) = if let Some(route) = usb_route {
                (route.transport(self, config.codec)?, "usb")
            } else {
                (
                    Transport::with_udp_peer(ip, remote_ip, config.codec)?,
                    cable_route.unwrap_or("local"),
                )
            };
            if physical_usb {
                transport.prefer_kernel_arrival();
            }
            self.set_display_route(id, route);
            let answer = transport.answer_with_features(offer, features)?;
            media = Some(transport);
            configuration = Some(config.clone());
            self.set_display_config(id, config);
            Ok(answer)
        };
        let (mut signal, peer, _) = if let Some(route) = usb_route {
            ensure!(route.current(self), "USB connection was cancelled");
            DisplaySignal::accept_with_settings_features_pinned(
                socket,
                &identity,
                &store,
                &route.peer,
                false,
                answer,
            )?
        } else {
            DisplaySignal::accept_with_features(socket, &identity, &store, answer)?
        };
        if physical_usb {
            media
                .as_mut()
                .context("Missing display transport")?
                .enable_physical_usb_recovery()?;
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let heartbeat = Arc::new(Mutex::new(signal.last_heartbeat()));
        let verified_heartbeat = heartbeat.clone();
        let (settings, settings_received) = std::sync::mpsc::channel();
        let mut selected = configuration
            .as_ref()
            .context("Missing display configuration")?
            .clone();
        let ended = stopped.clone();
        let app = self.clone();
        let remote = peer.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut sequence = 0;
                while !app.cancelled(id, &cancelled)
                    && app.inner.lock().unwrap().receiving_enabled
                    && app.store()?.peer(&remote)?.is_some()
                {
                    if sequence < 3 && std::env::var_os("EXTEND_DISPLAY_METRICS").is_some() {
                        eprintln!("Display heartbeat {sequence} waiting");
                    }
                    if !signal.respond_with_config(|config| {
                        validate_settings(&selected, &config)?;
                        let (reply, result) = std::sync::mpsc::channel();
                        settings.send(SettingsUpdate {
                            config: config.clone(),
                            reply,
                            suspend_input: false,
                        })?;
                        let config = result.recv_timeout(Duration::from_secs(8))??;
                        selected = config.clone();
                        Ok(config)
                    })? {
                        break;
                    }
                    *verified_heartbeat.lock().unwrap() = signal.last_heartbeat();
                    if sequence < 3 && std::env::var_os("EXTEND_DISPLAY_METRICS").is_some() {
                        eprintln!("Display heartbeat {sequence} answered");
                    }
                    sequence += 1;
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("Display signaling ended: {error:#}");
            }
            *verified_heartbeat.lock().unwrap() = None;
            ended.store(true, Ordering::SeqCst);
        });
        let result = self.run_display(
            id,
            &peer,
            media.context("Missing display transport")?,
            &configuration.context("Missing display configuration")?,
            false,
            stopped,
            remote_ip,
            settings_received,
            prepared_viewer,
            physical_usb,
            heartbeat,
        );
        let _ = shutdown.shutdown(Shutdown::Both);
        result
    }
    pub fn connect_display(
        self: &Arc<Self>,
        peer: String,
        device: Device,
        mut config: Config,
        presets: Vec<(u32, u32)>,
    ) -> Result<()> {
        // The Windows helper mirrors only, encodes H.264 and draws the
        // pointer into the video.
        #[cfg(windows)]
        {
            ensure!(
                config.mode == extend_computer_agent::session::display::Mode::Mirror,
                "Only display mirroring is available on Windows so far"
            );
            config.codec = extend_computer_agent::session::display::VideoCodec::H264;
            config.separate_cursor = false;
        }
        config.validate()?;
        ensure!(
            presets.len() <= 8
                && presets
                    .iter()
                    .all(|&(w, h)| (640..=4096).contains(&w) && (480..=4096).contains(&h)),
            "Invalid display sizes"
        );
        if config.mode == extend_computer_agent::session::display::Mode::Mirror && config.auto_size
        {
            helper::Helper::screen_size(&self.display_helper()?, &mut config)?;
            // The source aspect is a preference. Keep automatic sizing enabled
            // so the pinned receiver can select a hardware-supported raster.
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        anyhow::bail!("Display connections currently require macOS or Windows");
        let identity = self.identity()?;
        ensure!(
            self.store()?.peer(&peer)?.is_some(),
            "Pair this device or sign in first"
        );
        ensure!(
            !self.usb_connecting.load(Ordering::Acquire),
            "USB connection is still starting"
        );
        let usb_route = self.selected_usb_route(&peer);
        let (id, cancelled) = self.reserve(SessionKind::Outgoing, Some(peer.clone()))?;
        self.set_display_mode(id, config.mode);
        if let Some(job) = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_mut()
            .filter(|j| j.view.id == id)
        {
            job.display_presets = presets;
        }
        let app = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut device = device;
                let (socket, remote_ip) = if let Some(route) = &usb_route {
                    (
                        route.open(
                            &app,
                            extend_computer_agent::display::usb_stream::UsbService::Display,
                            &cancelled,
                        )?,
                        "192.0.2.2".parse()?,
                    )
                } else {
                    app.refresh_local_route(&peer, &device, &identity)?;
                    if let Some(saved) = app.devices.lock().unwrap().get(&peer) {
                        device = saved.clone();
                    }
                    let address: SocketAddr = device
                        .address
                        .parse()
                        .context("No local address found for this device")?;
                    ensure!(
                        !address.ip().is_unspecified(),
                        "This device is not available on your local network"
                    );
                    let display_address = SocketAddr::new(address.ip(), 48179);
                    (
                        extend_computer_agent::local_network::connect_timeout(
                            &display_address,
                            Duration::from_secs(3),
                        )
                        .with_context(|| {
                            format!("Display control connection to {display_address}")
                        })?,
                        address.ip(),
                    )
                };
                let cable_route = wired::socket_route(&socket);
                let local_ip = socket.local_addr()?.ip();
                // Carry physical route provenance into capture startup after
                // the pinned signaling handshake. Generic "usb" labels also
                // include Android bulk links and cannot select this preference.
                let physical_usb = usb_route.is_none()
                    && cable_route == Some("usb")
                    && physical_usb_peer(local_ip, remote_ip);
                app.set_socket_route(id, &socket, usb_route.as_ref().map(|_| "usb"))?;
                let shutdown = socket.try_clone()?;
                let (mut media, route) = if let Some(route) = &usb_route {
                    (route.transport(&app, config.codec)?, "usb")
                } else {
                    (
                        Transport::with_udp_peer(local_ip, remote_ip, config.codec)?,
                        cable_route.unwrap_or("local"),
                    )
                };
                if physical_usb {
                    media.prefer_kernel_arrival();
                }
                app.set_display_route(id, route);
                let (mut signal, config, answer, _) = DisplaySignal::connect_with_settings_offer(
                    socket,
                    &identity,
                    &peer,
                    &config,
                    true,
                    |features| {
                        #[cfg(not(target_os = "macos"))]
                        let features = features.without_input();
                        media.offer_with_features(features)
                    },
                )?;
                media.accept_answer(&answer)?;
                if physical_usb {
                    media.enable_physical_usb_recovery()?;
                }
                app.set_display_config(id, &config);
                let stopped = Arc::new(AtomicBool::new(false));
                let heartbeat = Arc::new(Mutex::new(signal.last_heartbeat()));
                let verified_heartbeat = heartbeat.clone();
                let (settings, settings_received) = std::sync::mpsc::channel();
                let (requests, requested) = std::sync::mpsc::channel::<SettingsUpdate>();
                if let Some(job) = app
                    .inner
                    .lock()
                    .unwrap()
                    .job
                    .as_mut()
                    .filter(|j| j.view.id == id)
                {
                    job.display_updates = Some(requests);
                }
                let ended = stopped.clone();
                let monitor = app.clone();
                let remote = peer.clone();
                std::thread::spawn(move || {
                    let result = (|| -> Result<()> {
                        let mut sequence = 0;
                        while !ended.load(Ordering::SeqCst)
                            && !monitor.cancelled(id, &cancelled)
                            && monitor.store()?.peer(&remote)?.is_some()
                        {
                            while let Ok(update) = requested.try_recv() {
                                let outcome = (|| -> Result<Config> {
                                    apply_source_settings(&mut signal, &settings, update.config)
                                })();
                                let _ = update.reply.send(outcome);
                            }
                            if sequence < 3 && std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()
                            {
                                eprintln!("Display heartbeat {sequence} request");
                            }
                            signal.pulse_with_settings(sequence, |signal, config| {
                                apply_source_settings(signal, &settings, config)
                            })?;
                            *verified_heartbeat.lock().unwrap() = signal.last_heartbeat();
                            if sequence < 3 && std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()
                            {
                                eprintln!("Display heartbeat {sequence} verified");
                            }
                            sequence += 1;
                            std::thread::sleep(Duration::from_secs(1));
                        }
                        signal.close();
                        Ok(())
                    })();
                    if let Err(error) = result {
                        eprintln!("Display signaling ended: {error:#}");
                    }
                    *verified_heartbeat.lock().unwrap() = None;
                    ended.store(true, Ordering::SeqCst);
                });
                let result = app.run_display(
                    id,
                    &peer,
                    media,
                    &config,
                    true,
                    stopped.clone(),
                    remote_ip,
                    settings_received,
                    None,
                    physical_usb,
                    heartbeat,
                );
                stopped.store(true, Ordering::SeqCst);
                let _ = shutdown.shutdown(Shutdown::Both);
                result
            })();
            app.finish(id, result);
        });
        Ok(())
    }
    fn set_display_mode(&self, id: u64, mode: extend_computer_agent::session::display::Mode) {
        if let Some(job) = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_mut()
            .filter(|j| j.view.id == id)
        {
            job.view.display = Some(mode);
        }
    }
    fn set_display_config(&self, id: u64, config: &Config) {
        if let Some(job) = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_mut()
            .filter(|j| j.view.id == id)
        {
            job.view.display_config = Some(config.clone());
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn run_display(
        self: &Arc<Self>,
        id: u64,
        peer: &str,
        mut media: Transport,
        config: &Config,
        source: bool,
        stopped: Arc<AtomicBool>,
        remote_ip: std::net::IpAddr,
        settings: std::sync::mpsc::Receiver<SettingsUpdate>,
        mut helper: Option<helper::Helper>,
        physical_usb: bool,
        heartbeat: Arc<Mutex<Option<Instant>>>,
    ) -> Result<()> {
        let mut config = config.clone();
        let mut connected_at = None::<Instant>;
        let fixed_bitrate = source
            && bitrate::fixed_bitrate_requested(
                std::env::var("EXTEND_DISPLAY_FIXED_BITRATE")
                    .ok()
                    .as_deref(),
            );
        if fixed_bitrate {
            eprintln!(
                "Display fixed bitrate experiment: enabled=1 requested_bps={} source=true",
                config.bitrate
            );
            if let Some(helper) = &helper {
                // Native readout records the actual requested encoder rate.
                helper.set_bitrate(config.bitrate)?;
            }
        }
        // Declared after the capture helper so teardown releases held input
        // before the virtual display can be removed.
        let input_worker = source.then(|| {
            input::Worker::start(
                Arc::downgrade(self),
                id,
                peer.into(),
                media.text_input_negotiated(),
                media.shortcuts_negotiated(),
            )
        });
        let mut input_closed = false;
        let mut receiver_input = (!source).then(|| {
            receiver::Receiver::new(
                media.input_negotiated(),
                media.text_input_negotiated(),
                media.shortcuts_negotiated(),
            )
        });
        let mut viewer_input_state = None;
        let mut input_recovery_checkpoint = (0, None);
        let is_usb = self
            .inner
            .lock()
            .unwrap()
            .job
            .as_ref()
            .is_some_and(|j| j.view.id == id && j.view.route == Some("usb"));
        let mut wifi = if is_usb {
            None
        } else {
            extend_computer_agent::low_jitter::start_for_peer(remote_ip, true)
        };
        let mut wifi_checked = Instant::now();
        if source {
            media.set_target_bitrate(config.bitrate)?;
        }
        let _activity = crate::platform::session_activity();
        let until = Instant::now() + Duration::from_secs(15);
        let mut waiting_keyframe = false;
        let mut announced = false;
        let mut bitrate = config.bitrate;
        let mut bitrate_changed = Instant::now();
        let mut capture_paused = false;
        let mut timing = Instant::now();
        let mut previous_tick = timing;
        let mut ticks = 0u64;
        let mut max_tick = Duration::ZERO;
        let diagnostic = std::env::var_os("EXTEND_DISPLAY_METRICS").is_some();
        while !stopped.load(Ordering::SeqCst) {
            // A send late in the previous iteration may have started recovery.
            // Observe it before poll can retry successfully and clear the state.
            observe_recovery_input(
                media.usb_recovery_checkpoint(),
                &mut input_recovery_checkpoint,
                input_worker.as_ref(),
                receiver_input.as_mut(),
            )?;
            while let Ok(update) = settings.try_recv() {
                if update.suspend_input {
                    if let Some(receiver) = &mut receiver_input {
                        receiver.suspend(Instant::now());
                    }
                    let outcome = input_worker
                        .as_ref()
                        .map_or(Ok(()), |worker| worker.suspend());
                    let _ = update.reply.send(outcome.map(|()| config.clone()));
                    continue;
                }
                let outcome = (|| -> Result<Config> {
                    let native = helper.as_ref().context("Display settings are not ready")?;
                    let mut next = update.config;
                    if !source && next.auto_size {
                        native.fit_viewport(&mut next)?;
                    }
                    validate_settings(&config, &next)?;
                    if let Some(worker) = &input_worker {
                        worker.suspend()?;
                    }
                    if let Some(receiver) = &mut receiver_input {
                        receiver.reconfigure(Instant::now())?;
                        native.take_viewer_input();
                    }
                    native.configure(&next)?;
                    if source {
                        media.set_target_bitrate(next.bitrate)?;
                    }
                    config = next;
                    bitrate = config.bitrate;
                    self.set_display_config(id, &config);
                    Ok(config.clone())
                })();
                if let Err(error) = &outcome {
                    let _ = update.reply.send(Err(anyhow::anyhow!("{error:#}")));
                    return Err(anyhow::anyhow!("{error:#}"));
                }
                let _ = update.reply.send(outcome);
            }
            if diagnostic {
                let now = Instant::now();
                max_tick = max_tick.max(now.duration_since(previous_tick));
                previous_tick = now;
                ticks += 1;
                if timing.elapsed() >= Duration::from_secs(5) {
                    eprintln!(
                        "Display loop /5s: ticks={ticks} max_tick_us={}",
                        max_tick.as_micros()
                    );
                    timing = now;
                    ticks = 0;
                    max_tick = Duration::ZERO;
                }
            }
            if wifi_checked.elapsed() >= Duration::from_secs(1) {
                extend_computer_agent::low_jitter::maintain(&mut wifi);
                wifi_checked = Instant::now();
            }
            if !self
                .inner
                .lock()
                .unwrap()
                .job
                .as_ref()
                .is_some_and(|j| j.view.id == id && !j.cancelled.load(Ordering::SeqCst))
            {
                break;
            }
            if !announced {
                ensure!(
                    Instant::now() < until,
                    "Could not establish the display connection"
                );
            }
            media.update_usb_control_health(if stopped.load(Ordering::SeqCst) {
                None
            } else {
                *heartbeat.lock().unwrap()
            });
            let event = media.poll()?;
            observe_recovery_input(
                media.usb_recovery_checkpoint(),
                &mut input_recovery_checkpoint,
                input_worker.as_ref(),
                receiver_input.as_mut(),
            )?;
            match event {
                Some(DisplayEvent::Connected) => {
                    connected_at.get_or_insert_with(Instant::now);
                    if helper.is_none() {
                        let presets = self
                            .inner
                            .lock()
                            .unwrap()
                            .job
                            .as_ref()
                            .filter(|j| j.view.id == id)
                            .map(|j| j.display_presets.clone())
                            .unwrap_or_default();
                        helper = Some(helper::Helper::start(
                            &self.display_helper()?,
                            &config,
                            source,
                            physical_usb,
                            &presets,
                        )?);
                        if fixed_bitrate {
                            helper.as_ref().unwrap().set_bitrate(config.bitrate)?;
                        }
                    }
                }
                Some(DisplayEvent::Frame(frame)) if !source => {
                    if !waiting_keyframe || frame.keyframe {
                        if let Some(helper) = &helper {
                            if helper.present(frame) {
                                waiting_keyframe = false;
                            } else {
                                waiting_keyframe = true;
                                media.request_keyframe()?;
                            }
                        }
                    }
                }
                Some(DisplayEvent::Cursor(bytes)) if !source && config.separate_cursor => {
                    if let Some(helper) = &helper {
                        helper.cursor(bytes);
                    }
                }
                Some(DisplayEvent::Input(message)) if source && !input_closed => {
                    if let Some(worker) = &input_worker {
                        let target = helper
                            .as_ref()
                            .and_then(|native| native.target.lock().unwrap().clone());
                        if let Err(error) = worker.enqueue(message, target, &config) {
                            eprintln!("Display input denied: {error:#}");
                            worker.revoke();
                        }
                    }
                }
                Some(DisplayEvent::InputClosed) if source => {
                    input_closed = true;
                    if let Some(worker) = &input_worker {
                        worker.revoke();
                    }
                }
                Some(DisplayEvent::Input(message))
                    if !source
                        && !input_closed
                        && !input_quarantined(input_recovery_checkpoint, Instant::now()) =>
                {
                    if let Some(receiver) = &mut receiver_input {
                        if let Err(error) = receiver.remote(message, &config, Instant::now()) {
                            eprintln!("Display viewer input retired: {error:#}");
                        }
                    }
                }
                Some(DisplayEvent::InputClosed) if !source => {
                    input_closed = true;
                    if let Some(receiver) = &mut receiver_input {
                        receiver.close(Instant::now());
                    }
                }
                Some(DisplayEvent::Keyframe) => {
                    // Keyframes are expected while the stream starts; later
                    // ones mean data was lost and the picture had to recover.
                    if connected_at.is_some_and(|at| at.elapsed() > Duration::from_secs(3)) {
                        self.record_quality(id, super::quality::Sample::Recovery);
                    }
                    if source {
                        if let Some(helper) = &helper {
                            helper.request_keyframe()?;
                        }
                    } else {
                        waiting_keyframe = true;
                        media.request_keyframe()?;
                    }
                }
                Some(DisplayEvent::Stats { rtt, loss }) => {
                    if let Some(rtt) = rtt {
                        self.record_quality(id, super::quality::Sample::RoundTrip(rtt));
                    }
                    if let Some(loss) = loss {
                        self.record_quality(id, super::quality::Sample::Loss(loss));
                    }
                }
                Some(DisplayEvent::Bitrate(estimate))
                    if source
                        && !fixed_bitrate
                        && bitrate_changed.elapsed() >= Duration::from_secs(1) =>
                {
                    let next = (estimate.saturating_mul(85) / 100)
                        .clamp(1_000_000, config.bitrate as u64)
                        as u32;
                    if next.abs_diff(bitrate) > bitrate / 10 {
                        if let Some(helper) = &helper {
                            helper.set_bitrate(next)?;
                            bitrate = next;
                            bitrate_changed = Instant::now();
                        }
                    }
                }
                _ => {}
            }
            observe_recovery_input(
                media.usb_recovery_checkpoint(),
                &mut input_recovery_checkpoint,
                input_worker.as_ref(),
                receiver_input.as_mut(),
            )?;
            if !input_closed {
                if let Some(worker) = &input_worker {
                    if let Some(reply) = worker.reply() {
                        let sent = media.write_input(&reply)?;
                        observe_recovery_input(
                            media.usb_recovery_checkpoint(),
                            &mut input_recovery_checkpoint,
                            input_worker.as_ref(),
                            receiver_input.as_mut(),
                        )?;
                        if !sent {
                            worker.restore_reply(reply);
                        }
                    }
                }
                if let (Some(receiver), Some(native)) = (&mut receiver_input, &helper) {
                    let now = Instant::now();
                    let recovering = input_quarantined(input_recovery_checkpoint, now);
                    let (overflow, events) = native.take_viewer_input();
                    if overflow {
                        receiver.reconfigure(now)?;
                        eprintln!("Display viewer input retired: native queue overflow");
                    }
                    for event in events.into_iter().filter(|_| !recovering) {
                        if let Err(error) = receiver.native(event, now) {
                            eprintln!("Display viewer input retired: {error:#}");
                        }
                    }
                    if let Some(message) = receiver.message(now) {
                        if media.write_input(message)? {
                            receiver.accepted(now);
                        }
                    }
                }
            }
            observe_recovery_input(
                media.usb_recovery_checkpoint(),
                &mut input_recovery_checkpoint,
                input_worker.as_ref(),
                receiver_input.as_mut(),
            )?;
            if let (Some(receiver), Some(native)) = (&receiver_input, &helper) {
                let state = viewer_state_for_transport(
                    receiver,
                    input_quarantined(input_recovery_checkpoint, Instant::now()),
                );
                if viewer_input_state.as_ref() != Some(&state) {
                    if diagnostic {
                        eprintln!("Display viewer input state: available={} focused={} enabled={} revision={}",
                            state.available, state.focused, state.enabled, state.revision);
                    }
                    native.input_state(&state)?;
                    viewer_input_state = Some(state);
                }
            }
            if let Some(helper) = &helper {
                if helper.closed.load(Ordering::SeqCst) {
                    if let Some(receiver) = &mut receiver_input {
                        receiver.suspend(Instant::now());
                        if let Some(message) = receiver.message(Instant::now()) {
                            let _ = media.write_input(message);
                        }
                    }
                    return if source {
                        Err(helper.failure())
                    } else {
                        Ok(())
                    };
                }
                if source {
                    for (kind, slot) in helper.cursors.iter().enumerate() {
                        let pending = slot.lock().unwrap().take();
                        if let Some(bytes) = pending {
                            extend_computer_agent::display::metrics::cursor_event(
                                if kind == 1 {
                                    "cursor_shape_runtime_taken"
                                } else {
                                    "cursor_position_runtime_taken"
                                },
                                &bytes,
                            );
                            if !media.write_cursor(&bytes)? {
                                slot.lock().unwrap().get_or_insert(bytes);
                            }
                        }
                    }
                    observe_recovery_input(
                        media.usb_recovery_checkpoint(),
                        &mut input_recovery_checkpoint,
                        input_worker.as_ref(),
                        receiver_input.as_mut(),
                    )?;
                    let paused = media.pause_capture(config.fps)?;
                    observe_recovery_input(
                        media.usb_recovery_checkpoint(),
                        &mut input_recovery_checkpoint,
                        input_worker.as_ref(),
                        receiver_input.as_mut(),
                    )?;
                    if paused != capture_paused {
                        helper.pause_capture(paused)?;
                        capture_paused = paused;
                    }
                    if helper.keyframe.swap(false, Ordering::SeqCst) {
                        helper.request_keyframe()?;
                    }
                    if let Ok(frame) = helper.frames.try_recv() {
                        let sent = media.write(frame)?;
                        if sent && !announced {
                            self.stage(id, Phase::Connected, Some(peer));
                            announced = true;
                        }
                    }
                } else if helper.keyframe.swap(false, Ordering::SeqCst) {
                    waiting_keyframe = true;
                    media.request_keyframe()?;
                }
                if !source && !announced && helper.rendering_ready() {
                    eprintln!("Display video decoded and GPU rendered by Metal");
                    self.stage(id, Phase::Connected, Some(peer));
                    announced = true;
                }
            }
            observe_recovery_input(
                media.usb_recovery_checkpoint(),
                &mut input_recovery_checkpoint,
                input_worker.as_ref(),
                receiver_input.as_mut(),
            )?;
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(receiver) = &mut receiver_input {
            receiver.suspend(Instant::now());
            if let Some(message) = receiver.message(Instant::now()) {
                let _ = media.write_input(message);
            }
        }
        Ok(())
    }
}
