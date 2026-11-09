# extend.computer

## 0.4.0

### Minor Changes

- [`07d5d71`](https://github.com/DaniGuardiola/extend.computer/commit/07d5d71ca049272acff38d043240957ee56cfe2f) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Show whether a local network connection runs over Wi-Fi or Ethernet. The connection status names the link, and its explanation says which link each device uses.

- [`09cc0b3`](https://github.com/DaniGuardiola/extend.computer/commit/09cc0b313b76c9a9791b6c9e96d397867cee91cd) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Show live connection quality for connected devices. Signal bars in the connection status show good, fair or poor quality, hovering shows the round trip and packet loss, and a connection that stays below good suggests a fix for its route, such as Ethernet or a USB cable instead of Wi-Fi.

- [`ff60ac3`](https://github.com/DaniGuardiola/extend.computer/commit/ff60ac3f706671ab5ae881111da9711074a85796) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Display settings now offer 120 FPS when the other device's screen can show it, such as a MacBook Pro or a 120 Hz monitor, instead of only frame rates used before. Choosing a lower frame rate no longer hides the higher ones.

- [`feaf8b3`](https://github.com/DaniGuardiola/extend.computer/commit/feaf8b36b70ef46337025aa97db313d7d367f22e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - A Mac connected by Ethernet, with Wi-Fi off or not joined to a network, can now accept connections without approving Wi-Fi optimization, instead of showing "Allow connections · USB only". Wi-Fi connections still need Wi-Fi optimization.

- [`a2a6882`](https://github.com/DaniGuardiola/extend.computer/commit/a2a6882508325a2c32e2871a1c001a2a8fd5a942) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Settings has a new Help section. Report a problem opens a new GitHub issue with the app and system versions filled in, and Copy diagnostic info copies a short summary of connection and permission states that you can read first; it leaves out device names, keys, addresses and your email.

- [`103e044`](https://github.com/DaniGuardiola/extend.computer/commit/103e044401eb52e34bb611167b8dff5c7c92e88e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - When another device starts controlling this computer or showing a display on it, extend.computer posts a notification naming the device, and its menu bar icon shows a dot until that session ends.

- [`be32574`](https://github.com/DaniGuardiola/extend.computer/commit/be32574ef6956ac58b0b7d0ce866b5ce03cb1e4a) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Sign in with a passkey or security key directly in the app on extend.computer accounts, without switching to the browser. This also works for accounts that use a passkey or security key as their second factor. Custom account servers keep passkey sign-in through the browser.

- [`ecb9ebd`](https://github.com/DaniGuardiola/extend.computer/commit/ecb9ebde465974b856ae841bf153acd9e048fc39) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Turn on Open at login in Settings to start extend.computer in the menu bar when you log in, ready for your devices to connect.

- [`ecb9ebd`](https://github.com/DaniGuardiola/extend.computer/commit/ecb9ebde465974b856ae841bf153acd9e048fc39) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Closing the window keeps extend.computer running, so your devices can still connect. Its icon in the menu bar (the notification area on Windows) shows whether connections are allowed and the current session, and can turn Allow connections on or off, end the session, open the window or quit.

- [`319cc6c`](https://github.com/DaniGuardiola/extend.computer/commit/319cc6cf3a262c4ff2c4a7a822ffb3972f32a0ad) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - A Windows PC with the extend.computer display driver can now extend its desktop onto another device. The new display appears right of your other displays the first time, comes back where you arranged it for that device and frame rate after that, and goes away when the connection ends. Without the driver, Extend display says what it needs.

- [`37af216`](https://github.com/DaniGuardiola/extend.computer/commit/37af21643ef8a1aa08c2d24666c271b1600edddf) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Extending a Mac to a Windows PC whose display is scaled to 150% or more now gives a sharp Retina display that matches the PC's screen pixel for pixel, and the Retina setting is available for it. PCs at 100% or 125% keep their full resolution.

- [`d68e621`](https://github.com/DaniGuardiola/extend.computer/commit/d68e62136a9cb7c50134fbe6898a407fcdd2d88e) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - A Mac can now extend or mirror its screen to a Windows PC running extend.computer. Devices now tell each other what they support, so the Mac picks a video format the other device can show, and Extend, Mirror and Share keyboard & mouse explain when a device can't do them instead of failing after connecting.

- [`b5ac9ea`](https://github.com/DaniGuardiola/extend.computer/commit/b5ac9ea1feed49e636588e051d075d65dfdfa67d) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Make the app bigger or smaller with Command (Control on Windows) and plus or minus, from 80% to 150%, and press Command or Control and 0 to go back to 100%. The app remembers your zoom level.

### Patch Changes

- [`9913018`](https://github.com/DaniGuardiola/extend.computer/commit/9913018498a3dd5cd594ae4574ca698bdf1b961f) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Learn more next to Use a custom account server now opens a guide to running your own account server and signing in to it, instead of a placeholder page.

- [`821d2fb`](https://github.com/DaniGuardiola/extend.computer/commit/821d2fb238f0b4cb55f8a3696ee45966c8715d1f) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Switch between a USB cable and the local network within seconds when the cable is plugged in or removed, instead of losing the device until the app restarts. A connection error on a device now clears once that device is reachable again.

- [`4a804ba`](https://github.com/DaniGuardiola/extend.computer/commit/4a804bac273fbb27d31f7622d69fa4ec6ee84199) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Changing the extended display's size no longer makes other monitors look darker or warmer. The extended display now offers its sizes from the start, so changing size switches modes instead of rebuilding it, which had made macOS reset other screens' color profiles until disconnecting.

- [`c33f448`](https://github.com/DaniGuardiola/extend.computer/commit/c33f448156dc15432b4dd95abb938ad7e476b621) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Fix extending a display failing on the first attempt with "placement rejected": the app now waits for macOS to bring the new display online before placing it.

- [`d2a25b5`](https://github.com/DaniGuardiola/extend.computer/commit/d2a25b559bb94497a644d9b503d6e43b898bc964) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Keep keyboard and mouse sharing smooth while the app is in the background, and send right-hand Shift, Control, Option and Command keys as themselves instead of their left-hand counterparts.

- [`d94658a`](https://github.com/DaniGuardiola/extend.computer/commit/d94658abc9849fb1909520befa84c5e72ade5723) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Computers stay awake during keyboard and mouse and display sessions, and a Mac showing another device's display keeps its screen on instead of dimming and going to sleep.

- [`72ae3f9`](https://github.com/DaniGuardiola/extend.computer/commit/72ae3f9c51d95029942f1ed20ac4a80cbfbbc2a4) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Phones and tablets no longer show a dimmed Remote desktop tile, since they have no desktop to view. Computers still show it as coming later.

- [`03ab278`](https://github.com/DaniGuardiola/extend.computer/commit/03ab27869413e72a2a93592b40ee3233f64facb1) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - The first Extend display or Mirror display from a Mac now asks for Screen Recording before starting, and starts the display by itself once it is allowed, instead of failing and asking you to reconnect. Settings › Permissions also shows Screen Recording.

- [`49be8d3`](https://github.com/DaniGuardiola/extend.computer/commit/49be8d342e68d59ebc09d7a424fb16ca144880f3) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Settings › Updates no longer offers Beta, Alpha and Canary before they have any releases, so update checks don't fail on an empty channel. On Windows, Updates now says automatic updates aren't available there yet, instead of calling the app a development build.

- [`6914d91`](https://github.com/DaniGuardiola/extend.computer/commit/6914d919d833fd86c06b8611a0c6f758cf792b53) Thanks [@DaniGuardiola](https://github.com/DaniGuardiola)! - Mirroring a Windows PC to a screen of another shape now keeps the picture's proportions, with black bars at the edges, instead of stretching it.

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
