# iOS parity tracking

The target is the Android app's behavior (`PARITY.md`) on iPhone and iPad,
minus what iOS does not allow third-party apps to do. Each OS limit below names
the missing public API; none is papered over with a fake control.

Hardware evidence comes from an iPhone SE (3rd generation, iOS 26.0.1) cabled
to an M3 Max MacBook Pro. Detailed checks used the shared engine through
`engine/examples/iphone_mac_peer.rs` and `engine/examples/iphone_screen_receiver.rs`;
a final pass repeated the main flows against the desktop app itself: visual
pairing, extend and mirror over LAN, the desktop USB dialog connecting
through usbmux with extend over USB, the iPhone trackpad moving the Mac cursor
over LAN and USB, and iPhone screen sharing in the desktop viewer over LAN and
USB.

| Capability | iOS state | Remaining verification or work |
| --- | --- | --- |
| Application UI and appearance | Same React Native screens as Android with iOS adaptations: chevron back, safe areas, keyboard insets, platform wording, persistent appearance. UIScene lifecycle adopted (required to launch on iOS 27; checked on the iOS 27.2 simulator). The full-width back swipe is off on the trackpad screen so right swipes stay on the pad. After the full-screen display, the root view controller pads the top until iOS updates the window's safe area, so headers don't slide under the status bar | iPad layouts unchecked |
| Persistent identity and trust | Engine seed in the Keychain (this device only, after first unlock, shared access group for the broadcast extension). Engine root in the App Group container, excluded from backup; existing roots move there once. Identity survived relaunch and reinstall on hardware | Upgrade path from a released build; backup/restore lifecycle |
| Code pairing | Both directions on hardware: Mac-to-iPhone with on-phone Allow, iPhone-to-Mac with address and code | Expiry and cancellation lifecycle cases |
| Nearby discovery and visual pairing | Bonjour advertisement and browsing (`_extendpair`, `_extendprobe`, `_extendpresence`) with the Local Network prompt. Visual pairing in both directions on hardware, eight matching symbols | Denial, expiry and background cases; IPv6 |
| Device information and rename | Name editor with Android's rules; installed version/build; fingerprint. Rename shown on hardware | Name propagation to connected peers |
| Paired-device management | List, connection check (USB, then LAN, then presence browse, then Internet), Online state, address editor (invalid input rejected), forget with unpair notification (peer's trust store updated) on hardware | Account-only peers on hardware |
| Availability and receiving | Receiving while the app is in the foreground; it pauses in the background and resumes on return (checked on hardware, LAN and USB) | **OS limit:** iOS has no foreground-service equivalent, so the iPhone can't stay available with the app closed. A push-to-wake path would need APNs through the account server |
| Receive extended or mirrored desktop | VideoToolbox HEVC/AVC decode shared with the macOS viewer (`native/apple/shared`), Metal presenter with real present timestamps, cursor overlay, decoder capabilities in Android's format. On hardware: mirror over LAN and USB with HEVC and AVC (rendered counts 185–244 per run), extend mode rendered the virtual display and the Mac arrangement was restored afterwards | Live mode/codec changes, rotation-driven size, long runs |
| Display configuration | Receiver size/frame-rate/quality settings use the shared engine calls as on Android | Hardware check of live receiver changes |
| Send iPhone screen | ReplayKit broadcast upload extension (HEVC/AVC via VideoToolbox, bounded queues) started from the system picker, over LAN or USB. Over USB the app hands the USB port to the extension (App Group claim and Darwin notification; each side retries the bind until the other lets go), the desktop's usbmux reconnect attaches to the extension's engine and the sender uses that carrier; the port returns to the app when the broadcast ends. On hardware: first frame on the Mac in about 0.2 s at 60 fps with no drops, live size and 40 Mbps quality changes, both landscape directions upright, extension footprint 11–15 MB (limit 50 MB), clean stop. Over USB with the desktop app: desktop logged `Display media transport: usb`, the viewer rendered, sharing continued in Settings with the iPhone's Wi-Fi off (route `usb`, encoded frames rising, 13–14 MB footprint), returning to the app kept the stream, and Stop ended it normally with the desktop back on the app's carrier | The account relay isn't reachable from the extension. If the extension is killed, the app retakes the USB port on its next foreground. The system broadcast sheet stays open until dismissed (system UI). **OS limit:** whole-screen only, no single-app capture |
| Extend the iPhone with another display | — | **OS limit:** iOS has no public API to create a virtual display |
| Keyboard and mouse from the iPhone | Trackpad view (move, tap, two-finger right click and scroll, hold-drag) and keyboard with local IME composition and committed text sent once; hardware keys mapped to Mac key codes with key repeat. Display-scoped touch (tap, double tap, hold-drag, scroll, right click, cancel) maps to the right source positions. On hardware: all of these over LAN; display touch and typing also over USB; Japanese IME committed once with nothing sent while composing | Hardware Backspace/Escape/forward-delete (XCTest can't synthesize them); a single 32-unit Unicode commit (XCTest splits typed text) |
| Keyboard and mouse into the iPhone | Settings explains it's unavailable on iPhone | **OS limit:** no public API lets an app inject touches or keys into other apps (no AccessibilityService equivalent) |
| Local and Internet routes | LAN; USB through usbmuxd, where the desktop connects to the app on device port 47177 and sends a 72-byte host hello before the same framed carrier Android accessories use; account relay through the engine. USB display and input worked on hardware, including from the desktop app's USB dialog; after Disconnect the dialog rescans instead of asking for a replug. While an iPhone is selected the desktop reconnects through usbmux whenever the carrier ends (app backgrounded or restarted, port handed to screen sharing), with backoff, until Disconnect USB, unplug, unpairing or 30 minutes; reconnects after an app restart and a reinstall checked on hardware. Android accessories keep their one-shot lifecycle | USB only carries the app's own services while it's in the foreground (screen sharing keeps USB in the background). After an unplug the selection stays stopped until Disconnect USB. Internet display streaming isn't implemented on any platform |
| Account sign-in and devices | Loopback PKCE inside ASWebAuthenticationSession, native password and second-factor sheet, Keychain credentials bound to the server, registration with engine proof, heartbeat and directory, relay configuration, logout keeping manual pairings. 13 simulator tests against the real server. On hardware, against the real server behind a temporary HTTPS tunnel: password sign-in, password plus recovery code, browser sign-in with the system consent alert, the iPhone and another device in the directory, and sign-out keeping the manual pairing. The server and website accept the `ios` platform | Authenticator (TOTP) code on hardware; the production server must be deployed with `ios` support |
| Permissions | Local Network prompt; ReplayKit consent through the system picker; no accessibility or projection permissions exist on iOS | Local Network denial recovery UI |

## Tools

- `mobile/scripts/build-ios.sh` builds the engine xcframework, prebuilds, signs
  with the App Store Connect key, installs and launches on a cabled iPhone.
- `mobile/scripts/ios-driver/drive.sh` runs scripted XCUITest actions (taps,
  multi-touch, typing, hardware keys, orientation) and exports screenshots,
  accessibility trees and crash logs. It posts a macOS notification when the
  phone is waiting for the Touch ID prompt that enables UI automation.
- `engine/examples/usbmux_devices.rs` lists cabled iPhones and probes the app's
  USB port.
