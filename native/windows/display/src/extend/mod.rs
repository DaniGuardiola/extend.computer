//! `extend`: adds a virtual monitor through the extend.computer display driver
//! (`native/windows/driver`), makes it its own part of the desktop and streams
//! what Windows draws on it, the way the macOS helper extends onto its
//! virtual display. Windows composites the pointer into the monitor's frames,
//! since the driver has no hardware cursor.
//!
//! The monitor belongs to this process's driver handle: stop, the end of
//! stdin and the parent's exit remove it, and if the helper dies the driver
//! departs it when the handle closes. The extended displays of one desktop
//! session take turns changing the display layout.
#![cfg_attr(not(windows), allow(dead_code))]
pub mod plan;
#[cfg(windows)]
mod topology;

#[cfg(windows)]
pub use source::{available, Extend, Stopped};

#[cfg(windows)]
mod source {
    use super::{
        plan::{self, Rect},
        topology::{self, Config, Layout, Target},
    };
    use crate::{
        capture::{texture, video_device, Converter},
        display_driver::{
            errors, win32_code, Acquire, AddMonitorIn, AddMonitorOut, Driver, Mode,
            QueryMonitorOut, SharedSurface, FLAG_PRESENT, FLAG_SURFACE, PROTOCOL_VERSION,
        },
    };
    use anyhow::{bail, ensure, Context, Result};
    use extend_computer_agent::{
        helper::display::{Configuration, Launch},
        session::display::CONFIG_LIMITS,
    };
    use std::{
        cell::OnceCell,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, OnceLock,
        },
        time::{Duration, Instant},
    };
    use windows::{
        core::w,
        Win32::{
            Foundation::{HANDLE, LUID, POINT, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0},
            Graphics::{
                Direct3D11::{ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_USAGE_DEFAULT},
                Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4},
                Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL},
            },
            System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
            UI::{
                HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
                Input::KeyboardAndMouse::{
                    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
                },
            },
        },
    };

    /// How long Windows may take to activate an arriving monitor by itself.
    const ARRIVAL: Duration = Duration::from_secs(2);
    /// How long a layout change may take to show. The app waits 5 s for a
    /// configuration, which may replace the monitor.
    const SETTLE: Duration = Duration::from_secs(3);
    /// How many times a layout change is tried, reading the layout afresh.
    const CHANGE_TRIES: u32 = 3;
    /// How long Windows may take to show a new scale.
    const RESCALE: Duration = Duration::from_millis(500);
    /// How long Windows may take to draw on a placed monitor.
    const FIRST_SURFACE: Duration = Duration::from_secs(5);
    /// How long a removed monitor may take to leave the layout. The app
    /// gives a stopped source 2 s before it kills it.
    const DEPART: Duration = Duration::from_millis(1200);
    /// How long a departing monitor may keep its serial, as when the
    /// previous session to the same device is still ending.
    const SERIAL_FREED: Duration = Duration::from_secs(1);
    /// How often the driver is asked about the monitor while frames flow:
    /// a replaced texture can go quiet instead of reporting abandonment.
    const RECHECK: Duration = Duration::from_secs(1);
    /// How often it is asked while no frame comes, as on an idle desktop or
    /// a texture replaced as Windows finishes setting the monitor up.
    const QUIET_RECHECK: Duration = Duration::from_millis(200);
    /// How often it is asked while the monitor has no texture.
    const RETRY: Duration = Duration::from_millis(50);
    /// How often the layout is checked for the monitor.
    const LAYOUT_CHECK: Duration = Duration::from_millis(500);
    /// How long the monitor may be off the desktop, duplicate another one or
    /// have another size, before it goes back.
    const OFF_DESKTOP: Duration = Duration::from_secs(1);
    /// How long a starting extended display waits for its turn to change
    /// the layout before it goes ahead anyway: another one's placement
    /// takes about a second, at most about six.
    const START_TURN: Duration = Duration::from_secs(8);
    /// The same for a configuration, which the app waits 5 s for.
    const CONFIGURE_TURN: Duration = Duration::from_secs(1);
    /// The same for a monitor that leaves, which the app gives 2 s.
    const LEAVE_TURN: Duration = Duration::from_millis(500);

    const REMOVED: &str = "The extended display was removed.";
    const DRIVER_GONE: &str = "The extend.computer display driver stopped.";

    /// Stop, the end of stdin or the parent's exit came while the extended
    /// display was being set up, which then ends at once.
    #[derive(Debug)]
    pub struct Stopped;

    impl std::fmt::Display for Stopped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Stopped while setting up the extended display")
        }
    }

    impl std::error::Error for Stopped {}

    fn running(stop: &AtomicBool) -> Result<()> {
        if stop.load(Ordering::Relaxed) {
            bail!(Stopped);
        }
        Ok(())
    }

    /// The turn of one extended display of this desktop session to change
    /// the layout. Each reads the layout, changes its own monitor and passes
    /// every other one back as it read it, so two changes at once would undo
    /// each other, and Windows refuses one that passes back a monitor that
    /// arrived meanwhile, which it has made a duplicate across adapters.
    struct Turn(Option<HANDLE>);

    impl Turn {
        /// Waits up to `timeout` for the turn; `None` if another extended
        /// display still has it then, or once `stop` is set.
        fn take(timeout: Duration, stop: Option<&AtomicBool>) -> Option<Self> {
            static LOCK: OnceLock<Option<usize>> = OnceLock::new();
            let lock = *LOCK.get_or_init(|| {
                unsafe { CreateMutexW(None, false, w!(r"Local\ExtendComputerDisplayLayout")) }
                    .ok()
                    .map(|lock| lock.0 as usize)
            });
            // Without the lock, changes go ahead as they come.
            let Some(lock) = lock.map(|lock| HANDLE(lock as _)) else {
                return Some(Self(None));
            };
            let deadline = Instant::now() + timeout;
            loop {
                let slice = deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50));
                match unsafe { WaitForSingleObject(lock, slice.as_millis() as u32) } {
                    // A turn whose holder exited without giving it back is
                    // this one's.
                    WAIT_OBJECT_0 | WAIT_ABANDONED => return Some(Self(Some(lock))),
                    WAIT_FAILED => return Some(Self(None)),
                    _ => {}
                }
                if Instant::now() >= deadline
                    || stop.is_some_and(|stop| stop.load(Ordering::Relaxed))
                {
                    return None;
                }
            }
        }
    }

    impl Drop for Turn {
        fn drop(&mut self) {
            if let Some(lock) = self.0 {
                let _ = unsafe { ReleaseMutex(lock) };
            }
        }
    }

    /// Where the monitor is on the desktop, or was.
    #[derive(Clone, Copy, Debug)]
    enum Spot {
        /// It extends the desktop there.
        At(Rect),
        /// It did, before Windows took it off the desktop or replaced it.
        Was(Rect),
        New,
    }

    impl Spot {
        fn now(rect: Option<Rect>, was: Option<Rect>) -> Self {
            rect.map(Self::At)
                .or(was.map(Self::Was))
                .unwrap_or(Self::New)
        }
    }

    /// Whether the display driver is installed, this user can open it, and
    /// this session can arrange its monitors.
    pub fn available() -> bool {
        Driver::open()
            .and_then(|driver| driver.info())
            .is_ok_and(|info| info.protocol_version == PROTOCOL_VERSION)
            && Config::active().is_ok()
    }

    fn open() -> Result<Driver> {
        let driver = Driver::open().map_err(|error| match win32_code(&error) {
            Some(errors::NOT_FOUND) => anyhow::anyhow!(
                "Extending this PC's desktop needs the extend.computer display driver. Install it, then reconnect."
            ),
            Some(errors::ACCESS_DENIED) => anyhow::anyhow!(
                "The extend.computer display driver only opens for the signed-in user."
            ),
            _ => anyhow::Error::new(error).context("Could not open the extend.computer display driver"),
        })?;
        let info = driver.info().context(DRIVER_GONE)?;
        ensure!(
            info.protocol_version == PROTOCOL_VERSION,
            "The extend.computer display driver {} doesn't match this app. Update extend.computer.",
            info.driver_version()
        );
        Ok(driver)
    }

    fn wait<T>(timeout: Duration, mut ready: impl FnMut() -> Option<T>) -> Option<T> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(value) = ready() {
                return Some(value);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The ADD request for a monitor with `modes`, the preferred one first.
    /// Its EDID, with the physical size of the preferred mode at 96 points
    /// per inch, depends only on the serial and the frame rate.
    fn request(modes: &[(u32, u32)], fps: u32, serial: u32) -> AddMonitorIn {
        let (width, height) = modes[0];
        let fallbacks: Vec<Mode> = modes[1..]
            .iter()
            .map(|&(width, height)| Mode::new(width, height, fps))
            .collect();
        AddMonitorIn::new(Mode::new(width, height, fps), serial)
            .with_fallbacks(&fallbacks)
            .with_physical_size_mm(plan::millimetres(width), plan::millimetres(height))
    }

    /// A device for the video processor and encoders on the GPU that renders
    /// the monitor, where its shared texture lives.
    fn render_device(luid: LUID) -> Result<windows::Win32::Graphics::Direct3D11::ID3D11Device> {
        let adapter: IDXGIAdapter = unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
            factory.EnumAdapterByLuid(luid)?
        };
        Ok(video_device(&adapter)?.0)
    }

    /// Wakes a display that went to sleep, on which Windows draws nothing.
    /// `ES_DISPLAY_REQUIRED` keeps it awake but doesn't wake it; a mouse
    /// move that goes nowhere, input like any other, does.
    fn wake_display() {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_MOVE,
                    ..Default::default()
                },
            },
        };
        unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    }

    fn scale_at(rect: Rect) -> Option<u32> {
        let center = POINT {
            x: rect.x + rect.width as i32 / 2,
            y: rect.y + rect.height as i32 / 2,
        };
        unsafe {
            let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONULL);
            if monitor.is_invalid() {
                return None;
            }
            let (mut x, mut y) = (0, 0);
            GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).ok()?;
            Some(x * 100 / 96)
        }
    }

    fn report(rect: Rect) {
        eprintln!(
            "Extended display: {}x{} at {},{}, {}% scale",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            scale_at(rect).map_or("unknown".into(), |scale| scale.to_string())
        );
    }

    /// One virtual monitor, owned by the driver handle that added it.
    struct Monitor {
        driver: Driver,
        added: AddMonitorOut,
        serial: u32,
        /// The refresh rate of its modes.
        refresh: u32,
        /// The monitor's device path, once Windows has set the monitor up.
        device: OnceCell<String>,
        /// Set once the helper should stop, which cuts setting it up short.
        stop: Arc<AtomicBool>,
    }

    /// Adds a monitor, moving to the next serial while another extended
    /// display on this PC has it. Returns the serial it got.
    fn add_monitor(driver: &Driver, mut request: AddMonitorIn) -> Result<(AddMonitorOut, u32)> {
        let (asked, started) = (request.serial_seed, Instant::now());
        let mut bumps = 0;
        loop {
            match driver.add_monitor(&request) {
                Ok(added) => {
                    if request.serial_seed != asked {
                        eprintln!(
                            "Extended display serial {asked} is in use; this one has {}",
                            request.serial_seed
                        );
                    }
                    return Ok((added, request.serial_seed));
                }
                // A departing monitor keeps its serial until it is gone.
                Err(error)
                    if win32_code(&error) == Some(errors::ALREADY_EXISTS)
                        && started.elapsed() < SERIAL_FREED =>
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) if win32_code(&error) == Some(errors::ALREADY_EXISTS) => {
                    bumps += 1;
                    ensure!(
                        bumps < 4,
                        "Every extended display serial for this device is in use."
                    );
                    request.serial_seed = request.serial_seed.wrapping_add(1).max(1);
                }
                Err(error) if win32_code(&error) == Some(errors::NO_SYSTEM_RESOURCES) => {
                    bail!("This PC already shows as many extended displays as it can.")
                }
                Err(error) => {
                    return Err(error)
                        .context("The display driver could not add the extended display")
                }
            }
        }
    }

    impl Monitor {
        fn add(driver: Driver, request: AddMonitorIn, stop: Arc<AtomicBool>) -> Result<Self> {
            let (added, serial) = add_monitor(&driver, request)?;
            Ok(Self {
                driver,
                added,
                serial,
                refresh: request.refresh_hz,
                device: OnceCell::new(),
                stop,
            })
        }

        fn target(&self) -> Target {
            let (adapter, id) = (self.added.os_adapter_luid(), self.added.os_target_id);
            let monitor = self.device.get().cloned().or_else(|| {
                let path = topology::monitor_path(adapter, id)?;
                Some(self.device.get_or_init(|| path).clone())
            });
            Target {
                adapter,
                id,
                monitor,
                refresh: self.refresh,
            }
        }

        fn query(&self) -> Result<QueryMonitorOut> {
            match self.driver.query_monitor(self.added.monitor_id) {
                Ok(query) if query.has(FLAG_PRESENT) => Ok(query),
                Ok(_) => bail!(REMOVED),
                Err(error) if win32_code(&error) == Some(errors::NOT_FOUND) => bail!(REMOVED),
                Err(error) => Err(error).context(DRIVER_GONE),
            }
        }

        /// The monitor's place on the desktop while it extends it on its own.
        fn extension(&self) -> Result<Option<Rect>> {
            Ok(Config::active()?
                .find(&self.target())
                .filter(|(_, shared)| !shared)
                .map(|(rect, _)| rect))
        }

        /// Makes a new monitor its own part of the desktop at `size`, with
        /// the monitors of `earlier` that Windows switched off as it arrived.
        /// Windows gives a monitor it knows the place it had, and may extend
        /// onto a new one by itself; that place is kept. Otherwise,
        /// duplicated or left off, it goes where the monitor it replaces
        /// `was`, or right of the desktop.
        fn place(
            &self,
            size: (u32, u32),
            scale: u32,
            earlier: Option<&Config>,
            was: Option<Rect>,
        ) -> Result<Rect> {
            let arrived = wait(ARRIVAL, || {
                match Config::active().ok()?.find(&self.target()) {
                    Some(found) => Some(Some(found)),
                    None => self.stop.load(Ordering::Relaxed).then_some(None),
                }
            })
            .flatten();
            running(&self.stop)?;
            match arrived {
                Some((rect, false)) => eprintln!(
                    "Extended display: Windows extended the desktop onto it at {}x{} at {},{}",
                    rect.width, rect.height, rect.x, rect.y
                ),
                Some((_, true)) => {
                    eprintln!("Extended display: Windows duplicated another onto it")
                }
                None => eprintln!("Extended display: Windows left it off the desktop"),
            }
            let arrived = arrived.filter(|(_, shared)| !shared).map(|(rect, _)| rect);
            self.arrange(size, scale, Spot::now(arrived, was), earlier)
        }

        /// Switches the monitor to `size`, among the modes it offers, or puts
        /// it back on the desktop where it `was`.
        fn resize(&self, size: (u32, u32), scale: u32, was: Option<Rect>) -> Result<Rect> {
            let now = self.extension().ok().flatten();
            self.arrange(size, scale, Spot::now(now, was), None)
        }

        /// Puts the monitor at `size` where it is or was, keeping the edges
        /// that touch other monitors, or right of the desktop, then sets its
        /// scale: 100%, or 200% with Retina, where Windows allows it.
        fn arrange(
            &self,
            size: (u32, u32),
            scale: u32,
            spot: Spot,
            earlier: Option<&Config>,
        ) -> Result<Rect> {
            let target = self.target();
            let layout = Layout::read(&target, earlier)?;
            let rect = match spot {
                // Where Windows put it, at the right size: as it remembered
                // it for this peer, or as the user arranged it.
                Spot::At(rect)
                    if (rect.width, rect.height) == size
                        && layout.signal(&target) == Some(size)
                        && !layout.restores() =>
                {
                    rect
                }
                _ => {
                    let old = match spot {
                        Spot::At(rect) | Spot::Was(rect) => Some(rect),
                        Spot::New => None,
                    };
                    let mut layout = layout;
                    let mut tries = 1;
                    loop {
                        let origin = plan::origin(old, size, &layout.others());
                        match layout.extend(&target, size, origin) {
                            Ok(0) => break,
                            Ok(left_out) => {
                                eprintln!(
                                    "Extended display: Windows refused the layout with {left_out} other display duplicating another across adapters, which leaves the desktop"
                                );
                                break;
                            }
                            // The layout may have changed since it was read,
                            // as when another extended display went ahead
                            // without its turn.
                            Err(_) if tries < CHANGE_TRIES => {
                                tries += 1;
                                std::thread::sleep(Duration::from_millis(200));
                                layout = Layout::read(&target, earlier)?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let rect = wait(SETTLE, || {
                        self.extension()
                            .ok()
                            .flatten()
                            .filter(|rect| (rect.width, rect.height) == size)
                    })
                    .context("Windows did not add the extended display to the desktop.")?;
                    if let Some(signal) = Config::active().ok().and_then(|c| c.signal(&target)) {
                        if signal != size {
                            eprintln!(
                                "Extended display: Windows scales it to {}x{}",
                                signal.0, signal.1
                            );
                        }
                    }
                    rect
                }
            };
            let percent = scale * 100;
            match topology::set_scale(&target, percent) {
                Ok(rescaled) => {
                    if rescaled.percent != percent {
                        eprintln!(
                            "Extended display scale: Windows allows no {percent}% at {}x{}; the nearest it allows is {}%",
                            size.0, size.1, rescaled.percent
                        );
                    }
                    if rescaled.changed {
                        wait(RESCALE, || {
                            (scale_at(rect) == Some(rescaled.percent)).then_some(())
                        });
                    }
                }
                Err(error) => eprintln!("Extended display scale: {error:#}"),
            }
            Ok(rect)
        }

        /// Waits for the monitor's first shared texture, preferably at `size`.
        fn first_surface(&self, size: (u32, u32)) -> Result<QueryMonitorOut> {
            let started = Instant::now();
            loop {
                running(&self.stop)?;
                let query = self.query()?;
                if query.has(FLAG_SURFACE)
                    && ((query.width, query.height) == size
                        || started.elapsed() > FIRST_SURFACE / 2)
                {
                    if (query.width, query.height) != size {
                        eprintln!(
                            "Extended display texture: {}x{} for a {}x{} mode",
                            query.width, query.height, size.0, size.1
                        );
                    }
                    return Ok(query);
                }
                ensure!(
                    started.elapsed() < FIRST_SURFACE,
                    "Windows did not start drawing the extended display."
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// Replaces the monitor with one offering other modes, at `size`,
        /// where it is or `was`.
        fn replace(
            &mut self,
            request: AddMonitorIn,
            size: (u32, u32),
            scale: u32,
            was: Option<Rect>,
        ) -> Result<Rect> {
            let earlier = Config::active().ok();
            let was = self.extension().ok().flatten().or(was);
            self.remove();
            (self.added, self.serial) = add_monitor(&self.driver, request)?;
            self.device = OnceCell::new();
            self.place(size, scale, earlier.as_ref(), was)
        }

        fn remove(&mut self) {
            let target = self.target();
            match self.driver.remove_monitor(self.added.monitor_id) {
                Ok(()) => {}
                Err(error) if win32_code(&error) == Some(errors::NOT_FOUND) => {}
                Err(error) => eprintln!("Extended display removal: {error}"),
            }
            wait(DEPART, || {
                Config::active()
                    .ok()
                    .filter(|config| config.find(&target).is_none())
            });
        }
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            let _turn = Turn::take(LEAVE_TURN, None);
            let target = self.target();
            let earlier = Config::active().ok();
            self.remove();
            let Some(earlier) = earlier else { return };
            // Windows rearranges the other monitors as this one leaves. One
            // it switched off then, still connected, comes back as it was;
            // anything else stays as Windows or the user left it.
            let settled = wait(Duration::from_millis(300), || {
                Layout::read(&target, Some(&earlier))
                    .ok()
                    .filter(|layout| !layout.restores())
            });
            if settled.is_none() {
                match Layout::read(&target, Some(&earlier)).map(|layout| layout.restore()) {
                    Ok(0) => eprintln!("Display layout restored"),
                    Ok(status) => eprintln!("Display layout not restored ({status})"),
                    Err(error) => eprintln!("Display layout: {error:#}"),
                }
            }
        }
    }

    /// The `extend` source: frames come from the monitor's keyed-mutex shared
    /// texture, which the driver fills whenever Windows draws on it.
    pub struct Extend {
        surface: Option<SharedSurface>,
        /// The texture generation that last failed to open, logged once.
        unopened: Option<u32>,
        /// The latest frame, copied out of the shared texture so an idle
        /// desktop can still be encoded.
        image: Option<ID3D11Texture2D>,
        converter: Converter,
        /// The GPU of `converter`'s device.
        luid: LUID,
        queried: Instant,
        /// When the last frame came.
        drawn: Instant,
        /// When the layout was last checked, and since when the monitor
        /// hasn't extended the desktop at its size.
        checked: Instant,
        off_since: Option<Instant>,
        /// Where it last extended the desktop at its size.
        placed: Option<Rect>,
        /// Another size Windows or the user gave it, which it couldn't be
        /// put back from.
        unfixed: Option<(u32, u32)>,
        size: (u32, u32),
        scale: u32,
        fps: u32,
        modes: Vec<(u32, u32)>,
        presets: Vec<(u32, u32)>,
        frames: u64,
        /// Why the monitor is gone after a failed replacement, which ends
        /// the stream.
        broken: Option<String>,
        /// Declared last, so the monitor departs after the textures are released.
        monitor: Monitor,
    }

    impl Extend {
        /// Adds and places the monitor and waits for its first texture.
        /// Fails with [`Stopped`] once `stop` is set.
        pub fn start(launch: &Launch, stop: Arc<AtomicBool>) -> Result<Self> {
            let scale = if launch.retina { 2 } else { 1 };
            let size = (
                plan::even(launch.width * scale),
                plan::even(launch.height * scale),
            );
            ensure!(
                size.0 <= CONFIG_LIMITS.encoded && size.1 <= CONFIG_LIMITS.encoded,
                "Video capture size exceeds supported limit."
            );
            let presets = plan::presets(std::env::var(plan::PRESETS).ok().as_deref());
            let serial = plan::serial(std::env::var(plan::SERIAL).ok().as_deref());
            let driver = open()?;
            // Outside the signed-in desktop session (a service, ssh) Windows
            // doesn't let the helper arrange the monitors.
            const SESSION: &str = "Extending works only in the signed-in user's desktop session";
            Config::active().context(SESSION)?;
            wake_display();
            let modes = plan::modes(size, scale, &presets);
            // From adding the monitor until it is placed: Windows makes a
            // monitor that arrives meanwhile a duplicate.
            let turn = Turn::take(START_TURN, Some(&stop));
            running(&stop)?;
            if turn.is_none() {
                eprintln!(
                    "Extended display: another one is still changing the display layout; going ahead"
                );
            }
            let earlier = Config::active().context(SESSION)?;
            let monitor = Monitor::add(driver, request(&modes, launch.fps, serial), stop)?;
            let placed = monitor.place(size, scale, Some(&earlier), None)?;
            drop(turn);
            report(placed);
            let query = monitor.first_surface(size)?;
            let luid = query.render_luid();
            let converter = Converter::new(&render_device(luid)?, size.0, size.1)?;
            let mut extend = Self {
                surface: None,
                unopened: None,
                image: None,
                converter,
                luid,
                queried: Instant::now(),
                drawn: Instant::now(),
                checked: Instant::now(),
                off_since: None,
                placed: Some(placed),
                unfixed: None,
                size,
                scale,
                fps: launch.fps,
                modes,
                presets,
                frames: 0,
                broken: None,
                monitor,
            };
            extend.refresh()?;
            Ok(extend)
        }

        pub fn device(&self) -> &windows::Win32::Graphics::Direct3D11::ID3D11Device {
            self.converter.device()
        }

        /// The encoded size and whether it is Retina.
        pub fn size(&self) -> ((u32, u32), bool) {
            (self.size, self.scale == 2)
        }

        /// Asks the driver for the monitor's current texture and opens it when
        /// it was replaced, on a new device when Windows moved the monitor to
        /// another GPU.
        fn refresh(&mut self) -> Result<()> {
            self.queried = Instant::now();
            let query = self.monitor.query()?;
            if !query.has(FLAG_SURFACE) {
                // Off the desktop (Windows display settings or Win+P), asleep,
                // locked, or between swap chains: the stream repeats the last
                // frame.
                self.surface = None;
                return Ok(());
            }
            if query.render_luid() != self.luid {
                self.surface = None;
                self.image = None;
                let device = render_device(query.render_luid())?;
                self.converter = Converter::new(&device, self.size.0, self.size.1)?;
                self.luid = query.render_luid();
            }
            if let Err(error) =
                SharedSurface::reopen_if_stale(&mut self.surface, self.converter.device(), &query)
            {
                if self.unopened != Some(query.surface_generation) {
                    eprintln!("Extended display texture: {error:#}");
                    self.unopened = Some(query.surface_generation);
                }
            }
            Ok(())
        }

        /// Puts the monitor back when Windows took it off the desktop or made
        /// it a duplicate for a while, as when another monitor arrives or
        /// after Win+P, where it was; and to its size when Windows or the
        /// user gave it another one, whose frames would be stretched. While
        /// it extends the desktop at its size, asleep or locked, it stays as
        /// it is.
        fn keep_placed(&mut self) -> Result<()> {
            if self.checked.elapsed() < LAYOUT_CHECK {
                return Ok(());
            }
            self.checked = Instant::now();
            let size = self.size;
            // The layout is out of reach for now.
            let Ok(now) = self.monitor.extension() else {
                return Ok(());
            };
            match now {
                Some(rect) if (rect.width, rect.height) == size => {
                    (self.placed, self.unfixed, self.off_since) = (Some(rect), None, None);
                    return Ok(());
                }
                Some(rect) if self.unfixed == Some((rect.width, rect.height)) => return Ok(()),
                _ => {}
            }
            let since = *self.off_since.get_or_insert_with(Instant::now);
            if since.elapsed() < OFF_DESKTOP {
                return Ok(());
            }
            // Another extended display is changing the layout: look again
            // after it.
            let Some(_turn) = Turn::take(Duration::ZERO, None) else {
                return Ok(());
            };
            match self.monitor.extension() {
                Ok(Some(rect)) if (rect.width, rect.height) == size => {
                    (self.placed, self.off_since) = (Some(rect), None);
                }
                Ok(Some(rect)) => {
                    eprintln!(
                        "Extended display: Windows changed it to {}x{}, putting it back to {}x{}",
                        rect.width, rect.height, size.0, size.1
                    );
                    match self.monitor.arrange(size, self.scale, Spot::At(rect), None) {
                        Ok(rect) => {
                            report(rect);
                            self.placed = Some(rect);
                        }
                        // The frames are scaled to the stream meanwhile.
                        Err(error) => {
                            eprintln!("Extended display: {error:#}");
                            self.unfixed = Some((rect.width, rect.height));
                        }
                    }
                    (self.off_since, self.surface) = (None, None);
                }
                Ok(None) => {
                    eprintln!("Extended display: off the desktop, putting it back");
                    let spot = Spot::now(None, self.placed);
                    let rect = self
                        .monitor
                        .arrange(size, self.scale, spot, None)
                        .context("Windows took the extended display off the desktop.")?;
                    report(rect);
                    (self.placed, self.off_since, self.surface) = (Some(rect), None, None);
                }
                Err(_) => {}
            }
            Ok(())
        }

        /// Waits up to `timeout` for Windows to draw a new frame. Returns
        /// whether one came; the latest stays available.
        pub fn next_frame(&mut self, timeout: Duration) -> Result<bool> {
            if let Some(error) = &self.broken {
                bail!("{error}");
            }
            let due = match &self.surface {
                Some(_) if self.drawn.elapsed() < QUIET_RECHECK => RECHECK,
                Some(_) => QUIET_RECHECK,
                None => RETRY,
            };
            if self.queried.elapsed() >= due {
                self.refresh()?;
            }
            self.keep_placed()?;
            let Some(surface) = &self.surface else {
                std::thread::sleep(timeout.min(Duration::from_millis(100)));
                return Ok(false);
            };
            match surface.acquire(timeout.as_millis() as u32) {
                Acquire::Frame => {
                    let copied = copy(&mut self.image, &mut self.converter, surface);
                    // The copy queued before the release is ordered before
                    // the driver's next write.
                    let released = surface.release();
                    copied?;
                    if released.is_err() {
                        self.surface = None;
                    }
                    self.frames += 1;
                    self.drawn = Instant::now();
                    Ok(true)
                }
                // IddCx presents only on change; an idle desktop times out.
                Acquire::Timeout => Ok(false),
                Acquire::Abandoned | Acquire::Failed(_) => {
                    self.surface = None;
                    Ok(false)
                }
            }
        }

        /// Converts the latest frame to NV12. Returns the ring slot holding it.
        pub fn convert(&mut self) -> Result<Option<usize>> {
            let Some(image) = self.image.clone() else {
                return Ok(None);
            };
            self.converter.convert(&image).map(Some)
        }

        pub fn texture(&self, slot: usize) -> &ID3D11Texture2D {
            self.converter.texture(slot)
        }

        pub fn read(&mut self, slot: usize) -> Result<&[u8]> {
            self.converter.read(slot)
        }

        /// Applies a configuration and returns the new encoded size. A size
        /// the monitor offers is a mode switch; another one, or a frame rate
        /// above the monitor's refresh rate, replaces the monitor, and if
        /// that fails the stream ends. The scale follows Retina; a lower
        /// frame rate only paces encoding.
        pub fn configure(&mut self, configuration: &Configuration) -> Result<(u32, u32)> {
            let scale = if configuration.retina { 2 } else { 1 };
            let size = (
                plan::even(configuration.width * scale),
                plan::even(configuration.height * scale),
            );
            ensure!(
                size.0 <= CONFIG_LIMITS.encoded && size.1 <= CONFIG_LIMITS.encoded,
                "Video capture size exceeds supported limit."
            );
            let refresh = plan::refresh_rate(self.fps, configuration.fps);
            let offered = self.modes.contains(&size) && refresh == self.fps;
            let _turn = (!offered || (size, scale) != (self.size, self.scale))
                .then(|| Turn::take(CONFIGURE_TURN, None));
            if offered {
                if (size, scale) != (self.size, self.scale) {
                    let rect = self.monitor.resize(size, scale, self.placed)?;
                    report(rect);
                    self.placed = Some(rect);
                }
            } else {
                self.surface = None;
                self.unopened = None;
                self.image = None;
                let modes = plan::modes(size, scale, &self.presets);
                let request = request(&modes, refresh, self.monitor.serial);
                match self.monitor.replace(request, size, scale, self.placed) {
                    Ok(rect) => {
                        report(rect);
                        self.placed = Some(rect);
                    }
                    Err(error) => {
                        self.broken = Some(format!("{error:#}"));
                        return Err(error);
                    }
                }
                self.modes = modes;
                self.fps = refresh;
            }
            self.scale = scale;
            if size != self.size {
                let source = self.converter.source_size();
                self.converter = Converter::new(self.converter.device(), size.0, size.1)?;
                if self.image.is_some() {
                    self.converter.set_source(source)?;
                }
                self.size = size;
            }
            // Pick up the new texture right away.
            self.surface = None;
            (self.off_since, self.unfixed) = (None, None);
            Ok(size)
        }
    }

    impl Drop for Extend {
        fn drop(&mut self) {
            eprintln!("Extended display: {} frames from the driver", self.frames);
        }
    }

    /// Copies a frame out of the shared texture into `image`, recreating it
    /// when the texture's size or format changed.
    fn copy(
        image: &mut Option<ID3D11Texture2D>,
        converter: &mut Converter,
        surface: &SharedSurface,
    ) -> Result<()> {
        let desc = *surface.desc();
        let current = image.as_ref().map(|image| {
            let mut current = Default::default();
            unsafe { image.GetDesc(&mut current) };
            current
        });
        if current.is_none_or(
            |current: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC| {
                (current.Width, current.Height, current.Format)
                    != (desc.Width, desc.Height, desc.Format)
            },
        ) {
            *image = Some(texture(
                converter.device(),
                desc.Width,
                desc.Height,
                desc.Format,
                D3D11_USAGE_DEFAULT,
                // The video processor reads it.
                D3D11_BIND_RENDER_TARGET.0 as u32,
                0,
                0,
            )?);
            converter.set_source((desc.Width, desc.Height))?;
        }
        unsafe {
            converter
                .context()
                .CopyResource(image.as_ref().unwrap(), surface.texture())
        };
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::display_driver::{as_bytes, MAX_MODES};

        #[test]
        fn the_request_keeps_the_identity_and_offers_the_stream_size() {
            let modes = plan::modes((2388, 1668), 2, &[(1366, 1024), (1920, 1080)]);
            let tablet = request(&modes, 60, 7);
            // The EDID's preferred mode and physical size are the same
            // whatever the stream: 1920x1080 at 96 points per inch.
            assert_eq!(
                (tablet.width, tablet.height, tablet.refresh_hz),
                (1920, 1080, 60)
            );
            assert_eq!(
                (tablet.physical_width_mm, tablet.physical_height_mm),
                (508, 286)
            );
            assert_eq!(tablet.serial_seed, 7);
            assert_eq!(tablet.fallback_mode_count, 3);
            assert_eq!(
                tablet.fallback_modes[..3],
                [
                    Mode::new(2388, 1668, 60),
                    Mode::new(2732, 2048, 60),
                    Mode::new(3840, 2160, 60)
                ]
            );
            assert_eq!(as_bytes(&tablet).len(), 132);
            // At most eight fallbacks, as the driver takes.
            let presets: Vec<_> = (0..8).map(|n| (1024 + n * 16, 768)).collect();
            let many = request(&plan::modes((1366, 1024), 1, &presets), 60, 7);
            assert_eq!(many.fallback_mode_count as usize, MAX_MODES);
            assert_eq!(many.fallback_modes[0], Mode::new(1366, 1024, 60));
        }
    }
}
