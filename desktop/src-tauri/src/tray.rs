//! The status icon, in the menu bar on macOS and the notification area on
//! Windows. It shows whether this device accepts connections and the current
//! session, offers the window, Allow connections, ending the session and
//! quitting, and posts a notification when another device starts
//! controlling this one or showing a display on it. It follows the same
//! runtime state the window polls, as often.
#[cfg(not(target_os = "macos"))]
use crate::platform::light_taskbar;
use crate::runtime::{Activity, Desktop, Stage};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Wry,
};
use tauri_plugin_notification::NotificationExt;

const OPEN: &str = "open";
const ALLOW: &str = "allow";
const STOP: &str = "stop";
const QUIT: &str = "quit";
const POLL: Duration = Duration::from_millis(500);
#[cfg(target_os = "macos")]
const WHERE: &str = "in the menu bar";
#[cfg(not(target_os = "macos"))]
const WHERE: &str = "in the notification area";

pub fn start(app: &AppHandle, desktop: Arc<Desktop>) -> tauri::Result<()> {
    let shown = Activity::default();
    let refresh = Arc::new(AtomicBool::new(false));
    let actions = (desktop.clone(), refresh.clone());
    let tray = TrayIconBuilder::with_id("status")
        .menu(&menu(app, &shown)?)
        .tooltip(tooltip(&shown))
        .on_menu_event(move |app, event| act(app, &actions.0, &actions.1, event))
        .on_tray_icon_event(|tray, event| {
            // Windows opens the window on a click and the menu on a right click.
            if !cfg!(target_os = "macos")
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                )
            {
                crate::shell::show_window(tray.app_handle());
            }
        })
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .icon_as_template(cfg!(target_os = "macos"));
    let light = light_taskbar();
    let tray = tray.icon(icon(false, light)).build(app)?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("status icon".into())
        .spawn(move || follow(app, tray, desktop, refresh, shown, light))?;
    Ok(())
}

fn follow(
    app: AppHandle,
    tray: TrayIcon,
    desktop: Arc<Desktop>,
    refresh: Arc<AtomicBool>,
    mut shown: Activity,
    mut light: bool,
) {
    while !desktop.is_closing() {
        // Each session that connected since the last look, even one that
        // has already ended.
        for session in desktop.take_arrivals() {
            notify(&app, &session.describe());
        }
        let next = desktop.activity();
        // Pairing requests wait for an answer in the window.
        if next.approval.is_some() && next.approval != shown.approval {
            crate::shell::show_window(&app);
        }
        // Windows can switch the taskbar between light and dark at any time.
        let now_light = light_taskbar();
        if next.incoming() != shown.incoming() || now_light != light {
            light = now_light;
            if let Err(error) = tray.set_icon_with_as_template(
                Some(icon(next.incoming(), light)),
                cfg!(target_os = "macos"),
            ) {
                eprintln!("Could not update the status icon: {error}");
            }
        }
        if next != shown || refresh.swap(false, Ordering::SeqCst) {
            if let Err(error) = render(&app, &tray, &next) {
                eprintln!("Could not update the status icon: {error}");
            }
            shown = next;
        }
        std::thread::sleep(POLL);
    }
}

fn render(app: &AppHandle, tray: &TrayIcon, next: &Activity) -> tauri::Result<()> {
    tray.set_menu(Some(menu(app, next)?))?;
    tray.set_tooltip(Some(tooltip(next)))?;
    Ok(())
}

/// The logo, with a dot while another device controls this one or shows a
/// display on it. macOS tints the template images for the menu bar; Windows
/// shows the logo in its ink on a light taskbar and in white on a dark one,
/// where the ink would all but disappear. Both branches compile everywhere,
/// so a Windows-only mistake fails the macOS build too.
fn icon(incoming: bool, light_taskbar: bool) -> Image<'static> {
    if cfg!(target_os = "macos") {
        if incoming {
            tauri::include_image!("icons/tray-active.png")
        } else {
            tauri::include_image!("icons/tray.png")
        }
    } else {
        windows_icon(incoming, light_taskbar)
    }
}
fn windows_icon(incoming: bool, light_taskbar: bool) -> Image<'static> {
    match (light_taskbar, incoming) {
        (true, false) => tauri::include_image!("icons/tray-windows.png"),
        (true, true) => tauri::include_image!("icons/tray-windows-active.png"),
        (false, false) => tauri::include_image!("icons/tray-windows-white.png"),
        (false, true) => tauri::include_image!("icons/tray-windows-white-active.png"),
    }
}
/// The menu bar tints the template images itself.
#[cfg(target_os = "macos")]
fn light_taskbar() -> bool {
    true
}

fn tooltip(activity: &Activity) -> String {
    let line = activity
        .session
        .as_ref()
        .map_or_else(|| activity.status().to_owned(), |s| s.describe());
    format!("extend.computer\n{line}")
}

/// Menus read `&` as a mnemonic marker; device names show it as typed.
fn label(text: &str) -> String {
    text.replace('&', "&&")
}

fn menu(app: &AppHandle, activity: &Activity) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::new(app, activity.status(), false, None::<&str>)?)?;
    if let Some(session) = &activity.session {
        menu.append(&MenuItem::new(
            app,
            label(&session.describe()),
            false,
            None::<&str>,
        )?)?;
        if session.stage != Stage::Disconnecting {
            menu.append(&MenuItem::with_id(
                app,
                STOP,
                session.stop_label(),
                true,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        ALLOW,
        "Allow connections",
        true,
        activity.receiving,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        OPEN,
        "Open extend.computer",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        QUIT,
        "Quit extend.computer",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn act(app: &AppHandle, desktop: &Arc<Desktop>, refresh: &Arc<AtomicBool>, event: MenuEvent) {
    match event.id().as_ref() {
        OPEN => crate::shell::show_window(app),
        ALLOW => {
            let on = !desktop.activity().receiving;
            let (app, desktop, refresh) = (app.clone(), desktop.clone(), refresh.clone());
            // Starting checks permissions and binds ports, off the main thread.
            std::thread::spawn(move || {
                if desktop.set_receiving(on) {
                    crate::shell::show_window(&app);
                }
                // The menu toggled its check mark itself; show the real state.
                refresh.store(true, Ordering::SeqCst);
            });
        }
        STOP => desktop.end_session(),
        QUIT => app.exit(0),
        _ => {}
    }
}

fn notify(app: &AppHandle, notice: &str) {
    if let Err(error) = app
        .notification()
        .builder()
        .title(notice)
        .body(format!(
            "To stop it, choose Disconnect from the extend.computer icon {WHERE}."
        ))
        .show()
    {
        eprintln!("Could not show a notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::windows_icon;

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(rgb: [u8; 3]) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
    }

    #[test]
    fn the_windows_icon_stands_out_from_the_taskbar() {
        // Windows 11's taskbar in its light and dark themes.
        for (light, taskbar) in [(true, [243, 243, 243]), (false, [32, 32, 32])] {
            for incoming in [false, true] {
                let icon = windows_icon(incoming, light);
                assert_eq!((icon.width(), icon.height()), (32, 32));
                // The logo's ink: its most common opaque colour, which the
                // dot doesn't outnumber.
                let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
                for pixel in icon.rgba().chunks(4).filter(|pixel| pixel[3] == 255) {
                    *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
                }
                let ink = counts.into_iter().max_by_key(|(_, n)| *n).unwrap().0;
                let (a, b) = (luminance(ink), luminance(taskbar));
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                // WCAG's minimum for graphics that carry meaning.
                assert!(
                    contrast >= 3.0,
                    "light taskbar {light}, incoming {incoming}: contrast {contrast:.1}"
                );
            }
        }
    }
}
