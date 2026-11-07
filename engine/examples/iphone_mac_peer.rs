// Mac-side iPhone test peer; it drives macOS helpers, so other platforms get a stub.
#[cfg(target_os = "macos")]
include!("macos/iphone_mac_peer.rs");

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("iphone_mac_peer only runs on macOS");
}
