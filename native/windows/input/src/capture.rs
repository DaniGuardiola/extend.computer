//! Sends this computer's pointer, keyboard and buttons to the peer when the
//! pointer crosses the shared edge. Low-level hooks see
//! every event first; while the peer has control they swallow them and
//! report wire events on stdout instead.
use crate::{
    edge::{Layout, Motion, Point, Side},
    inject::{primary, SYNTHETIC},
    keys,
};
use anyhow::{bail, Context, Result};
use extend_computer_agent::input::InputEvent;
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL},
    System::Threading::GetCurrentThreadId,
    UI::{
        HiDpi::GetDpiForSystem, Input::KeyboardAndMouse::GetDoubleClickTime, WindowsAndMessaging::*,
    },
};

const HEARTBEAT: Duration = Duration::from_secs(2);
const ESCAPE: u16 = 0x01;

struct State {
    layout: Layout,
    display: RECT,
    scale: f64,
    /// Where the hidden local pointer waits while the peer has control.
    park: POINT,
    /// Held locally; crossing waits until everything is released.
    local_keys: BTreeSet<u16>,
    local_buttons: u8,
    /// Held on the peer's side.
    remote_keys: BTreeSet<u16>,
    remote_modifier_keys: BTreeSet<u16>,
    remote_buttons: BTreeSet<u8>,
    modifiers: u8,
    /// Key-ups for keys that were pressed while remote.
    swallow_up: BTreeSet<u16>,
    clicks: Option<(u8, Instant, POINT, u8)>,
    stop: Option<mpsc::Sender<String>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn emit_event(event: InputEvent) {
    emit(&serde_json::to_string(&event).unwrap_or_default());
}

impl State {
    fn stop(&self, reason: &str) {
        if let Some(stop) = &self.stop {
            let _ = stop.send(reason.into());
        }
    }

    fn exposed(&self, y: i32) -> bool {
        let x = match self.layout.side {
            Side::Left => self.display.left - 1,
            Side::Right => self.display.right,
        };
        unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) }.is_invalid()
    }

    fn can_enter(&self) -> bool {
        self.local_keys.is_empty() && self.local_buttons == 0
    }

    fn to_pixels(&self, point: Point) -> POINT {
        POINT {
            x: self.display.left + (point.x * self.scale).round() as i32,
            y: self.display.top + (point.y * self.scale).round() as i32,
        }
    }

    /// Hands control back: releases whatever the peer holds and shows the
    /// local pointer at `point`.
    fn return_local(&mut self, point: Point) {
        emit_event(InputEvent::Release);
        self.swallow_up.extend(self.remote_keys.iter().copied());
        self.swallow_up
            .extend(self.remote_modifier_keys.iter().copied());
        self.remote_keys.clear();
        self.remote_modifier_keys.clear();
        self.remote_buttons.clear();
        self.modifiers = 0;
        let target = self.to_pixels(point);
        unsafe {
            let _ = SetCursorPos(target.x, target.y);
        }
        emit("LOCAL");
    }

    fn mouse_move(&mut self, pt: POINT) -> bool {
        let remote = self.layout.is_remote();
        let d = self.display;
        if !remote
            && !(pt.x >= d.left - 64 && pt.x < d.right + 64 && pt.y >= d.top && pt.y < d.bottom)
        {
            return false;
        }
        let from = if remote {
            self.park
        } else {
            let mut current = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut current);
            }
            current
        };
        let local = Point {
            x: f64::from(pt.x.clamp(d.left, d.right - 1) - d.left) / self.scale,
            y: f64::from(pt.y.clamp(d.top, d.bottom - 1) - d.top) / self.scale,
        };
        let dx = f64::from(pt.x - from.x) / self.scale;
        let dy = f64::from(pt.y - from.y) / self.scale;
        let at_edge = match self.layout.side {
            Side::Left => local.x <= 1.0,
            Side::Right => local.x >= self.layout.local_width - 2.0,
        };
        let exposed = remote || (at_edge && self.can_enter() && self.exposed(pt.y));
        let allow_return = self.remote_buttons.is_empty();
        match self.layout.motion(local, dx, dy, exposed, allow_return) {
            Motion::Local => false,
            Motion::Remote(point) => {
                if !remote {
                    // Park at the crossing point; motion is measured from here.
                    self.park = POINT {
                        x: pt.x.clamp(d.left, d.right - 1),
                        y: pt.y.clamp(d.top, d.bottom - 1),
                    };
                    unsafe {
                        let _ = SetCursorPos(self.park.x, self.park.y);
                    }
                    emit("REMOTE");
                }
                emit(&format!("{} {}", point.x, point.y));
                true
            }
            Motion::Returned(point) => {
                self.return_local(point);
                true
            }
        }
    }

    fn button(&mut self, button: u8, down: bool, pt: POINT) -> bool {
        if !self.layout.is_remote() {
            let bit = 1 << button;
            if down {
                self.local_buttons |= bit;
            } else {
                self.local_buttons &= !bit;
            }
            return false;
        }
        if down {
            let limit = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
            let (cx, cy) = unsafe {
                (
                    GetSystemMetrics(SM_CXDOUBLECLK),
                    GetSystemMetrics(SM_CYDOUBLECLK),
                )
            };
            let count = match self.clicks {
                Some((last, at, p, count))
                    if last == button
                        && at.elapsed() <= limit
                        && (p.x - pt.x).abs() <= cx / 2
                        && (p.y - pt.y).abs() <= cy / 2 =>
                {
                    count % 3 + 1
                }
                _ => 1,
            };
            self.clicks = Some((button, Instant::now(), pt, count));
            self.remote_buttons.insert(button);
        } else if !self.remote_buttons.remove(&button) {
            return true;
        }
        let clicks = self.clicks.map_or(1, |c| c.3);
        emit_event(InputEvent::Button {
            button,
            down,
            clicks,
        });
        true
    }

    fn wheel(&mut self, delta: i16, horizontal: bool) -> bool {
        if !self.layout.is_remote() {
            return false;
        }
        // One 120-unit notch is 10 macOS pixels on the wire.
        let pixels = (i32::from(delta) / 12).clamp(-4096, 4096);
        let (dx, dy) = if horizontal {
            (-pixels, 0)
        } else {
            (0, pixels)
        };
        emit_event(InputEvent::Scroll { dx, dy });
        true
    }

    fn key(&mut self, scan: u16, down: bool) -> bool {
        let control = self.local_keys.contains(&0x1D)
            || self.local_keys.contains(&0xE01D)
            || self.remote_modifier_keys.contains(&0x1D)
            || self.remote_modifier_keys.contains(&0xE01D);
        let alt = self.local_keys.contains(&0x38)
            || self.local_keys.contains(&0xE038)
            || self.remote_modifier_keys.contains(&0x38)
            || self.remote_modifier_keys.contains(&0xE038);
        if scan == ESCAPE && down && control && alt {
            self.stop("user");
            return true;
        }
        if !self.layout.is_remote() {
            if !down && self.swallow_up.remove(&scan) {
                return true;
            }
            if down {
                self.local_keys.insert(scan);
            } else {
                self.local_keys.remove(&scan);
            }
            return false;
        }
        if keys::modifier_of(scan).is_some() {
            if down {
                self.remote_modifier_keys.insert(scan);
            } else {
                self.remote_modifier_keys.remove(&scan);
            }
            let mask = self.mask();
            if mask != self.modifiers {
                self.modifiers = mask;
                emit_event(InputEvent::Modifiers { mask });
            }
            return true;
        }
        let Some(code) = keys::scan_to_mac(scan) else {
            return true;
        };
        let held = self.remote_keys.contains(&scan);
        if down {
            self.remote_keys.insert(scan);
            emit_event(InputEvent::MacKey {
                code,
                down: true,
                repeat: held,
            });
        } else if held {
            self.remote_keys.remove(&scan);
            emit_event(InputEvent::MacKey {
                code,
                down: false,
                repeat: false,
            });
        }
        true
    }

    /// The wire mask: each held modifier, plus its right bit when only the
    /// right-hand key is down.
    fn mask(&self) -> u8 {
        let mut mask = 0;
        for (index, &(bit, left, right)) in keys::MODIFIERS.iter().enumerate() {
            let l = self.remote_modifier_keys.contains(&left);
            let r = self.remote_modifier_keys.contains(&right);
            if l || r {
                mask |= bit;
            }
            if r && !l {
                mask |= keys::RIGHT << index;
            }
        }
        mask
    }
}

fn with_state<T>(default: T, f: impl FnOnce(&mut State) -> T) -> T {
    match STATE.lock() {
        Ok(mut guard) => guard.as_mut().map_or(default, f),
        Err(_) => default,
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.dwExtraInfo != SYNTHETIC {
            let wheel = (info.mouseData >> 16) as i16;
            let swallow = with_state(false, |state| match wparam.0 as u32 {
                WM_MOUSEMOVE => state.mouse_move(info.pt),
                WM_LBUTTONDOWN => state.button(0, true, info.pt),
                WM_LBUTTONUP => state.button(0, false, info.pt),
                WM_RBUTTONDOWN => state.button(1, true, info.pt),
                WM_RBUTTONUP => state.button(1, false, info.pt),
                WM_MBUTTONDOWN => state.button(2, true, info.pt),
                WM_MBUTTONUP => state.button(2, false, info.pt),
                WM_MOUSEWHEEL => state.wheel(wheel, false),
                WM_MOUSEHWHEEL => state.wheel(wheel, true),
                // Side buttons have no wire equivalent; keep them off the local screen while remote.
                _ => state.layout.is_remote(),
            });
            if swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.dwExtraInfo != SYNTHETIC {
            let mut scan = (info.scanCode & 0xFF) as u16;
            if info.flags.0 & LLKHF_EXTENDED.0 != 0 {
                scan |= 0xE000;
            }
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if with_state(false, |state| state.key(scan, down)) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn run(side: Side, args: &[String]) -> Result<()> {
    let number = |index: usize| -> Result<f64> {
        args.get(index)
            .and_then(|v| v.parse::<f64>().ok())
            .context("Usage: capture-control-<side> <remoteWidth> <remoteHeight> <offsetY>")
    };
    let (remote_width, remote_height, offset_y) = (number(0)?, number(1)?, number(2)?);
    if args.len() != 3 {
        bail!("Usage: capture-control-<side> <remoteWidth> <remoteHeight> <offsetY>");
    }
    let (display, width, height) = primary();
    let layout = Layout::new(width, height, remote_width, remote_height, side, offset_y)?;
    let (stop, stopped) = mpsc::channel();
    *STATE.lock().unwrap() = Some(State {
        layout,
        display,
        scale: f64::from(unsafe { GetDpiForSystem() }.max(96)) / 96.0,
        park: POINT::default(),
        local_keys: BTreeSet::new(),
        local_buttons: 0,
        remote_keys: BTreeSet::new(),
        remote_modifier_keys: BTreeSet::new(),
        remote_buttons: BTreeSet::new(),
        modifiers: 0,
        swallow_up: BTreeSet::new(),
        clicks: None,
        stop: Some(stop.clone()),
    });

    // Hooks run on the thread that installs them, which must pump messages.
    let (hooked, hook_result) = mpsc::channel();
    let hook_thread = std::thread::spawn(move || unsafe {
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0);
        let ok = mouse.is_ok() && keyboard.is_ok();
        let _ = hooked.send((ok, GetCurrentThreadId()));
        if ok {
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {}
        }
        if let Ok(hook) = mouse {
            let _ = UnhookWindowsHookEx(hook);
        }
        if let Ok(hook) = keyboard {
            let _ = UnhookWindowsHookEx(hook);
        }
    });
    let (ok, hook_thread_id) = hook_result.recv()?;
    if !ok {
        bail!("Could not watch keyboard and mouse input");
    }
    emit(&format!("READY {width:.1} {height:.1}"));

    let (lines, input) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let mut alive = Instant::now();
    let reason = loop {
        if let Ok(reason) = stopped.try_recv() {
            break reason;
        }
        match input.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => match line.trim() {
                "ALIVE" => alive = Instant::now(),
                command => match command.strip_prefix("EDGE ").and_then(Side::parse) {
                    Some(side) => with_state((), |state| state.layout.set_side(side)),
                    None => break "invalid-command".to_string(),
                },
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break "input-closed".into(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if alive.elapsed() > HEARTBEAT {
            break "heartbeat-timeout".into();
        }
        if primary().0 != display {
            break "display-changed".into();
        }
    };
    {
        let mut guard = STATE.lock().unwrap();
        if let Some(state) = guard.as_mut() {
            if let Some(point) = state.layout.cancel() {
                state.return_local(point);
            }
        }
        *guard = None;
    }
    unsafe {
        let _ = PostThreadMessageW(hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
    }
    let _ = hook_thread.join();
    emit(&format!("STOP {reason}"));
    Ok(())
}
