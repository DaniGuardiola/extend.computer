# extend.computer

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
