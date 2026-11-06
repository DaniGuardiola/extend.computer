//! Lists cable-attached iPhones/iPads as the desktop USB inventory sees them.
//! `cargo run -p extend-computer-agent --example usbmux_devices [--probe] [--watch]`
//! `--probe` tries the app's USB port on each device (refused = app not listening).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    use extend_computer_agent::display::usbmux::{MuxEvent, Usbmux, IOS_USB_PORT};
    let mux = Usbmux::default();
    for device in mux.devices()? {
        println!(
            "{} id={} udid={} product=0x{:04x} via={}",
            device.name, device.id, device.udid, device.product_id, device.connection
        );
        if device.usb() && std::env::args().any(|arg| arg == "--probe") {
            match mux.connect(device.id, IOS_USB_PORT) {
                Ok(_) => println!("  port {IOS_USB_PORT}: app listening"),
                Err(error) => println!("  port {IOS_USB_PORT}: {error}"),
            }
        }
    }
    if std::env::args().any(|arg| arg == "--watch") {
        let mut listener = mux.listen()?;
        loop {
            match listener.next_event()? {
                MuxEvent::Attached(device) => println!("attached {device:?}"),
                MuxEvent::Detached(id) => println!("detached id={id}"),
            }
        }
    }
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("usbmuxd is available on macOS and Linux");
}
