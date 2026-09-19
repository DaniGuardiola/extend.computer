> 🤖🔧 ai generated

# Cursor prototype

Cursor-only mode remains available. For the newly implemented clicks, scrolling, and keyboard mode, see [full input prototype](input-prototype.md). The cursor-only historical results below do not describe the full-input permission scope.

## Current finding

Controlled sender-only, receiver-only, both-off, and restored trials identify AWDL activity on the two Macs as the source of the dominant periodic network freezes in this setup. The encrypted dummy-cursor path went from p95 88.38 ms / maximum 101.185 ms / 29 waits above 40 ms per 15 seconds with both interfaces restored to p95 8.704 ms / maximum 12.799 ms / zero such waits with both paused. Restoring interfaces brought the freezes back. This establishes a major contributor here, not a universal explanation of every Duet latency advantage. Native visual smoothness with both paused still needs a final human check.

Both interfaces were restored and temporary servers removed. A development privileged helper is now installed on both Macs. Session integration defaults on for recognized Wi-Fi peer routes after cursor consent, with `--awdl` opt-out; it skips Ethernet/VPN/unknown routes and falls back to ordinary networking if the helper is unavailable. The pipe-bound broker upgrade is installed on both Macs; integrated tests, opt-out, and sender-crash cleanup pass. See [session results](session-low-jitter.md). AirDrop and Continuity can be affected during an active lease. See the helper guide for recovery behavior and remaining production work.

The Rust session can now request a separate, temporary cursor capability after authenticating. Diagnostic automatic permission never grants cursor access. The receiver requires a new `allow-cursor` confirmation for each connection. A grant lasts 30 seconds and cannot be renewed within that connection; revocation is checked before every movement.

The Swift helper now uses an active session event tap for edge handoff. The sender learns the receiver’s logical main-display size after cursor consent, enters through the configured left or right edge, and sends normalized remote positions only while controlling that display. A vertical offset defines the shared edge segment. The old listen-only capture mode remains a diagnostic. The receiver maps these onto its main display and posts mouse-moved events with a tagged private event source. It never synthesizes buttons, keys, scroll, or drags. Synthetic extend.computer events are excluded from capture. The sender hides its cursor while controlling the peer and restores it on return or shutdown; remote button and scroll events are suppressed during this cursor-only milestone. Keyboard input stays local. Live entry and return work; a measured post-return freeze was fixed and the user confirmed it is gone. The final integrated test delivered 1,212 updates; the user confirmed correct hiding, reappearance, and smooth return.

The sender keeps only the latest captured position and sends within a receiver-enforced window of four positions. A probe acknowledges all preceding positions after four updates or 16 ms; the sender takes a fresh capture sample after that wait. A 4 ms pacing delay caps offered update rate. This bounds queued work while avoiding a round trip for every movement. It is still a TCP prototype, not the final high-rate transport. An acknowledgment confirms posting to macOS, not display presentation or measured visual latency.

## Build

```
cargo build --locked --bins --examples
sh scripts/build-cursor-helper.sh
```

The build produces universal `target/extend.computer Cursor.app` (arm64 and x86_64), signed ad hoc with a stable bundle identifier and registered with Launch Services. Explicit architectures avoid accidentally shipping Intel-only output when the build shell runs under Rosetta. It explicitly targets macOS 13 or later; development toolchain defaults on the second Mac unexpectedly selected macOS 28. Rebuilds can require renewed permission because these are development signatures.

## Permissions

Run the helper with `request-listen` on the sending Mac and `request-post` on the receiving Mac, or add the bundle to the relevant System Settings list. Edge capture requires Input Monitoring and Accessibility on the sender; injection requires permission to post events, normally granted through Accessibility. This is not a claim that every mouse-only capture technique requires Input Monitoring. No screen-recording permission is requested.

On the local development Mac, direct UI inspection found Duet enabled under Accessibility and absent from Input Monitoring. extend.computer Cursor was enabled under Input Monitoring. The two lists represent different permissions. Duet's precise capture authorization mechanism has not been established by this observation.

## Run

Receiver:

```
extend-computer serve --bind 0.0.0.0:48177 --pair --cursor-helper "$PWD/target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor"
```

Sender:

```
extend-computer cursor RECEIVER:48177 --edge left --offset-y 0 --helper "$PWD/target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor"
```

Enter the pairing code privately, approve the diagnostic session, then approve the separate cursor request on the receiver. `--peer FINGERPRINT` uses a previously remembered identity instead of a pairing code; it still requires fresh cursor consent. Ctrl-C stops either endpoint. Sender stops after 29 seconds, leaving margin before the receiver's 30-second limit.

`scripts/two_mac_cursor.py` automates this explicitly authorized test using ephemeral identities and one-session consent. It accepts SSH host/key/pinned-known-hosts, peer hostname, remote checkout, local driver, and local helper paths. It does not retain the pairing code and cleans up the listener. No persistent permission to control a device is saved.

## Scope and validation

- 28 Rust tests pass on both Macs, including receiver geometry, cursor authorization, revocation, and bounded-window behavior. Pure Swift layout tests cover unequal sizes, offsets, edge gaps, return mapping, hysteresis, and cancellation.
- Final local Clippy passed for all targets with warnings denied; Python harness passed syntax compilation.
- Native Swift builds succeeded on both Macs. Local helper now contains both architectures; explicit arm64 execution passed. Remote helper is already native arm64.
- A repeat live desktop-session test delivered 1,093 encrypted cursor updates acknowledged by the native receiver. The user confirmed visible movement, but reported lag/jumpiness. The user subsequently confirmed movement stopped; process/listener checks found none running. Two exited launchd registrations were removed, and helper shutdown now allows a grace period for launcher cleanup. No end-to-end latency measurement has been made.
- Main-display edge switching is implemented; live handoff and the return fix are verified. No keyboard, buttons, scroll, virtual display, or video implementation yet.
- Protocol is experimental and unaudited.

Native API references: [Apple event taps](https://developer.apple.com/documentation/coregraphics/cgevent/tapcreate(tap:place:options:eventsofinterest:callback:userinfo:)), [Apple mouse events](https://developer.apple.com/documentation/coregraphics/cgevent/init(mouseeventsource:mousetype:mousecursorposition:mousebutton:)).

## Development permission diagnosis

The receiver showed Accessibility enabled, but a separate PostEvent record was denied. Clearing only extend.computer's PostEvent denial with `tccutil reset PostEvent computer.extend.prototype.cursor` and registering its app with Launch Services resulted in a fresh desktop-session check reporting event posting allowed, without another prompt. Do not equate an event-posting preflight failure with the Accessibility toggle being off.

SSH-launched processes were attributed to `sshd-keygen-wrapper` by TCC. The cursor harness now uses `scripts/desktop-cursor-helper.py` to launch the already approved app as a temporary job in the logged-in GUI session. Private FIFOs carry cursor coordinates and acknowledgments. The job and files are removed when the test ends; no permission is granted to SSH. This is development orchestration, not the eventual app lifecycle.

## Bounded streaming benchmark

`cargo run --locked --example cursor_transport_bench` compares 80 positions through an encrypted loopback relay that adds 8 ms to each response. On this development Mac, per-position acknowledgment took 862.4 ms; a four-position window took 235.6 ms (3.66x throughput). This is a controlled transport-only result, not measured Wi-Fi latency or proof of visual smoothness. The improved path still needs a live subjective comparison.

## Improved live run

The four-position-window build delivered 1,887 encrypted cursor updates in the next approximately 30-second Wi-Fi test. The harness passed; post-test checks found no receiver cursor process, port-48177 listener, or temporary cursor-session launchd job. User reported slightly better smoothness, but brief freezes roughly every second or faster; Duet remains much more fluid. Counts across the two runs are not a controlled throughput comparison because the physical movement pattern was not fixed.

## Periodic stall investigation

A sustained 1,000-probe run (8 ms sleep between probes) reproduced approximately 90–100 ms response stalls every 0.52 seconds with no native cursor activity. Median RTT was 7.947 ms, p95 11.713 ms, maximum 101.959 ms. This establishes that freezes can occur in the diagnostic path independently of input capture and injection; it does not yet isolate Wi-Fi from OS scheduling, TCP, or application processing.

A separate test of macOS NET_SERVICE_TYPE_RV did not eliminate stalls: median 7.733 ms, p95 96.047 ms, maximum 204.919 ms. Conditions were not controlled enough to attribute worsening to this socket option. The experimental code was removed.

The cursor harness now accepts `--trace-dir DIRECTORY`. It saves sender.csv and receiver.csv with timing-only events: capture arrival in Rust, selected sample ID/age, send start/end, acknowledgment start/end, and native-injection request/completion. Rows contain no coordinates or keyboard contents. Each process uses its own monotonic clock: match update ordinals and compare durations, not absolute timestamps across machines. Native completion means event posted, not displayed. Traces are buffered in memory (50,000-row cap) and written on teardown to avoid per-event disk I/O. Fresh trace paths are required.

## Measured continuous-motion run

User reported regular visible pauses during the measured run. 1,997 cursor updates reached the native receiver. Sender recorded 2,940 capture arrivals, with median gap 8.06 ms and p95 9.09 ms; there were also 23 capture gaps over 40 ms (maximum 1,074.72 ms), so capture was not perfectly continuous.

The main repeated stalls appeared in acknowledgment waits: 49 exceeded 40 ms, median interval between stalls 527.61 ms. Typical long waits were 80–100 ms, with maximum 217.3 ms. Capture arrivals continued during 47 of those 49 waits (median 11 arrivals per stall). Sending itself took at most 0.74 ms per frame. Native posting request/completion took median 0.49 ms, p95 3.39 ms, maximum 24.40 ms.

This localizes the dominant periodic freeze to the transport/acknowledgment path, not synchronous event posting, and independently agrees with the diagnostic probe-only reproduction. It does not yet distinguish wireless scheduling/interference, TCP behavior, or server scheduling. No one-way latency is inferred from unsynchronized clocks, and event posting completion does not prove when pixels changed.

Post-run checks found no remaining cursor processes, listening socket, or temporary cursor-session jobs. Timing CSVs are stored in the thread's work/cursor-trace-01 directory; graph is in outputs/extend-computer-cursor-timing.png.

## Independent TCP/UDP check

A 15-second concurrent raw echo comparison reproduced stalls without extend.computer framing, encryption, discovery, capture, or injection. TCP: 727 samples, median 7.54 ms, p95 11.63 ms, max 119.97 ms, 29 samples above 40 ms. UDP: 1,606 replies, median 7.23 ms, p95 81.93 ms, max 123.35 ms, 202 samples above 40 ms; no unanswered datagrams. Different sending patterns mean sample counts and percentiles should not be directly equated. Both paths exhibiting stalls rules out extend.computer's cursor code and TCP alone as sufficient explanations.

Both Macs' AWDL interfaces were active during investigation; initial Duet inspection had found the local interface inactive. This is correlation only. Apple DTS documents that peer-to-peer Wi-Fi can introduce substantial latency in infrastructure traffic too: https://developer.apple.com/forums/thread/751839 . A brief AWDL A/B test with automatic restoration has been prepared but is awaiting authorization. No system interface has been changed.

## Fully synthetic regression test

`examples/cursor_dummy.rs` and `scripts/two_mac_cursor.py --synthetic` now generate cursor positions and deliver them through the real pairing/encrypted-session/flow-control path to a dummy sink. No native input APIs run. The first successful 15-second trial delivered 1,745 updates; 436 acknowledgment waits had median 8.50 ms, p95 89.50 ms, max 102.19 ms, with 28 waits above 40 ms. This reproduces the human-observed periodic freezes without requiring physical mouse motion. Initial startup timeouts cleared after the user approved the new executable in LuLu.

The sender-only AWDL A/B comparison did not remove stalls. Raw TCP had 28 waits above 40 ms with sender AWDL down (max 99.90 ms), versus 28 after restoration (max 105.69 ms). UDP had 186 delayed replies above 40 ms with sender AWDL down, versus 192 after restoration. The watcher verified the local interface remained down through the test and later returned up. Receiver-side AWDL trial remains pending a user-run admin step.

## Combined AWDL intervention and restored control

| Path / metric | Both AWDL down | Both restored |
|---|---:|---:|
| Raw TCP max RTT | 13.17 ms | 108.72 ms |
| Raw TCP waits >40 ms | 0 / 950 | 58 / 569 |
| Raw UDP max RTT | 11.70 ms | 111.45 ms |
| Raw UDP waits >40 ms | 0 / 1,592 | 402 / 1,574 |
| Encrypted dummy cursor p95 acknowledgment | 8.704 ms | 88.380 ms |
| Encrypted dummy cursor max acknowledgment | 12.799 ms | 101.185 ms |
| Encrypted dummy cursor waits >40 ms | 0 / 546 | 29 / 428 |
| Synthetic cursor updates delivered | 2,185 | 1,716 |

Raw echo and encrypted cursor trials each lasted 15 seconds, ran sequentially during the same temporary intervention, and repeated after restoration. The watcher observed both interfaces down after each intervention trial and both up before controls. It did not continuously sample interface state or synchronize the hosts' clocks. Single-side pauses left regular stalls; first-hop ICMP probes to the router also showed ~94 ms spikes independently from both Macs.

The result agrees with the earlier locally observed Duet feature that pauses AWDL and restores it afterward. Apple DTS also describes peer-to-peer Wi-Fi affecting infrastructure Wi-Fi latency: [Apple networking discussion](https://developer.apple.com/forums/thread/751839). No reverse-engineered proprietary code was copied.

The measurement loop runs without human mouse movement. Session-managed low-jitter mode is now implemented; its current validation status is described above. Earlier sections record the investigation in chronological order and may describe experiments that were pending at that time.

## Edge layout milestone

`--edge left` (default) places the receiver to the left; `--edge right` places it to the right. `--offset-y 0` aligns tops. Positive values place the receiver lower, negative values higher, in logical display points. Dimensions are exchanged only after cursor consent. This is desktop control, not a virtual extended display.

Control-Option-Escape stops capture. Capture also exits on input-pipe EOF, two seconds without an acknowledged network heartbeat, layout changes, or its 30-second limit. Local EOF and heartbeat watchdog tests passed. Cursor disassociation is not used. Hide/show calls are balanced on return and shutdown; a separate native probe also confirmed cursor visibility recovery on forced process death.

Both universal native builds pass. The updated encrypted synthetic two-Mac regression delivered 2,125 updates, with ACK median 7.063 ms, p95 11.257 ms, maximum 27.36 ms, and zero waits above 40 ms. Both AWDL leases cleaned up. These are protocol timings, not visual smoothness measurements.

Receiver startup now reports 1728×1117 logical points; sender reports 1512×982. After rebuilding, the receiver’s current Accessibility grant was valid but a separate PostEvent denial remained. Resetting only extend.computer’s PostEvent record and registering the app restored successful GUI-session startup without another toggle.

## Return hesitation diagnosis and fix

The first live edge trial delivered 1,024 native updates. The user reported good entry, visible local cursor, and sticky return. Event-tap timings alone showed the next event within 0.2–9.3 ms and no attempts blocked by the display gap. A position probe then reproduced six returns with approximately 230 ms of unchanged cursor position despite positive movement deltas: the cursor-warp suppression window was the cause.

A small C compatibility boundary disables the legacy per-process warp suppression interval. Swift does not expose this deprecated public API. The fixed live run delivered 1,224 updates with zero frozen samples across six sampled returns; the user confirmed sticky return is gone. This is separate from network ACK delays.

Background hiding uses dynamically resolved private WindowServer connection configuration, followed by public balanced hide/show calls. If the compatibility symbols or configuration are unavailable, the cursor remains visible and control still works. This is an explicit macOS compatibility limitation, not an App Store-ready implementation. A half-second native probe verified hiding, normal restoration, and SIGKILL restoration on the development Mac. The final integrated visual test delivered 1,212 updates; the user confirmed hiding, reappearance, and smooth return. Both AWDL interfaces were up afterward, with zero leases and no pending restoration.

`EXTEND_COMPUTER_EDGE_DIAGNOSTICS=1` enables bounded post-return position diagnostics (up to 256 samples); it is off by default. These optional diagnostics contain cursor coordinates. Existing transport timing CSVs still contain no coordinates.
