//! The status icon, in the menu bar on macOS and the notification area on
//! Windows. It shows whether this device accepts connections and the current
//! session, and offers the window, Allow connections, ending the session and
//! quitting. It follows the same runtime state the window polls, as often.
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

const OPEN: &str = "open";
const ALLOW: &str = "allow";
const STOP: &str = "stop";
const QUIT: &str = "quit";
const POLL: Duration = Duration::from_millis(500);

pub fn start(app: &AppHandle, desktop: Arc<Desktop>) -> tauri::Result<()> {
    let shown = Activity::default();
    let refresh = Arc::new(AtomicBool::new(false));
    let actions = (desktop.clone(), refresh.clone());
    let mut tray = TrayIconBuilder::with_id("status")
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
    if let Some(icon) = icon(app) {
        tray = tray.icon(icon);
    }
    let tray = tray.build(app)?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("status icon".into())
        .spawn(move || follow(app, tray, desktop, refresh, shown))?;
    Ok(())
}

fn follow(
    app: AppHandle,
    tray: TrayIcon,
    desktop: Arc<Desktop>,
    refresh: Arc<AtomicBool>,
    mut shown: Activity,
) {
    while !desktop.is_closing() {
        let next = desktop.activity();
        // Pairing requests wait for an answer in the window.
        if next.approval.is_some() && next.approval != shown.approval {
            crate::shell::show_window(&app);
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

/// The logo. macOS tints the template image for the menu bar.
fn icon(#[allow(unused)] app: &AppHandle) -> Option<Image<'static>> {
    #[cfg(target_os = "macos")]
    return Some(tauri::include_image!("icons/tray.png"));
    #[cfg(not(target_os = "macos"))]
    app.default_window_icon().cloned()
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
