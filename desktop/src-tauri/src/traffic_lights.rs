use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use objc2_foundation::{MainThreadMarker, NSProcessInfo};
use tauri::Manager;

// On macOS 26, the live buttons stay at their default frames despite Wry's
// configured inset. Reapply it after the window opens or its title bar relayouts.
// TODO: When Tauri 2.12 is available, retest startup and title/fullscreen
// relayouts on macOS 26, then remove this workaround if the inset holds.
// Related: https://github.com/tauri-apps/tauri/issues/15451
//          https://github.com/tauri-apps/tauri/issues/13044
pub fn apply(app: &tauri::AppHandle) {
    if MainThreadMarker::new().is_none()
        || NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            != 26
    {
        return;
    }
    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
    else {
        return;
    };
    let Some(position) = config.traffic_light_position.as_ref() else {
        return;
    };
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(handle) = window.ns_window() else {
        return;
    };

    unsafe {
        let native: &NSWindow = &*handle.cast();
        let Some(close) = native.standardWindowButton(NSWindowButton::CloseButton) else {
            return;
        };
        let Some(minimize) = native.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
            return;
        };
        let Some(container) = close.superview().and_then(|view| view.superview()) else {
            return;
        };
        let close_frame = NSView::frame(&close);
        let spacing = NSView::frame(&minimize).origin.x - close_frame.origin.x;
        let height = close_frame.size.height + position.y;
        let mut frame = NSView::frame(&container);
        frame.size.height = height;
        frame.origin.y = native.frame().size.height - height;
        container.setFrame(frame);

        let buttons = [
            Some(close),
            Some(minimize),
            native.standardWindowButton(NSWindowButton::ZoomButton),
        ];
        for (index, button) in buttons.into_iter().flatten().enumerate() {
            let mut frame = NSView::frame(&button);
            frame.origin.x = position.x + index as f64 * spacing;
            button.setFrameOrigin(frame.origin);
        }
    }
}
