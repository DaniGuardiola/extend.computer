//! Posts keyboard and mouse input with SendInput and remembers everything it
//! holds down, so every exit path can release it.
use crate::keys;
use anyhow::{bail, ensure, Result};
use extend_computer_agent::input::InputEvent;
use std::collections::BTreeSet;
use windows::Win32::{
    Foundation::RECT,
    UI::{
        HiDpi::GetDpiForSystem,
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
    },
};

/// Marks events this helper posts so capture can tell them from the user's.
pub const SYNTHETIC: usize = 0x504f_5254;
/// Wire scroll units are macOS pixels: one wheel notch is 10 of them.
const WHEEL_PER_PIXEL: i32 = 120 / 10;

/// The primary display in physical pixels, and its size in logical points.
pub fn primary() -> (RECT, f64, f64) {
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let scale = f64::from(unsafe { GetDpiForSystem() }.max(96)) / 96.0;
    let rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    (rect, f64::from(width) / scale, f64::from(height) / scale)
}

pub struct Injector {
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u8>,
    modifiers: u8,
    held_modifier_keys: Vec<u16>,
    dry_run: bool,
}

fn keyboard(scan: u16, down: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if scan & 0xFF00 == 0xE000 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan & 0xFF,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: SYNTHETIC,
            },
        },
    }
}

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: SYNTHETIC,
            },
        },
    }
}

impl Injector {
    pub fn new(dry_run: bool) -> Self {
        Self {
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            modifiers: 0,
            held_modifier_keys: Vec::new(),
            dry_run,
        }
    }

    fn send(&self, inputs: &[INPUT]) -> Result<()> {
        if self.dry_run || inputs.is_empty() {
            return Ok(());
        }
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        // Windows blocks input into higher-integrity windows (UIPI) without saying so.
        ensure!(sent as usize == inputs.len(), "Windows refused the input");
        Ok(())
    }

    /// Moves to a normalized point on the primary display.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        ensure!(
            (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            "Invalid pointer position"
        );
        // Absolute coordinates span 0..=65535 across the primary display.
        let ax = (x * 65535.0).round() as i32;
        let ay = (y * 65535.0).round() as i32;
        self.send(&[mouse(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)])
    }

    fn button(&mut self, button: u8, down: bool) -> Result<()> {
        let flags = match (button, down) {
            (0, true) => MOUSEEVENTF_LEFTDOWN,
            (0, false) => MOUSEEVENTF_LEFTUP,
            (1, true) => MOUSEEVENTF_RIGHTDOWN,
            (1, false) => MOUSEEVENTF_RIGHTUP,
            (2, true) => MOUSEEVENTF_MIDDLEDOWN,
            (2, false) => MOUSEEVENTF_MIDDLEUP,
            _ => bail!("Invalid mouse button"),
        };
        // Ignore repeats so a held button is released exactly once.
        if down == self.buttons.contains(&button) {
            return Ok(());
        }
        self.send(&[mouse(0, 0, 0, flags)])?;
        if down {
            self.buttons.insert(button);
        } else {
            self.buttons.remove(&button);
        }
        Ok(())
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()> {
        let mut inputs = Vec::new();
        if dy != 0 {
            inputs.push(mouse(0, 0, dy * WHEEL_PER_PIXEL, MOUSEEVENTF_WHEEL));
        }
        if dx != 0 {
            // macOS scrolls content left for positive dx; Windows tilts right.
            inputs.push(mouse(0, 0, -dx * WHEEL_PER_PIXEL, MOUSEEVENTF_HWHEEL));
        }
        self.send(&inputs)
    }

    fn key(&mut self, code: u16, down: bool, repeat: bool) -> Result<()> {
        // Keys with no Windows equivalent are accepted and dropped.
        let Some(scan) = keys::mac_to_scan(code) else {
            return Ok(());
        };
        let held = self.keys.contains(&scan);
        if (repeat && !held) || (!repeat && down == held) {
            return Ok(());
        }
        self.send(&[keyboard(scan, down)])?;
        if down {
            self.keys.insert(scan);
        } else {
            self.keys.remove(&scan);
        }
        Ok(())
    }

    fn modifiers(&mut self, mask: u8) -> Result<()> {
        let wanted = keys::modifier_keys(mask);
        let mut inputs = Vec::new();
        // Release first so switching sides never holds both keys.
        for &scan in &self.held_modifier_keys {
            if !wanted.contains(&scan) {
                inputs.push(keyboard(scan, false));
            }
        }
        for &scan in &wanted {
            if !self.held_modifier_keys.contains(&scan) {
                inputs.push(keyboard(scan, true));
            }
        }
        self.send(&inputs)?;
        self.held_modifier_keys = wanted;
        self.modifiers = mask;
        Ok(())
    }

    fn text(&mut self, value: &str) -> Result<()> {
        ensure!(
            self.keys.is_empty() && self.modifiers == 0,
            "Text cannot be typed while keys are held"
        );
        let mut inputs = Vec::new();
        for unit in value.encode_utf16() {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: unit,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: SYNTHETIC,
                        },
                    },
                });
            }
        }
        self.send(&inputs)
    }

    /// Applies one wire event after the engine's own validation.
    pub fn event(&mut self, event: &InputEvent) -> Result<()> {
        event.validate()?;
        match event {
            InputEvent::Button { button, down, .. } => self.button(*button, *down),
            InputEvent::Scroll { dx, dy } => self.scroll(*dx, *dy),
            InputEvent::MacKey { code, down, repeat } => self.key(*code, *down, *repeat),
            InputEvent::Modifiers { mask } => self.modifiers(*mask),
            InputEvent::Text { value } => self.text(value),
            InputEvent::Release => self.release(),
        }
    }

    /// Releases keys, then buttons, then modifiers.
    pub fn release(&mut self) -> Result<()> {
        let keys: Vec<u16> = self.keys.iter().copied().collect();
        let buttons: Vec<u8> = self.buttons.iter().copied().collect();
        let mut result = Ok(());
        for scan in keys {
            if let Err(error) = self.send(&[keyboard(scan, false)]) {
                result = Err(error);
            }
            self.keys.remove(&scan);
        }
        for button in buttons {
            if let Err(error) = self.button(button, false) {
                self.buttons.remove(&button);
                result = Err(error);
            }
        }
        if let Err(error) = self.modifiers(0) {
            self.held_modifier_keys.clear();
            self.modifiers = 0;
            result = Err(error);
        }
        result
    }

    pub fn summary(&self) -> String {
        format!(
            "keys={} buttons={} modifiers={}",
            self.keys.len(),
            self.buttons.len(),
            self.modifiers
        )
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        let _ = self.release();
    }
}
