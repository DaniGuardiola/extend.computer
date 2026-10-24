# Display streaming

Extend display and mirror screen use LAN or active Mac-to-Mac USB networking between macOS apps. The source captures a virtual display or its main display; the receiver opens a native fullscreen viewer that can also be resized. Both use the same native hardware-codec pipeline and WebRTC transport. Native app connections request hardware HEVC (H.265); H.264 remains available through explicit codec negotiation. HEVC Main uses 4:2:0 color, so fine colored text can still lose detail.

## Mac-to-Mac USB networking

Mac-to-Mac cable connections use the macOS USB NCM network interface when it has an active IPv4 address. This interface appears as `AppleUSBNCMData` on the host and `AppleUSBDeviceNCMData` on the device; the classifier also accepts `AppleUSBDeviceNCM11Data`. Interface names and link-local addresses are discovered at runtime; they are not fixed configuration. This path uses the same authenticated signaling and encrypted WebRTC media as LAN streaming. Android accessory USB remains a separate carrier.

TCP and UDP sockets bind to the physical interface so an overlapping VPN route cannot redirect cable traffic. Source-address binding precedes the interface option on connecting sockets, and listening sockets apply the option after `listen()`, because macOS can reset the option in both operations. Concrete USB listeners share the wildcard listener's port using `SO_REUSEADDR`; another app cannot claim the same listening address. USB listeners refresh after cable and interface changes. No system routes or VPN settings are changed.

The ICE library rejects IPv4 link-local host candidates. When both signaling endpoints use distinct link-local IPv4 addresses, the transport presents private logical addresses to ICE and maps datagrams back to the signaling peer's physical IP. The real UDP socket retains its USB address and interface binding; SDP cannot redirect mapped traffic to another IP, and datagrams from another physical IP are ignored. This translation requires both peers to support the cable transport.

Pinned USB presence replies take priority over a faster Wi-Fi reply, with a bounded fallback when the cable candidate fails or stalls. Where multicast discovery is blocked, known peers can also be found using existing ARP neighbors on active USB interfaces; the app reads the numeric neighbor table with a bounded local command and performs the same pinned presence checks. It does not scan the subnet or use neighbor data to establish trust. The app reports `usb` only when the verified presence or session socket uses an active NCM interface. Receiving over that interface requires the native input helper and Accessibility; Wi-Fi connections still require Wi-Fi optimization. Background permission checks on an active cable or idle NCM interface avoid the Wi-Fi broker; native permissions are still checked, and a late result cannot overwrite a replacement session. This requires macOS to expose USB networking for the attached cable and devices; a charge-only cable or an inactive interface cannot carry a session. Transport tests and socket routing checks alone do not establish physical display latency or superiority over another product.

## Build and checks

```sh
npm run native:display --prefix desktop
python3 scripts/macos/test-display-parent.py
sh scripts/macos/test-display-codec.sh
cargo test -p extend-computer-agent --lib display
cargo run -p extend-computer-agent --example display-smoke -- extend retina
```

The smoke test creates a real virtual display, sends captured video through encrypted loopback WebRTC, and waits for the viewer's Metal command to complete. It then releases the virtual display and both helpers. Screen Recording permission is required for capture. The helper's `self-test` command checks hardware encode/decode and framing with a synthetic image, without capturing the screen.

The parent-exit check verifies that the native helper exits after either SIGTERM or SIGKILL terminates its owning process, even when stdin remains open. This prevents an abandoned helper from retaining a virtual display after an app crash. The check uses a synthetic helper mode; it does not by itself verify virtual-display teardown during a real session.

The codec check compiles the actual native decoder source, compares Annex B parsing against the previous parser on malformed and sliced inputs, and verifies that returned NAL storage survives input mutation. Its alternating warmed microbenchmarks measure parser cost only; they are not evidence of end-to-end latency or total application CPU savings.

## Validation status

Display streaming has been exercised on two Macs over LAN and over the Mac-to-Mac USB cable, in both directions, for Extend and Mirror. Checks covered native Metal presentation on the receiver, clean stop and reconnect, Retina streaming (for example 1440×900 logical encoded at 2880×1800 with a 60 FPS capture limit and an adaptive 20 Mbps ceiling), automatic sizing that fits the receiving screen, and a guarded click plus committed Unicode text delivered to a dedicated source window over the cable.

What these checks do and do not show:

- Native software gates and cable motion and scroll runs reached roughly 55–60 frames/s through encode, decode and presentation, with medians of about 20 ms encode, 2 ms decode and 8 ms decoded-to-drawable. Optical screen-to-screen medians at that profile were in the 30–46 ms range across sessions; the spread between sessions is unexplained. Native stage timings are host-local software intervals, not screen-to-screen latency or panel scanout.
- ScreenCaptureKit reports idle status for frames where it decides the display did not change, so accepted new images do not consistently reach 60 Hz even when callbacks do. The records cannot distinguish compositor coalescing from an unchanged display.
- The capture limit is not a delivered frame rate. Early fixed 2560×1440 runs presented about 41–46 distinct frames/s in animated scenes, below the 60 FPS target.
- The optical session measured one direction and profile. Reverse-direction runs are functional checks only.
- Static cursor raster, hotspot and visibility checks pass, as do full-screen digital screenshot comparisons (strong text similarity, larger errors on fine colored stripes). Neither measures cursor latency, and the screenshot comparisons include scaling and color normalization, so they do not establish original-pixel or panel quality.
- Cross-host stage timing needs clock-offset measurements; raw wall-clock differences between hosts are not latency.

Startup and recovery behavior that is now in place:

- Capture has an eight-second deadline for the first accepted image, so a source that never delivers frames reports a specific capture-access error instead of waiting indefinitely. Capture can also resume after the source app and helper are paused briefly without replacing their processes.
- Native sender termination serializes start/configure/stop, stops capture and cursor work, and releases the virtual display before encoder draining and process exit. Fatal errors, broken output, EOF and parent loss share that cleanup path, with an independent bounded watchdog. Pure lifecycle and closed-pipe tests cover it.
- The UDP transport services up to 64 due pacer timeouts per drain call before returning to application I/O. An isolated 400 KB encrypted frame previously stalled with zero packet loss until the receiver's 500 ms reassembly deadline expired; a large-frame regression test now verifies complete reassembly within 400 ms, alongside packet-loss recovery and authenticated setup tests.
- The video retransmission history uses the transport library's default size. A packet-reordering deadline that is too short causes visible freezes.
- Locked or backgrounded receiver sessions produce no drawable presentation events, so presentation gates fail in that state. Acceptance runs need an unlocked receiver with the viewer in the foreground.

Known issues not yet explained:

- Rarely, after a capture first-image failure on a reconnect, the physical source screen stayed frozen or black until the Mac restarted. Removing owned helpers and virtual displays did not recover it, and diagnostics did not identify the cause.
- After a long idle gap following Stop, reconnect once ended on the receiver with `fixed USB route recovery grace expired`. Recurrence has not been confirmed.
- Physical cable unplug/replug with automatic session recovery, VPN reconnect recovery, bidirectional physical input, and steady 60 FPS remain unverified.
- VPN or firewall permission prompts (for example a per-endpoint prompt from a local firewall) can block packet probes and look like transport failures.

Optional diagnostics are enabled with `EXTEND_DISPLAY_METRICS=out/display-metrics.jsonl`. They record numeric frame timestamps and stage counts, not screen content. GPU completion establishes presentation work completion, not physical display scanout or motion-to-display latency. Use a fresh filename for each run.

Transport diagnostics also report numeric RTP packet totals, loss, NACKs, RTT, and socket buffer sizes. The send queue snapshot includes retransmissions and padding; its age is not a direct measurement of video latency.

## Cursor presentation

Cursor images and hotspots travel over a reliable WebRTC data channel. Normalized display positions and visibility travel over an unordered channel with no retransmissions, independently of video. The source samples position up to 120 times per second, coalesces changes, and sends an idle keepalive. The viewer renders a native cursor layer over cursor-free video, rejects stale sequences, and maps logical sizes through the same aspect fit as video. These are source cursor updates rather than predictions of viewer input.

Image size and encoded message length are bounded. Cursor state is cleared with the session and hidden after two seconds without position updates. Global cursor shape uses the macOS system cursor API; visibility uses an optional runtime-resolved compositor query because macOS no longer offers a supported global visibility API. If that query is absent, containment determines visibility. This platform dependency needs checking on new macOS versions.

The native synthetic self-test exercises both hardware codecs plus cursor hotspot, Retina logical sizing, letterboxing, hidden state, and stale/out-of-bounds positions. Transport tests verify that cursor state arrives before any video is sent, along with fragmented encrypted HEVC delivery.

## Viewer input

The native Mac viewer forwards pointer, button, drag, scroll, key, shortcut, and committed Unicode text events through the authenticated display session. Input capabilities are negotiated separately from video. The source grants control only after checking native Accessibility permission and the current display geometry; opening the viewer alone does not request control. The first deliberate input event requests a bounded lease. Only the foreground viewer can use it. Cable checks have delivered a guarded click and Unicode text to a dedicated source window; these check delivery and lifecycle, not physical input latency.

Local text composition stays in the viewer until committed. Committed text clears held modifiers before injection and restores them afterward; unsupported text and shortcut capabilities are rejected. Input queues are bounded, pointer positions are coalesced, commands are acknowledged, and the lease expires without fresh keepalives. Losing focus, changing the viewport or display configuration, revoking permission, closing the viewer, and stale grants release held input. An input denial leaves video running.

## Current limits

- Display streaming has local UDP host candidates only. STUN, TURN, direct internet traversal, and browser clients are not implemented.
- Display sessions do not yet reconnect automatically after their signaling connection ends.
- The viewer renders each newly selected decoded frame directly. A single pending main-thread callback replaces older decoded frames when delivery arrives in a burst; encoded reference frames are not discarded by this policy. GPU completion and drawable presentation remain separate telemetry stages.
- Device settings offer resolution, Retina density, 30/60/120 FPS, and Lower bandwidth/Balanced/Detailed quality before a connection and during an outgoing display session. Quality sets an 8/20/40 Mbps bitrate ceiling; actual bandwidth may be lower. Fit display selects the receiver's geometry for Extend and the source's geometry for Mirror. Encoded dimensions are capped at 4096 pixels per axis. Settings apply after both helpers acknowledge them, without replacing the virtual monitor. Resolution and density changes explicitly select and verify the macOS display mode before acknowledgment. A 60 Hz receiver cannot show 120 distinct frames per second.
- Mirror captures the main display. Audio and selecting another source display are not implemented.
- Virtual-display creation uses a small runtime-checked private macOS API adapter. Capture, hardware codecs, and presentation use public APIs.

Before claiming performance parity, measure motion-to-display latency, delivered frame cadence, text quality, CPU/GPU load, and recovery under packet loss on both Macs while they are idle.

## Repeatable benchmarks

The [display benchmark core](../../benchmarks/display/README.md) owns versioned synthetic workloads, numeric contracts, generic sensors and validity-aware analysis. The shared two-host runner also owns host/phone orchestration, optical screen-to-screen timing, route validation and settings sweeps. Third-party adapters and their entry points remain outside this repository. Run `npm run benchmark:check` for offline contract and sensor checks. The optical detail strip includes both panels and the camera; full-screen digital image quality and physical cursor latency need separate coverage.


Cursor shapes and positions carry a shared revision. A new shape must immediately
publish the current position with that revision, even when the pointer has not
moved. The receiver hides a cursor when shape and position revisions differ;
waiting for the normal position heartbeat would create visible gaps during shape
changes. Static cursor screenshots alone do not verify this behavior. Use a
shape-and-position sequence with delayed source raster verification and an
independent receiver observation.
