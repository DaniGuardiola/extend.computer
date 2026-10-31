# extend.computer

## 0.4.0

### Minor Changes

- [`07d5d71`](https://github.com/DaniGuardiola/extend.computer/commit/07d5d71ca049272acff38d043240957ee56cfe2f) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Show whether a local network connection runs over Wi-Fi or Ethernet. The connection status names the link, and its explanation says which link each device uses.

- [`09cc0b3`](https://github.com/DaniGuardiola/extend.computer/commit/09cc0b313b76c9a9791b6c9e96d397867cee91cd) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Show live connection quality for connected devices. Signal bars in the connection status show good, fair or poor quality, hovering shows the round trip and packet loss, and a connection that stays below good suggests a fix for its route, such as Ethernet or a USB cable instead of Wi-Fi.

- [`be32574`](https://github.com/DaniGuardiola/extend.computer/commit/be32574ef6956ac58b0b7d0ce866b5ce03cb1e4a) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Sign in with a passkey or security key directly in the app on extend.computer accounts, without switching to the browser. This also works for accounts that use a passkey or security key as their second factor. Custom account servers keep passkey sign-in through the browser.

- [`d68e621`](https://github.com/DaniGuardiola/extend.computer/commit/d68e62136a9cb7c50134fbe6898a407fcdd2d88e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - A Mac can now extend or mirror its screen to a Windows PC running extend.computer. Devices now tell each other what they support, so the Mac picks a video format the other device can show, and Extend, Mirror and Share keyboard & mouse explain when a device can't do them instead of failing after connecting.

### Patch Changes

- [`821d2fb`](https://github.com/DaniGuardiola/extend.computer/commit/821d2fb238f0b4cb55f8a3696ee45966c8715d1f) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Switch between a USB cable and the local network within seconds when the cable is plugged in or removed, instead of losing the device until the app restarts. A connection error on a device now clears once that device is reachable again.

- [`4a804ba`](https://github.com/DaniGuardiola/extend.computer/commit/4a804bac273fbb27d31f7622d69fa4ec6ee84199) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Changing the extended display's size no longer makes other monitors look darker or warmer. The extended display now offers its sizes from the start, so changing size switches modes instead of rebuilding it, which had made macOS reset other screens' color profiles until disconnecting.

- [`c33f448`](https://github.com/DaniGuardiola/extend.computer/commit/c33f448156dc15432b4dd95abb938ad7e476b621) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Fix extending a display failing on the first attempt with "placement rejected": the app now waits for macOS to bring the new display online before placing it.

- [`d2a25b5`](https://github.com/DaniGuardiola/extend.computer/commit/d2a25b559bb94497a644d9b503d6e43b898bc964) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Keep keyboard and mouse sharing smooth while the app is in the background, and send right-hand Shift, Control, Option and Command keys as themselves instead of their left-hand counterparts.

## 0.3.1

### Patch Changes

- [`8bec829`](https://github.com/DaniGuardiola/extend.computer/commit/8bec829e22b3cfea056b11ceecc2ed6849830586) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Keep Wi-Fi optimization showing as allowed while a USB cable is connected. Background permission checks over a cable skip the Wi-Fi check, and they no longer reset its status, so the setting stops flipping to "Permission needed" and devices on the local network can keep connecting.

## 0.3.0

### Minor Changes

- [`3b1df17`](https://github.com/DaniGuardiola/extend.computer/commit/3b1df1762d65ed629ab5192d00247eb1507a8b66) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Add experimental macOS display extension and screen mirroring over the local network. Authenticate paired and account devices before exchanging WebRTC setup, encode and decode video with native hardware codecs, present with Metal, and release display resources when the connection ends. Use Retina capture, bounded frame queues, keyframe recovery, and adaptive bitrate to reduce latency and keep text sharp.

  Service already-due WebRTC pacer timeouts in bounded batches so the application loop does not add a sleep to every video packet. This prevents large LAN frames from timing out during reassembly while preserving future pacing deadlines and regular input processing.

- [`3b1df17`](https://github.com/DaniGuardiola/extend.computer/commit/3b1df1762d65ed629ab5192d00247eb1507a8b66) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Connect two Macs over a USB cable. When macOS exposes USB networking for the cable, the app prefers it for paired devices, binds traffic to the cable interface so a VPN cannot redirect it, and falls back to the local network when the cable is unavailable.

### Patch Changes

- [`3b1df17`](https://github.com/DaniGuardiola/extend.computer/commit/3b1df1762d65ed629ab5192d00247eb1507a8b66) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Refresh the device list: show device icons from the reported hardware type, explain how each device is trusted, show connection errors in the device card, and keep long names readable with tooltips. Change stream size, frame rate limit and video quality from an active display connection's Display settings, and remember the choices for the next connection.

- [`3b1df17`](https://github.com/DaniGuardiola/extend.computer/commit/3b1df1762d65ed629ab5192d00247eb1507a8b66) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Find nearby paired and account devices faster by checking local routes in parallel and verifying device identity before updating the connection address. Refresh local routes when connecting and recovering from interruptions, and close stalled or cancelled checks.

- [`9023d58`](https://github.com/DaniGuardiola/extend.computer/commit/9023d58fb7056b6aaa265a3831f36efd8f86bb9e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Paired devices and verified devices in the same account connect without an extra control approval prompt. Allow connections and the required OS permissions still need to be enabled on the receiving device.

- [`0fde805`](https://github.com/DaniGuardiola/extend.computer/commit/0fde805cb4c76d82f754cf4f2141db78f550d39b) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Show "USB only" next to Allow connections when Wi-Fi optimization is off, so it is clear why devices on the local network can't connect. Show a readable message instead of socket details when display video can't be sent.

## 0.2.1

### Patch Changes

- [`c9e43df`](https://github.com/DaniGuardiola/extend.computer/commit/c9e43df398c9e382581829bf4f8dd590bdab9754) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Bring the desktop app to the foreground after browser sign-in. Close the browser tab when permitted, with a branded success page as a fallback.

- [`2ecc290`](https://github.com/DaniGuardiola/extend.computer/commit/2ecc29087c04336eabc4aa3f33d416f8de0b249e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Fix a crash when opening macOS Accessibility settings and restore Wi-Fi optimization setup in signed releases. Keep permission guides beside System Settings with a small overlap, visible arrows, and safer positioning near screen edges.

## 0.2.0

### Minor Changes

- [`b99827c`](https://github.com/DaniGuardiola/extend.computer/commit/b99827c6e1dde9aa78fc7b729fd8557d80fc9681) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Add native macOS updates, update channels, and offline release history. Check device protocol compatibility before starting a connection.

## 0.1.0

### Initial preview

- Pair Macs and share keyboard and mouse control over encrypted connections.
- Manage accounts, device permissions, and local device settings.
