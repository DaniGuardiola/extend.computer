# Android application

React Native and Expo Router own the platform UI. NativeWind provides utility
styles; `npm run theme:sync` reads the desktop's color tokens. Native Android
MediaCodec and SurfaceView handle video without routing frame data through JS.

## Local development

Use a supported Node release (22.13+, 24.3+, or 25+) and JDK 17. Install the
Android SDK, platform/build-tools 36 and NDK 27.1.12297006. Then:

```sh
cd mobile
npm ci
rustup target add aarch64-linux-android
export JAVA_HOME=/path/to/jdk-17
export ANDROID_HOME=/path/to/android-sdk
sh scripts/build-android.sh
```

The resulting `android/app/build/outputs/apk/release/app-release.apk` is a
standalone **development** APK, with the development package identity and
local debug signing. It is not a production release. Native folders are generated
by Expo; make native changes in `modules/`, app configuration, or config plugins.
Gradle builds the shared Rust engine JNI adapter with the workspace's pinned
Rust toolchain. The current local build targets arm64 and links native engine
segments for Android's 16 KB page size.

From the repository root, with an authorized USB tablet:

```sh
python3 scripts/android/usb-display.py --serial TABLET_SERIAL
```

This development bridge creates an unmirrored Mac desktop and uses an
ADB loopback tunnel with a fresh capability token. HEVC is the default;
`--codec h264` selects AVC. `--mode mirror` mirrors the Mac's main display.
Stop with Ctrl-C or the tablet's Back control. `--skip-install` uses the
already installed APK. Run `adb devices` to find the tablet serial.
The bridge requires an authorized physical USB transport and rejects wireless
ADB serials. macOS may separately ask to allow the USB accessory to connect;
that approval is distinct from Android's USB debugging authorization.

The native USB media transport is available in the local development application.
Release distribution remains unfinished. The shared native media engine accepts a non-UDP `DatagramLink`, retaining DTLS/SRTP and authenticated
SDP signaling. Encrypted 400 KB AVC and HEVC access units reassemble through
bounded one-packet native test queues, including actual queue backpressure;
this is an in-process transport check, not physical USB video or a benchmark.
`display::packet_link` adds bounded byte-stream framing for a native bulk worker:
partial/coalesced reads preserve packet boundaries, malformed lengths close the
link before allocating payloads, and full queues retain unconsumed input. Actual
DTLS/SRTP AVC and HEVC frames also pass through seven-byte stream fragments.
The native Android accessory service and macOS libusb host now carry encrypted
media directly over physical bulk endpoints. The tablet accepts only a descriptor
whose host fingerprint is already paired; OS USB permission alone does not grant
peer trust. Bounded multiplexing supports independent display sessions, with
partial read/write handling and disconnect propagation. A physical tablet
check rendered at least 120 AVC frames and 120 HEVC frames at 1280×800/30 FPS,
with screenshot color/checker validation and explicit `route: "usb"` telemetry.
This is a functional media check, not an optical latency or Duet comparison.
The desktop runtime now selects the native carrier for the exact paired peer in
both directions. Its packaged helper and bundled libusb load under both local
development and Developer ID signatures. Real tablet-to-Mac capture passed AVC
and HEVC at 1280×800/30-fps requested mode, with Metal presentation and projection
stop cleanup; HEVC pixels were inspected in the native viewer. Real desktop UI
extension to the tablet passed HEVC at 1920×1200/60-fps requested mode, with over
120 native rendered callbacks and a screenshot of the extended macOS desktop.
Source queue overflows can occur and are a known performance/recovery limit.
The paired device's desktop **Connect with USB** action now lists attached Android
accessory-capable devices and requires an explicit device selection. Inventory
queries do not switch USB mode or grant trust; pinned media authentication still
verifies the paired peer. A normal connection and cancellation passed on the
physical tablet without an ADB media tunnel. Initial automatic mirroring selected
hardware-supported 1920×1200 HEVC and produced 4,200 rendered callbacks over USB.
After the required macOS Screen Recording grant, a fresh normal GUI launch showed
the tablet Online and mirroring produced 3,840 callbacks over LAN. The actual
mirrored desktop was inspected on the tablet. These counts establish functional
rendering, not achieved frame rate or latency.

Pairing and authenticated signaling still use LAN, including for USB video.
Accessory mode may require unplugging and reconnecting the cable before another
connection. The observed stale device selection after cancellation/disconnection
has a source fix that requires a physical recheck. USB-only signaling, Intel
runtime verification, universal/release distribution and optical measurements of
the current application remain unfinished.

The generic probe requires an exact vendor and serial; it never sends accessory
strings, switches USB mode, or claims an interface:

```sh
cc -Wall -Wextra -Werror scripts/android/aoa-probe.c \
  $(pkg-config --cflags --libs libusb-1.0) -o out/extend-aoa-probe
out/extend-aoa-probe VENDOR_HEX TABLET_SERIAL
```

The receiver has been verified on a Redmi Pad 2 with Android 16, using
1280×800 at 60 Hz and hardware HEVC/AVC decoders. The decoder checks advertised
size/rate support rather than assuming native tablet resolution works at 60 Hz.

## Current scope

The receiver, adaptive UI, fullscreen controls and persistent appearance setting
work. The shared Rust engine now initializes a durable device identity and trust
store. Android Keystore encrypts the private seed in app-private storage;
private key bytes never cross the React bridge. Corrupt identity storage fails
initialization rather than silently replacing the device identity.

Code pairing, pinned authenticated probes and local forgetting use the same
Rust session protocol and trust store as desktop. The mobile UI saves peer names
and addresses across app restarts. The Devices screen checks paired peers through
authenticated presence, refreshes while focused and foregrounded, and expires
status evidence after 25 seconds. Online, Offline, Not accepting connections and
Unavailable are distinct states. Connection actions appear only with fresh Online
evidence and are guarded while another connection is active. Physical Mac/tablet
checks showed both devices Online; turning Mac receiving off changed Android to
Not accepting connections and hid its connection actions, then turning it on
restored Online.

Pair device uses Android's native DNS-SD browser while the screen is active.
Selecting a nearby device starts visual pairing: compare all eight ordered symbols
and their numbered labels on both screens, then approve on both devices. Discovery
never grants trust. Android-to-Mac and Mac-to-Android pairing both passed through
the real application UIs on Android 16, with matching symbols, explicit approvals
and authenticated device names. Pair manually is a separate screen with name,
address and code fields, with Show my code as the receiving path. Nearby device
selection appears only on the easy-pairing page. Symbol confirmation hides the
discovery status and navigation until approval finishes. The focused confirmation
and waiting state were checked on the tablet. Older Android resolution paths,
scoped IPv6, and physical visual denial, cancellation and expiry remain unverified.

Outgoing manual pairing has an explicit Cancel pairing action. A native
reservation is created before async startup; leaving the screen, backgrounding,
or losing app focus invalidates it and interrupts its captured socket. Numeric
connection attempts check cancellation every 200 ms, and native pairing expires
after two minutes. Local trust is committed only after the authenticated remote
grant, under the same lock as cancellation. A completed pairing remains paired;
cancellation cannot undo approval already committed on the other device. Host
PAKE/consent-race and lifecycle checks pass; the new outgoing cancellation UI
still needs physical acceptance on the next APK.

Show pairing code opens a foreground-only native listener and DNS-SD advertisement
for two minutes. The tablet requires explicit Allow/Deny for a device that entered
the code; the shared engine authenticates it before showing the request. Leaving
the screen or backgrounding the app closes the code, interrupts pending sockets,
and withdraws the advertisement. Incoming code pairing, pinned LAN reconnect,
explicit consent, denial, pending-request cancellation and screen-exit cleanup
were exercised on the tablet.
Peer names exchange only over pinned, trusted sessions and persist in app-private
storage across process restarts. User-entered names retain priority. Incoming
sessions save only their authenticated source IP as a route hint. Check connection
tries pinned presence on that host and bounded nearby anonymous presence candidates;
a verified reply supplies the current receiving port. A successful pinned probe is
required before replacing the saved address. A connection's source port is never
saved as a listening endpoint. Discovery failure retains existing trust/routes.

Devices → Allow connections and Settings → Stay available control the same Android
connected-device foreground service with an ongoing notification and Stop action.
Its native listener accepts only paired reconnects/name exchanges, and a separate
read-only presence endpoint uses
the desktop's pinned presence protocol. Anonymous DNS-SD suggests presence
endpoints; authenticated replies supply the current receiving port and name.
Physical tablet checks passed for background reconnect, app-restart reconnect,
pinned port repair, unknown-peer rejection, persistent names, and switch-off
socket/service cleanup. Reopening the app restores enabled availability after
a force stop; boot startup and long Doze runs are not verified. This is session
availability groundwork: full media/input lifecycle verification remains unfinished.

Real macOS application interoperability also passed: the desktop discovered the
tablet, manual code pairing required tablet consent, and the desktop showed Redmi
Pad 2 as Online after the code listener closed. The tablet authenticated the real
Mac, repaired its receiving address, and reconnected after an application restart.
The later visual-pairing checks restored the actual Mac/tablet pairing and Online
availability in both applications. Nearby advertisement has a two-minute window;
backgrounding closes it. Retry nearby pairing or reopening Pair device creates a
fresh window.

The account backend accepts Android devices through the existing native API.
The local loopback HTTP regression in
[`server/tests/android-device-http.rs`](../server/tests/android-device-http.rs)
passed registration, shared native identity proof, device listing, heartbeat and
revocation with a temporary database. It also rejects wrong-key/replayed proofs,
cross-account removal and misuse of device credentials, and verifies registration
credential rotation. This is a local backend check, separate from the client.

The Account screen implements the existing desktop browser/PKCE contract,
cancelable five-minute native callbacks and an API-only password/MFA fallback.
Authentication requests go from native code to the selected account server over
HTTPS, with localhost HTTP allowed for development. Passwords, second-factor
tickets, PKCE verifiers and bearer credentials never cross the React Native
bridge; React Native receives only sanitized account/device state. Saved tokens
are encrypted with a non-exportable Android Keystore AES-GCM key in origin-bound
private no-backup records. The native client registers Android identity, completes
the shared possession proof, binds cached device credentials to the current native
fingerprint, heartbeats and refreshes verified account peers with
a 90-second trust lifetime. Removal and sign-out retire account-only sessions
while preserving direct pairing. Account devices appear in Devices; connection
checks still authenticate identity and local route availability.

The host regression uses actual loopback HTTP to check PKCE state/replay,
password/MFA fallback, registration/proof request shape, directory/heartbeat,
restore, identity-binding/migration recovery, removal, expiration,
canceled/unsaved-token revocation, callback header
bounds, slow-byte total deadlines, idle-socket cancellation, blocked-worker native
expiry, cleanup failure recovery, redirect refusal and response limits. It injects test storage and identity;
it does not verify Android Keystore or browser behavior on a physical device.
Run it with an SDK and JDK 17:

```sh
python3 scripts/android/test_accounts.py --sdk "$ANDROID_HOME" --java-bin "$JAVA_HOME/bin"
```

Physical account flows, damaged-record/install lifecycle and deployed account
interoperability remain unverified. Account-based Internet relay is not yet wired
into Android; these account connections currently use the local network. API-only
servers support native password plus authenticator/recovery-code login; signup,
passkeys/security keys and account settings require their account website.

The ADB USB video bridge remains a development harness. The ordinary application
uses paired shared-engine sessions for native LAN and USB media in both directions,
including Android screen capture.
See [PARITY.md](PARITY.md) for the full desktop comparison. Android
capture and remote input use the relevant OS permission flows.

With display sessions stopped, verify the installed native engine on a device:

```sh
python3 scripts/android/test_engine_device.py \
  --sdk "$ANDROID_HOME" --java-bin "$JAVA_HOME/bin" --serial TABLET_SERIAL
```

This app-signed instrumentation checks key wrapping, ciphertext tamper rejection,
account proof validation and identity durability across fresh application
processes. Its temporary tamper check restores the original encrypted record.
For pairing interoperability, build `cargo build -p extend-android-engine
--example tablet_pairing_peer`, run it with a fresh private fixture JSON path,
reverse its emitted loopback port through ADB, then pass `--pair-fixture FILE`
to the instrumentation runner. The fixture uses a temporary identity and trust
store, accepts three test connections and never changes desktop account state.

For a real LAN discovery test, run the fixture with `--advertise` instead of
creating an ADB reverse tunnel. Pass `--discovery-name 'Discovery Test Mac'` and
`--pair-fixture FILE` to the runner. The native test resolves the advertised
endpoint itself, checks discovery cleanup, then verifies pairing and reconnect
across fresh application processes. Keep the fixture code private and stop its
process after testing.

The test APK also contains an accessibility driver for debugging other apps:
`python3 scripts/android/ui.py --serial TABLET_SERIAL --package PACKAGE_ID`.
It excludes password controls and is absent from the mobile application. Each
invocation restarts the instrumentation target, so use the keyboard/UI harness
for multi-step interactions with the Extend app itself. The runner's
`--build-only` option builds the test APK without starting instrumentation.
It also builds a separate `observer-test.apk`. Install that APK and pass
`--preserve-services` to `ui.py` when inspecting an active accessibility receiver:
this observer preserves accessibility services and avoids restarting the target
app. Ordinary shell `uiautomator dump` suppresses those services during observation.

To exercise the actual React Native pairing form and device controls, use
`python3 scripts/android/test_pairing_ui.py --serial TABLET_SERIAL` after
building that example and installing the development app. This UI harness
requires an empty paired-device list, creates one temporary test peer, checks
reconnect after restart, and forgets only that test peer through confirmation.
It writes a screenshot and receipt under a fresh ignored `.local/` directory.

For the incoming flow, build the `tablet_pairing_client` example and run
`python3 scripts/android/test_incoming_pairing_ui.py --serial TABLET_SERIAL
--tablet-peer VERIFIED_FINGERPRINT`. The runner enters the real Show pairing code
screen, uses a temporary desktop identity over LAN, verifies the consent identifies
that exact peer, reconnects with the tablet identity pinned, closes the listener,
and forgets only its test peer. `--decision deny` or `--decision cancel` checks
rejection/exit without establishing trust. It requires an empty paired list,
keeps the one-use code in a private ignored fixture, and deletes that fixture on
exit. The test does not change desktop user trust or account state.
Add `--receiving` to exercise the foreground-service setting, its normal
notification consent, anonymous DNS-SD, pinned presence port repair, background
and restarted-app reconnect, Android UI return-route repair over ephemeral presence
ports, persisted route reconnect after restart, and switch-off teardown. This variant requires
availability initially off and restores that preference, including on failure.

## Trusted native media

Availability now also listens on the desktop display protocol's port 48179.
Pinned shared-engine signaling negotiates encrypted LAN UDP or native USB bulk
media; only a locally trusted peer can open an offer. The foreground React app opens the
viewer, whose native SurfaceView/MediaCodec worker must confirm readiness before
the answer is sent. Encoded bytes stay in Rust/Java and never cross React.
Hardware decoder capabilities select automatic dimensions, and unsupported
explicit dimensions fail rather than silently using software decoding. Separate
cursor updates render natively independently of video decoding. Android API31+
defaults to a SurfaceControl child of the admitted video surface; exact session,
configuration, decoded raster and layout-holder geometry gates must pass first.
Older Android, uncertain geometry, changed raster/logical profiles or backend
failure use the Canvas overlay. Set `log.tag.ExtendCursorCanvas=DEBUG` before
reconnecting to force Canvas for diagnosis; clear that property to restore the
default. `log.tag.ExtendCursorTrace=DEBUG` separately enables bounded timing
instrumentation, which is off by default. Native transaction submission and
completion timestamps do not themselves prove physical presentation. Physical
AVC and HEVC USB checks verified cursor shape and hotspot pixels before video,
movement while video was paused, and removal when hidden. Cursor and video bytes
remain native; these checks do not measure optical cursor latency.

The bounded encoded queue never drops reference frames to hide a stalled decoder.
Network discontinuity requests a fresh keyframe; decoded output may skip stale
presentation buffers. Back/view detachment or disabling availability closes the
signaling/media session. Same-codec live geometry, frame rate and bitrate changes
now recreate the hardware decoder within the same media session; native rendered
callbacks are tracked separately for each configuration revision. Codec changes
still require stopping the session. Rotation, incoming offers while backgrounded,
release USB distribution and long-run recovery remain unfinished.

Physical generated-pattern checks passed on Redmi Pad 2 for hardware AVC and
HEVC at 1280×800@30, including 120 actual rendered callbacks per codec and visible
color/checkerboard output. These are functional decoder checks, not latency
benchmarks. Actual desktop extension over native USB and automatic desktop
mirroring over native USB and LAN were verified through the normal desktop UI,
with the required macOS Screen Recording permission granted. Mirror configurations with dimensions the hardware decoder does not support are rejected; automatic mode selects a supported size such as 1920×1200.

The receiver accepts same-codec live size, frame-rate and bitrate changes through
pinned shared signaling. It clears queued frames, validates the new hardware
decoder settings, releases the previous decoder and acknowledges the handoff.
Native frame metadata and render callbacks carry a configuration revision so old
callbacks cannot update the new stream's status. A fresh keyframe initializes the
new decoder. Live automatic sizing, mode changes and codec switches still require
work; these restrictions are implementation gaps, not established OS limitations.
Physical generated-pattern checks on Android 16 passed for H.264 and HEVC:
1280×800 at 8 Mbps changed to 960×600 at 4 Mbps on the same session, with at least
120 actual rendered frames after each switch and matching pattern pixels.
This establishes functional configuration behavior, not optical latency.

Generate deterministic patterns without capturing any screen, then run the
physical encrypted-media check with availability on and Devices visible:

```sh
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
  native/macos/Display/Metrics.swift native/macos/Display/BenchmarkMarker.swift \
  native/macos/Display/VideoCodec.swift scripts/android/media-pattern.swift \
  -o native/android/.local/media-pattern
native/android/.local/media-pattern h264 > native/android/.local/media-pattern.h264.frames
native/android/.local/media-pattern hevc > native/android/.local/media-pattern.hevc.frames
# Optional fixtures for the live configuration gate:
native/android/.local/media-pattern h264 960 600 > native/android/.local/media-pattern-960.h264.frames
native/android/.local/media-pattern hevc 960 600 > native/android/.local/media-pattern-960.hevc.frames
cargo build -p extend-android-engine --example tablet_media_source
python3 scripts/android/test_media_ui.py --serial TABLET_SERIAL --tablet-peer VERIFIED_FINGERPRINT \
  --preserve-name 'An existing paired device' --image-python PYTHON_WITH_PILLOW
# Add --resize to exercise both live decoder handoffs with the optional fixtures.
```

Screenshot validation requires Pillow; `--image-python` selects its Python runtime.
The test also preserves source PTS, codec-reported render time and callback
delivery time separately, and requires the two tablet clock samples to agree
within one second. This sanity gate does not establish physical display latency.
Decoded buffers use the tablet clock for SurfaceView submission, not source PTS.
The optional repeated `--preserve-name` asserts existing devices remain visible.
The test requires its temporary name to be absent initially, verifies its exact
consent fingerprint, and removes only that newly created peer. Private one-use
codes are kept in ignored mode-0600 fixtures and deleted on exit. If cleanup
cannot identify its action safely, it fails without deleting another peer.

## Native keyboard and mouse sending

A paired device's Keyboard & mouse action opens a native trackpad for the Mac's
main screen. Movement, tap-to-click, held dragging, two-finger scrolling/right
click and physical keyboard events stay native. React owns status and occasional
toolbar actions. The shared engine pins the peer, requests the desktop control
protocol, bounds the event queue and maintains session heartbeats. Leaving the
screen or backgrounding the app closes the transport and releases held input.

Physical Redmi Pad 2 checks against a disposable native Mac window passed for
movement, left/right clicks, typing, toolbar Tab, long-press dragging and mouse
release on background. Native transport tests cover ordered events, pin rejection
and cancellation. Multi-touch scrolling and soft keyboard interaction still need
physical verification. The current soft keyboard accepts short US-layout text;
composed Unicode and other layouts remain unfinished. This controls the main
screen. Android input receiving is described below. Display-scoped touch input
is implemented separately in the native video viewer, with an opt-in Touch control
and a small reachable menu. It gates gestures on a rendered configuration and a
fresh pinned input grant; fitted video margins reject input. Native tap, repeated
tap, held drag, two-finger scroll/right-click and lifecycle release logic have
passed pure gesture and native queue checks, and the development APK builds.
Physical display-scoped touch acceptance remains pending.

For the physical check, compile `scripts/android/input-target.swift` into a fresh
disposable Mac app bundle, run it with a fresh JSONL event-log path, and focus its
window. It records only Extend-tagged input delivered to its own window. Keep the
real Mac paired and receiving enabled, then run:

```sh
python3 scripts/android/test_control_ui.py --serial TABLET_SERIAL \
  --device-name 'The paired Mac name' --window-event-log /path/to/events.jsonl \
  --output /path/to/fresh-test-directory
```

The app-signed test APK also contains `ControlInstrumentation` for direct native
diagnosis with an explicitly verified `peer` fingerprint. That bypasses the
trackpad UI and does not establish gesture behavior. Reopen the mobile app after
instrumentation to restore its enabled availability service.

## Native Android input receiving

Settings → Receive keyboard & mouse is off by default and requires Android's
accessibility service permission plus Stay available. The receiver accepts only
paired shared-engine sessions. Cursor updates and ordered input use a bounded
native mailbox; adjacent motion can coalesce, while button/key transitions retain
order. Losing permission, platform polling or display geometry revokes the session.
The service rejects control while the device is locked. Accessibility window
content is not retrieved, recorded or transmitted by the application.

The native Android adapter shows a cursor and sends touch gestures. It currently
maps right click to long press and middle click to Home. On Android 13 and newer,
mapped physical keys use the accessibility input method's focused editor connection.
General app shortcuts outside an editor, older Android keyboard paths, other
layouts and broader gesture/keyboard coverage remain unfinished. Transport receipts
acknowledge native queue admission; they do not measure physical input latency.

Physical Redmi Pad 2 checks passed for a pinned encrypted Mac fixture clicking a
native Android button, typing into a native field, scrolling, a two-second
stationary hold, continued dragging, releasing held touch on disconnect, and
preserving the real Mac pairing. Releasing input also leaves the control session
usable. Stationary contacts stay held without dispatching empty continuation
strokes, which Android rejects. The
disposable target and observer ship only in test APKs. Build/install both test
APKs, keep orientation fixed and the Devices screen visible, then run:

```sh
cargo build -p extend-android-engine --example tablet_input_source
python3 scripts/android/test_input_receiving_ui.py --serial TABLET_SERIAL \
  --tablet-peer VERIFIED_FINGERPRINT --preserve-name 'An existing paired device'
```

The runner verifies its exact temporary peer at consent, uses pinned presence to
find the receiving port, observes native UI results, and removes only that peer.
Its private one-use pairing fixture is deleted on exit. A failed cleanup preserves
the peer for inspection rather than risking another device's trust.

## Optical diagnostics

Shared camera/workload/analysis code lives in `benchmarks/display/`. With the
camera helper installed and both screens visible:

```sh
benchmarks/display/automation/.venv/bin/python \
  benchmarks/display/automation/android_display.py \
  --tablet TABLET_SERIAL --camera CAMERA_SERIAL \
  --output /path/to/new-run --show-report
```

The wrapper sets source and receiver to fixed maximum brightness through native
APIs/ADB, saves original settings before changes, and restores them on success,
failure or normal interruption. A saved `brightness-original.json` supports
manual recovery through `brightness.py --snapshot FILE --restore` after a forced
process kill. The camera phone is a sensor, not a benchmark display.

Default inward patches suit a laptop on the left and tablet on the right;
`--patch-sides left,right` supports the opposite arrangement. Physical timing
requires readable counters, stable framing and matching source geometry.
Diagnostics report screen-to-screen transitions, including panel scanout and
camera effects. They do not establish calibrated input latency or product parity.
External product adapters and their results belong in their separate private repo.

Optical comparisons are tied to the recorded display geometry and application build; the functional USB and mirror checks make no latency claim.

## Send the Android screen

A paired device's **Share screen** action opens a React Native screen with Start
and Stop controls. Start first verifies the paired route, then opens Android's
system capture consent. Consent is requested for every new projection; tokens
are never persisted or passed to JavaScript. The native foreground service keeps
capture running when another application is visible and provides a notification
Stop action.

The source path mirrors the Android screen using a hardware AVC or HEVC Surface
encoder. Per-device controls select 1280 or 1920 pixels on the longest edge or
the screen size, 30/60/120 fps, and an 8/20/40 Mbps quality budget. Defaults are
1280 pixels, 60 fps, 20 Mbps and H.264. Native hardware capability checks reject
unsupported settings before opening capture consent or replacing a running
encoder. Resolution, frame rate and quality can change while streaming. Controls
wait for the native handoff and save only after the new encoder starts; hardware
rejections preserve the active stream and previous preferences. Codec changes
currently require stopping capture first. Portrait sizes keep the
shared protocol's minimum width. Native codec output crosses JNI
into shared pinned signaling and encrypted LAN UDP or native USB bulk media.
JavaScript
receives status only. The sender queue holds at most four encoded frames or
8 MiB and requests a fresh keyframe after overflow or transport recovery. A
forgotten peer, closed signal, failed encoder or stopped projection ends capture.

Captured-content size changes reconfigure the pinned native session, detach the
old encoder Surface, and resize the existing VirtualDisplay before attaching a
new hardware encoder. The same projection consent and transport session remain
active. Queued frames from the old size are discarded and a fresh keyframe is
requested. No video passes through JavaScript during this transition.

Physical Redmi Pad 2 → macOS checks showed the real tablet screen in the native
Metal viewer, then a separate Android test activity while the mobile application
was in the background. System consent cancellation started no projection.
Returning from the app selector cancels pending consent. Native Stop also finishes
the pending consent activity. Real
selected-app capture was verified separately: the chosen engine-test activity
appeared in the Mac viewer with Android status/navigation bars excluded. A fresh
selector could be opened after abandoning the previous selection. App
Stop removed the active projection and foreground capture service and closed the
Mac viewer. These are functional checks, not comparative latency measurements.
Both H.264 and HEVC were verified at 1280×800 with a 60 fps requested mode and
20 Mbps budget. HEVC was additionally verified at 1920×1200 with a 30 fps requested
mode and 40 Mbps budget. Preferences survived navigation and were restored after
the tests. Portrait-to-landscape source resizing was verified on Android 16 with
both codecs: 800×1280 and 1280×800 configurations sent fresh frames under each
revision, retained the same transport session, and showed correctly reflowed
pixels in the Mac viewer. Rotation settings were restored after each run. The
Android 29–33 configuration-change fallback still needs physical verification.
Live source UI changes were also verified with both codecs: quality changed from
20 to 8 Mbps, frame rate from 60 to 30 fps, and resolution from 1280×800 to
1920×1200 without replacing the transport session or requesting new consent.
Fresh frames were sent after each change and the Mac viewer showed current
pixels. A rejected 120 fps choice kept capture running with 30 fps selected.
The updated choices survived leaving and reopening the sharing screen; original
preferences were then restored. These are requested configurations, not achieved
frame-rate or bandwidth measurements.
This does not establish sustained 60 fps delivery. Notification/OS Stop,
selected-app visibility and split-screen behavior, other capture profiles,
live codec changes and Android display extension still require implementation or
verification. The Redmi Pad 2 hardware
capability checks rejected 2560×1600 capture for both codecs at every tested
30/60/120 fps and 8/20/40 Mbps combination, and accepted 1920×1200 at 30 fps.
These capability checks do not establish achieved capture performance. Android's
capture requirements are documented in the
[MediaProjection guide](https://developer.android.com/media/grow/media-projection).


The generic source lifecycle runner uses the observer APK and a live local desktop
receiver metrics log. Keep Devices visible and the paired desktop accepting
connections. Use `--capture-scope app` for the disposable activity. The experimental
`--stop-control notification` path still needs an OEM-compatible notification-panel
flow: it could not find the action on this tablet and is not a passing gate. A
bounded inspection window lets the receiver's actual pixels be checked separately
from telemetry:

```sh
python3 scripts/android/test_screen_sharing_ui.py --serial TABLET_SERIAL \
  --peer VERIFIED_DESKTOP_FINGERPRINT --name 'Paired desktop name' \
  --desktop-log /absolute/path/to/live-desktop.log --codec hevc --fps 60 \
  --resolution 1280 --quality balanced --inspection-seconds 30
```

It preserves all pairings, collects fresh capture and receiver evidence, and stops
its projection and foreground service, checks persisted preferences, then restores
the original choices. `--rotation-check` additionally checks full-screen portrait
and landscape configurations, requires frames sent under each new revision, and
restores the original rotation settings even after failure. `--live-settings`
checks real quality/frame-rate/resolution controls and unsupported-profile
rejection from the default 1280/60/balanced full-screen baseline; it cannot be
combined with rotation in this runner. Each `ROTATION READY`
message starts the requested inspection window. A transport/renderer telemetry pass alone
is not a pixel-quality or optical-latency result.
