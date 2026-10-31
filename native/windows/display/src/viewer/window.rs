//! The viewer's window and threads. The window thread owns the window, the
//! swap chain and input; a second thread reads stdin and decodes frames,
//! handing the newest picture over; a third writes stdout (see `output`).
use super::{
    decode::{self, Decoder, Mode, Picture},
    fit::{self, Area},
    input::{self, Capabilities, Clicks, Model, Produces, Typing},
    output::Writer,
    present::Presenter,
    protocol::{self, Inbound, Presented},
};
use crate::keys::{self, CONTROL, OPTION};
use anyhow::{ensure, Context, Result};
use extend_computer_agent::{
    display::input::Point,
    helper::{
        display::{HelperMessage, Launch, NativeEvent, RenderReady, Startup, ViewerCommand},
        packet::{read_packet, Kind},
    },
    session::display::VideoCodec,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::{Direct3D11::ID3D11Device, Gdi::*},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            LibraryLoader::GetModuleHandleW,
            Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
            },
            RemoteDesktop::{
                WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
                NOTIFY_FOR_THIS_SESSION,
            },
            Threading::{
                GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess,
                WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE,
            },
        },
        UI::{
            Input::{
                Ime::{
                    ImmAssociateContextEx, ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow,
                    CANDIDATEFORM, CFS_CANDIDATEPOS, HIMC, IACE_DEFAULT,
                },
                KeyboardAndMouse::*,
            },
            WindowsAndMessaging::*,
        },
    },
};

/// Posted to the window when the reader thread has a picture or a command.
const WM_VIEWER: u32 = WM_APP + 1;
/// How long the reader or the parent watch waits for the window to close
/// before ending the process itself.
const EXIT_GRACE: Duration = Duration::from_millis(1500);
/// Ctrl+Alt+Esc, registered while the viewer is the active window.
const EXIT_HOTKEY: i32 = 1;

/// What the reader thread hands the window.
struct Shared {
    latest: Mutex<Option<Picture>>,
    commands: Mutex<VecDeque<ViewerCommand>>,
    error: Mutex<Option<anyhow::Error>>,
    stop: AtomicBool,
    posted: AtomicBool,
    window: isize,
}
impl Shared {
    fn post(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.window as _)),
                WM_VIEWER,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    /// Wakes the window, once until it handles the wake.
    fn wake(&self) {
        if !self.posted.swap(true, Ordering::SeqCst) {
            self.post();
        }
    }

    fn finish(&self, error: Option<anyhow::Error>) {
        if let Some(error) = error {
            self.error.lock().unwrap().get_or_insert(error);
        }
        self.stop.store(true, Ordering::SeqCst);
        self.post();
    }
}

pub fn run(launch: Launch) -> Result<()> {
    ensure!(
        launch.codec == VideoCodec::H264,
        "Only H.264 is supported on Windows."
    );
    let writer = Writer::spawn(std::io::stdout());
    let (monitor, area) = primary_monitor()?;
    let device = decode::device(decode::adapter_for(monitor).as_ref())?;
    eprintln!("Display viewer adapter: {}", decode::adapter_name(&device));
    let hwnd = create_window(area)?;
    let size = client_size(hwnd);
    let presenter = Presenter::new(&device, hwnd, size.0, size.1)?;
    let waitable = presenter.waitable;
    let shared = Arc::new(Shared {
        latest: Mutex::new(None),
        commands: Mutex::new(VecDeque::new()),
        error: Mutex::new(None),
        stop: AtomicBool::new(false),
        posted: AtomicBool::new(false),
        window: hwnd.0 as isize,
    });

    let scale = if launch.retina { 2 } else { 1 };
    let coded = (launch.width * scale, launch.height * scale);
    let (started, decoder) = mpsc::channel();
    let (go, start) = mpsc::channel();
    {
        let (shared, writer, device) = (shared.clone(), writer.clone(), device.clone());
        std::thread::spawn(move || receive(&shared, &writer, &device, coded, &started, &start));
    }
    let name = decoder.recv().context("The decoder stopped")??;
    eprintln!("Display decoder: {name}");

    VIEWER.with(|viewer| {
        *viewer.borrow_mut() = Some(Viewer {
            hwnd,
            area,
            presenter,
            shared: shared.clone(),
            writer: writer.clone(),
            canvas: (launch.width, launch.height),
            current: None,
            redraw: true,
            presented: Presented::default(),
            render_ready: false,
            model: Model::default(),
            clicks: Clicks::default(),
            typing: Typing::default(),
            captured: false,
            releasing_capture: false,
            text_input: false,
            closed: false,
            effects: Effects::default(),
        })
    });
    unsafe {
        // Keys go out as physical keys until the source takes text.
        let _ = ImmAssociateContextEx(hwnd, HIMC::default(), 0);
        let _ = ShowWindow(hwnd, SW_SHOW);
        // Windows may refuse while another app is in front; the taskbar
        // button then flashes and a click brings the viewer forward.
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        // Locking or switching away needn't reach this window as a focus
        // change, so it is watched separately to release held keys.
        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
    }
    let startup = Startup {
        ready: true,
        width: None,
        height: None,
        retina: None,
        target: None,
    };
    writer.packet(Kind::Startup, startup.to_bytes()?);
    let _ = go.send(());
    watch_parent(shared.clone());

    message_loop(&shared, waitable);
    let closed = with(|viewer| viewer.closed).unwrap_or(false);
    if closed {
        writer.control(&HelperMessage::Closed);
    }
    writer.flush(Duration::from_millis(500));
    let error = shared.error.lock().unwrap().take();
    error.map_or(Ok(()), Err)
}

/// Reads framed stdin, decodes frames and queues commands for the window.
fn receive(
    shared: &Shared,
    writer: &Writer,
    device: &ID3D11Device,
    coded: (u32, u32),
    started: &mpsc::Sender<Result<String>>,
    start: &mpsc::Receiver<()>,
) {
    let mut decoder = match Decoder::new(device, coded) {
        Ok(decoder) => {
            let _ = started.send(Ok(format!("{} ({:?})", decoder.name, decoder.mode)));
            decoder
        }
        Err(error) => {
            let _ = started.send(Err(error));
            return;
        }
    };
    if start.recv().is_err() {
        return;
    }
    let result = (|| -> Result<()> {
        let mut failures = 0;
        // Joining mid-stream, the first keyframe could be seconds away.
        let mut asked = false;
        let mut input = std::io::stdin().lock();
        while let Some((kind, payload)) = read_packet(&mut input)? {
            match protocol::inbound(kind, &payload)? {
                Inbound::Frame(frame) if decoder.waiting() && !frame.keyframe => {
                    if !std::mem::replace(&mut asked, true) {
                        writer.control(&HelperMessage::Keyframe);
                    }
                }
                Inbound::Frame(frame) => match decoder.decode(&frame) {
                    Ok(pictures) => {
                        failures = 0;
                        asked = false;
                        if let Some(picture) = pictures.into_iter().last() {
                            *shared.latest.lock().unwrap() = Some(picture);
                            shared.wake();
                        }
                    }
                    Err(error) => {
                        eprintln!("Display decoding: {error:#}");
                        decoder.reset();
                        writer.control(&HelperMessage::Keyframe);
                        asked = true;
                        failures += 1;
                        if failures >= 3 && decoder.mode == Mode::Hardware {
                            eprintln!("Display decoding: switching to software");
                            decoder = Decoder::with_mode(device, Mode::Software, coded)?;
                            failures = 0;
                        }
                    }
                },
                Inbound::BadFrame => {
                    writer.control(&HelperMessage::Keyframe);
                    asked = true;
                }
                Inbound::Ignored => {}
                Inbound::Command(ViewerCommand::Stop) => return Ok(()),
                Inbound::Command(command) => {
                    shared.commands.lock().unwrap().push_back(command);
                    shared.wake();
                }
            }
        }
        // Stdin closes when the app exits or drops the helper.
        Ok(())
    })();
    let failed = result.is_err();
    shared.finish(result.err());
    // The window normally ends the process first; don't outlive a stuck one.
    std::thread::sleep(EXIT_GRACE);
    std::process::exit(i32::from(failed));
}

/// Pipes alone don't end the viewer when another process inherited them;
/// it also ends with the app that started it.
fn watch_parent(shared: Arc<Shared>) {
    let Some(parent) = parent_process() else {
        return;
    };
    let parent = parent.0 as isize;
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(HANDLE(parent as _), INFINITE) };
        shared.finish(None);
        std::thread::sleep(EXIT_GRACE);
        std::process::exit(0);
    });
}

fn parent_process() -> Option<HANDLE> {
    let created = |process: HANDLE| -> Option<u64> {
        let mut times = [FILETIME::default(); 4];
        let [creation, exit, kernel, user] = &mut times;
        unsafe { GetProcessTimes(process, creation, exit, kernel, user) }.ok()?;
        Some(u64::from(times[0].dwHighDateTime) << 32 | u64::from(times[0].dwLowDateTime))
    };
    let parent = unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let me = GetCurrentProcessId();
        let mut parent = None;
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            if entry.th32ProcessID == me {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
        parent?
    };
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            parent,
        )
    }
    .ok()?;
    // A reused process ID names a process that started after this one.
    if created(handle)? > created(unsafe { GetCurrentProcess() })? {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(handle)
}

fn primary_monitor() -> Result<(HMONITOR, RECT)> {
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    ensure!(
        unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool(),
        "No display available."
    );
    Ok((monitor, info.rcMonitor))
}

fn create_window(area: RECT) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: w!("ExtendComputerViewer"),
            ..Default::default()
        };
        ensure!(
            RegisterClassExW(&class) != 0,
            "Could not register the viewer window"
        );
        // Borderless and covering the monitor: Windows treats it as full
        // screen and hides the taskbar while it's in front.
        CreateWindowExW(
            WS_EX_APPWINDOW,
            w!("ExtendComputerViewer"),
            w!("extend.computer"),
            WS_POPUP,
            area.left,
            area.top,
            area.right - area.left,
            area.bottom - area.top,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .context("Could not open the viewer window")
    }
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut rect = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut rect) };
    (
        (rect.right - rect.left).max(1) as u32,
        (rect.bottom - rect.top).max(1) as u32,
    )
}

/// Runs the window until it closes or the reader stops. Draws whenever a
/// new picture or a redraw is pending and the swap chain can take a frame.
fn message_loop(shared: &Shared, waitable: HANDLE) {
    let mut message = MSG::default();
    let handles = [waitable];
    loop {
        let render = with(|viewer| viewer.needs_render()).unwrap_or(false);
        let wait = unsafe {
            MsgWaitForMultipleObjectsEx(
                render.then_some(&handles[..]),
                INFINITE,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if render && wait == WAIT_OBJECT_0 {
            with(Viewer::render);
        }
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                return;
            }
            if message.message == WM_VIEWER {
                // Later pictures and commands post a new wake.
                shared.posted.store(false, Ordering::SeqCst);
            }
            unsafe {
                // Queues WM_CHAR after a key press, which key handling peeks at.
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
    }
}

/// Window calls that send messages back to the window run after its state
/// is released.
#[derive(Default)]
struct Effects {
    focus: bool,
    capture: Option<bool>,
    text_input: Option<bool>,
    candidates: Option<POINT>,
    refit: bool,
    close: bool,
}

struct Viewer {
    hwnd: HWND,
    /// The monitor the window covers.
    area: RECT,
    presenter: Presenter,
    shared: Arc<Shared>,
    writer: Arc<Writer>,
    /// The encoded canvas size before Retina, which input points refer to.
    canvas: (u32, u32),
    current: Option<Picture>,
    redraw: bool,
    presented: Presented,
    render_ready: bool,
    model: Model,
    clicks: Clicks,
    typing: Typing,
    /// The window holds the mouse capture for a press.
    captured: bool,
    /// The next capture change is the window's own release.
    releasing_capture: bool,
    text_input: bool,
    /// The user closed the window, which ends the session.
    closed: bool,
    effects: Effects,
}

thread_local! {
    static VIEWER: RefCell<Option<Viewer>> = const { RefCell::new(None) };
}

/// Runs `f` on the window state, unless there is none yet or a window call
/// re-entered the window procedure.
fn with<T>(f: impl FnOnce(&mut Viewer) -> T) -> Option<T> {
    VIEWER.with(|viewer| {
        let mut viewer = viewer.try_borrow_mut().ok()?;
        viewer.as_mut().map(f)
    })
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Peeking can deliver messages sent from other threads, so it happens
    // before the state is borrowed.
    let produces = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN).then(|| peek_produces(hwnd));
    let characters = (message == WM_CHAR).then(|| queued_characters(hwnd, wparam));
    let handled = with(|viewer| viewer.handle(message, wparam, lparam, produces, characters));
    if let Some(effects) = with(|viewer| std::mem::take(&mut viewer.effects)) {
        apply(hwnd, effects);
    }
    match handled {
        Some(Some(result)) => result,
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// What the key press being handled types: TranslateMessage has queued its
/// WM_CHAR or WM_DEADCHAR, if any.
fn peek_produces(hwnd: HWND) -> Produces {
    let mut queued = MSG::default();
    let found = unsafe {
        PeekMessageW(
            &mut queued,
            Some(hwnd),
            WM_CHAR,
            WM_DEADCHAR,
            PM_NOREMOVE | PM_NOYIELD,
        )
    };
    if found.as_bool() {
        Produces::of(queued.wParam.0 as u16, queued.message == WM_DEADCHAR)
    } else {
        Produces::Nothing
    }
}

/// This WM_CHAR's unit and those queued behind it, such as the rest of an
/// input method's commit, so they leave as one text event.
fn queued_characters(hwnd: HWND, first: WPARAM) -> Vec<u16> {
    let mut units = vec![first.0 as u16];
    let mut queued = MSG::default();
    while units.len() < 4096
        && unsafe { PeekMessageW(&mut queued, Some(hwnd), WM_CHAR, WM_CHAR, PM_REMOVE) }.as_bool()
    {
        units.push(queued.wParam.0 as u16);
    }
    units
}

fn apply(hwnd: HWND, effects: Effects) {
    unsafe {
        if effects.focus {
            let _ = SetFocus(Some(hwnd));
        }
        match effects.capture {
            Some(true) => {
                SetCapture(hwnd);
            }
            Some(false) => {
                let _ = ReleaseCapture();
            }
            None => {}
        }
        if let Some(enabled) = effects.text_input {
            // Like the macOS viewer, an input method composes only when the
            // source takes text; otherwise keys go out as physical keys.
            let _ = ImmAssociateContextEx(
                hwnd,
                HIMC::default(),
                if enabled { IACE_DEFAULT } else { 0 },
            );
        }
        if let Some(position) = effects.candidates {
            let context = ImmGetContext(hwnd);
            if !context.is_invalid() {
                let form = CANDIDATEFORM {
                    dwIndex: 0,
                    dwStyle: CFS_CANDIDATEPOS,
                    ptCurrentPos: position,
                    rcArea: RECT::default(),
                };
                let _ = ImmSetCandidateWindow(context, &form);
                let _ = ImmReleaseContext(hwnd, context);
            }
        }
        if effects.refit {
            if let Ok((_, area)) = primary_monitor() {
                with(|viewer| viewer.area = area);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    area.left,
                    area.top,
                    area.right - area.left,
                    area.bottom - area.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        if effects.close {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn signed_words(value: isize) -> (i32, i32) {
    (
        i32::from(value as u16 as i16),
        i32::from((value >> 16) as u16 as i16),
    )
}

fn modifiers_now() -> u8 {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    input::modifier_mask(
        down(VK_SHIFT),
        down(VK_CONTROL),
        down(VK_MENU),
        down(VK_LWIN) || down(VK_RWIN),
    )
}

/// The set-1 scancode of a key message, with 0xE000 for extended keys.
fn scancode(lparam: LPARAM) -> u16 {
    let scan = ((lparam.0 >> 16) & 0xFF) as u16;
    if lparam.0 & (1 << 24) != 0 {
        scan | 0xE000
    } else {
        scan
    }
}

impl Viewer {
    fn handle(
        &mut self,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        produces: Option<Produces>,
        characters: Option<Vec<u16>>,
    ) -> Option<LRESULT> {
        let done = Some(LRESULT(0));
        match message {
            WM_VIEWER => {
                self.commands();
                done
            }
            WM_PAINT => {
                unsafe {
                    let _ = ValidateRect(Some(self.hwnd), None);
                }
                self.redraw = true;
                done
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_SIZE => {
                // A new size ends a gesture; the runtime maps input anew.
                self.stop_input();
                let (width, height) = signed_words(lparam.0);
                if width > 0 && height > 0 {
                    if let Err(error) = self.presenter.resize(width as u32, height as u32) {
                        self.fail(error);
                    }
                }
                self.redraw = true;
                done
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                self.stop_input();
                self.effects.refit = true;
                done
            }
            WM_ACTIVATE => {
                // Windows takes Alt+Esc for itself before the window sees
                // Esc, so the way out is a hotkey while the viewer is active.
                if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    self.stop_input();
                    let _ = unsafe { UnregisterHotKey(Some(self.hwnd), EXIT_HOTKEY) };
                } else {
                    let _ = unsafe {
                        RegisterHotKey(
                            Some(self.hwnd),
                            EXIT_HOTKEY,
                            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                            u32::from(VK_ESCAPE.0),
                        )
                    };
                }
                None
            }
            WM_HOTKEY if wparam.0 == EXIT_HOTKEY as usize => {
                self.close();
                done
            }
            WM_ACTIVATEAPP => {
                if wparam.0 == 0 {
                    self.stop_input();
                }
                None
            }
            WM_KILLFOCUS | WM_WTSSESSION_CHANGE => {
                self.stop_input();
                None
            }
            WM_CAPTURECHANGED => {
                if !std::mem::take(&mut self.releasing_capture) && self.captured {
                    self.captured = false;
                    self.stop_input();
                }
                done
            }
            WM_CLOSE => {
                self.close();
                done
            }
            WM_DESTROY => {
                unsafe {
                    let _ = WTSUnRegisterSessionNotification(self.hwnd);
                    PostQuitMessage(0);
                }
                done
            }
            // No menu to open, and the screen stays on while viewing.
            WM_SYSCOMMAND
                if [SC_KEYMENU, SC_SCREENSAVE, SC_MONITORPOWER]
                    .contains(&((wparam.0 & 0xFFF0) as u32)) =>
            {
                done
            }
            WM_MOUSEMOVE => {
                self.mouse_move(lparam);
                done
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP => {
                self.button(0, message == WM_LBUTTONDOWN, lparam);
                done
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => {
                self.button(1, message == WM_RBUTTONDOWN, lparam);
                done
            }
            WM_MBUTTONDOWN | WM_MBUTTONUP => {
                self.button(2, message == WM_MBUTTONDOWN, lparam);
                done
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                self.wheel(wparam, lparam, message == WM_MOUSEHWHEEL);
                done
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => self.key_down(wparam, lparam, produces?),
            WM_KEYUP | WM_SYSKEYUP => self.key_up(lparam),
            WM_CHAR => {
                self.characters(characters?);
                done
            }
            // Alt chords and dead keys never reach the default handler,
            // which would look for a menu and beep.
            WM_SYSCHAR | WM_SYSDEADCHAR | WM_DEADCHAR => done,
            WM_IME_STARTCOMPOSITION => {
                self.effects.candidates = self.candidates();
                None
            }
            _ => None,
        }
    }

    fn needs_render(&self) -> bool {
        self.redraw || self.shared.latest.lock().unwrap().is_some()
    }

    fn render(&mut self) {
        let fresh = self.shared.latest.lock().unwrap().take();
        let is_fresh = fresh.is_some();
        if fresh.is_some() {
            self.current = fresh;
        }
        if let Err(error) = self.presenter.draw(self.current.as_ref()) {
            self.fail(error);
            return;
        }
        self.redraw = false;
        if !is_fresh {
            return;
        }
        if !self.render_ready && self.presenter.finished(Duration::from_millis(500)) {
            self.render_ready = true;
            self.writer
                .control(&HelperMessage::RenderReady(RenderReady { ready: true }));
        }
        if let Some(message) = self.presented.shown() {
            self.writer.control(&message);
        }
    }

    fn fail(&mut self, error: anyhow::Error) {
        self.shared.finish(Some(error));
    }

    fn fullscreen(&self) -> bool {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok()
            && rect == self.area
            && self.presenter.size == client_size(self.hwnd)
            && !unsafe { IsIconic(self.hwnd) }.as_bool()
    }

    fn commands(&mut self) {
        let commands: Vec<_> = self.shared.commands.lock().unwrap().drain(..).collect();
        for command in commands {
            match command {
                ViewerCommand::Viewport { id } => {
                    let (width, height) = self.presenter.size;
                    self.writer
                        .control(&protocol::viewport(id, width, height, self.fullscreen()));
                }
                ViewerCommand::Configuration(configuration) => {
                    let ok = protocol::configuration_valid(&configuration);
                    if ok {
                        self.canvas = (configuration.width, configuration.height);
                    }
                    self.writer.control(&HelperMessage::Configured {
                        id: configuration.id,
                        ok,
                    });
                }
                ViewerCommand::InputState(state) => match Capabilities::new(&state) {
                    Ok(capabilities) => {
                        let text = capabilities.text_supported;
                        if self.model.configure(capabilities) {
                            self.typing.reset();
                        }
                        if text != self.text_input {
                            self.text_input = text;
                            self.effects.text_input = Some(text);
                        }
                    }
                    Err(error) => self.fail(error),
                },
                // The reader stops on its own.
                ViewerCommand::Stop => {}
            }
        }
    }

    fn send(&mut self, events: Vec<NativeEvent>) {
        if self.writer.input(events) {
            // The queue already holds the focus loss.
            self.model.blur();
            self.typing.reset();
        }
    }

    /// Ends any gesture and gives up the lease, which releases everything
    /// the source holds for this viewer.
    fn stop_input(&mut self) {
        let events = self.model.blur();
        self.send(events);
        self.typing.reset();
        if self.captured {
            self.captured = false;
            self.releasing_capture = true;
            self.effects.capture = Some(false);
        }
    }

    fn close(&mut self) {
        if !self.closed {
            self.stop_input();
            self.closed = true;
            self.effects.close = true;
        }
    }

    fn owns_focus(&self) -> bool {
        unsafe { GetForegroundWindow() == self.hwnd && GetFocus() == self.hwnd }
    }

    /// The user acted on the viewer; asks for the lease if it isn't held.
    fn intent(&mut self) -> bool {
        if !self.owns_focus() || !self.model.capabilities().available {
            return false;
        }
        let events = self.model.intent();
        self.send(events);
        self.model.accepts_input()
    }

    fn point(&self, x: i32, y: i32) -> Option<Point> {
        let (width, height) = self.presenter.size;
        fit::point(
            (f64::from(x), f64::from(y)),
            Area {
                x: 0.0,
                y: 0.0,
                width: f64::from(width),
                height: f64::from(height),
            },
            (f64::from(self.canvas.0), f64::from(self.canvas.1)),
            &self.model.capabilities().content,
        )
    }

    fn mouse_move(&mut self, lparam: LPARAM) {
        let (x, y) = signed_words(lparam.0);
        if !self.owns_focus() {
            return;
        }
        if let Some(point) = self.point(x, y) {
            let events = self.model.move_to(point);
            self.send(events);
        }
    }

    fn button(&mut self, button: u8, down: bool, lparam: LPARAM) {
        let (x, y) = signed_words(lparam.0);
        let location = self.point(x, y);
        if down {
            if location.is_none() {
                return;
            }
            self.effects.focus = true;
            if !self.intent() {
                return;
            }
            let (limit, slop) = unsafe {
                (
                    Duration::from_millis(u64::from(GetDoubleClickTime())),
                    (
                        GetSystemMetrics(SM_CXDOUBLECLK),
                        GetSystemMetrics(SM_CYDOUBLECLK),
                    ),
                )
            };
            self.clicks
                .press(button, Instant::now(), (x, y), limit, slop);
            if !self.captured {
                // A drag that leaves the window still ends here.
                self.captured = true;
                self.effects.capture = Some(true);
            }
        } else if !self.owns_focus() {
            self.stop_input();
            return;
        }
        let events = self.model.button(
            button,
            down,
            self.clicks.count(button),
            location,
            modifiers_now(),
        );
        self.send(events);
        if !down && self.captured && self.model.held().buttons.is_empty() {
            self.captured = false;
            self.releasing_capture = true;
            self.effects.capture = Some(false);
        }
    }

    fn wheel(&mut self, wparam: WPARAM, lparam: LPARAM, horizontal: bool) {
        // Wheel messages carry screen coordinates.
        let (x, y) = signed_words(lparam.0);
        let mut position = POINT { x, y };
        if !unsafe { ScreenToClient(self.hwnd, &mut position) }.as_bool() {
            return;
        }
        let Some(point) = self.point(position.x, position.y) else {
            return;
        };
        self.effects.focus = true;
        if !self.intent() {
            return;
        }
        let delta = (wparam.0 >> 16) as u16 as i16;
        let (dx, dy) = fit::wheel_pixels(delta, horizontal);
        let events = self.model.scroll(dx, dy, point, modifiers_now());
        self.send(events);
    }

    fn key_down(&mut self, wparam: WPARAM, lparam: LPARAM, produces: Produces) -> Option<LRESULT> {
        let key = VIRTUAL_KEY(wparam.0 as u16);
        let mask = modifiers_now();
        // Local ways out, whether or not the source took input. Win and
        // Alt+Tab stay with Windows too, as Command does on the macOS viewer.
        let alt_only = mask & (CONTROL | OPTION) == OPTION;
        if (key == VK_ESCAPE && mask & (CONTROL | OPTION) == CONTROL | OPTION)
            || (key == VK_F4 && alt_only)
        {
            self.close();
            return Some(LRESULT(0));
        }
        let scan = scancode(lparam);
        if keys::modifier_of(scan).is_some() {
            // A modifier alone doesn't ask for input, so starting Alt+Tab or
            // opening Start leaves the source alone, as on the macOS viewer.
            // The next key or click asks and sends the current mask first.
            if self.owns_focus() && self.model.accepts_input() {
                let events = self.model.modifiers(mask);
                self.send(events);
            }
            return Some(LRESULT(0));
        }
        if !self.intent() {
            return None;
        }
        if key == VK_PROCESSKEY {
            // The input method composes; its commit arrives as WM_CHAR.
            self.typing.pressed(false);
            return Some(LRESULT(0));
        }
        // A key press leaves as the text it types or as a physical key,
        // never both.
        let text = input::sends_text(self.model.capabilities().text_supported, mask, produces);
        self.typing.pressed(!text);
        if !text {
            if let Some(code) = keys::scan_to_mac(scan) {
                let repeat = lparam.0 & (1 << 30) != 0;
                let events = self.model.key(code, true, repeat, mask);
                self.send(events);
            }
        }
        Some(LRESULT(0))
    }

    fn key_up(&mut self, lparam: LPARAM) -> Option<LRESULT> {
        if !self.owns_focus() || !self.model.accepts_input() {
            return None;
        }
        let scan = scancode(lparam);
        let mask = modifiers_now();
        let events = if keys::modifier_of(scan).is_some() {
            self.model.modifiers(mask)
        } else if let Some(code) = keys::scan_to_mac(scan) {
            self.model.key(code, false, false, mask)
        } else {
            Vec::new()
        };
        self.send(events);
        Some(LRESULT(0))
    }

    fn characters(&mut self, units: Vec<u16>) {
        if !self.owns_focus() || !self.model.accepts_input() {
            return;
        }
        let text: String = units
            .into_iter()
            .filter_map(|unit| self.typing.character(unit))
            .collect();
        if !text.is_empty() {
            let events = self.model.text(&text);
            self.send(events);
        }
    }

    /// Where an input method shows its candidates: at the last pointer
    /// position on the picture, or its center.
    fn candidates(&self) -> Option<POINT> {
        let (width, height) = self.presenter.size;
        let picture = fit::aspect_fit(
            Area {
                x: 0.0,
                y: 0.0,
                width: f64::from(width),
                height: f64::from(height),
            },
            (f64::from(self.canvas.0), f64::from(self.canvas.1)),
        )?;
        let point = self.model.last_point().unwrap_or(Point { x: 0.5, y: 0.5 });
        Some(POINT {
            x: (picture.x + point.x * picture.width) as i32,
            y: (picture.y + point.y * picture.height) as i32,
        })
    }
}
