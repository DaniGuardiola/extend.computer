> 🤖🔧 ai generated

# duet responsiveness investigation

2026-09-16 · installed mac app 3.20.3.0 · two macs, same wi-fi · keyboard/mouse sharing

## updated scope: cross-platform input and external display

user clarified future support for different operating systems and using another device as an external monitor. architecture recommendation below supersedes the original mac-only swift prototype recommendation. existing measurements cover keyboard/mouse sharing only; they do not establish duet's display-streaming latency or quality.

recommend **shared rust engine + tauri settings ui + thin native platform adapters**. electron also viable for ui; neither ui framework should carry high-frequency input or raw video frames through javascript ipc. tauri already separates its rust core and webview processes: [process model](https://tauri.app/concept/process-model/).

shared rust engine owns pairing, authenticated sessions, protocol/version negotiation, topology, input ownership, transport, congestion policy, recovery and telemetry. native adapters own os permissions, event capture/injection, display creation/capture, hardware codec integration and presentation. swift/objective-c on mac where useful; rust bindings where straightforward. language purity provides no guarantee of lower latency: platform apis, queues, copies and scheduling determine performance.

design host and viewer capabilities independently. windows hosting a virtual display viewed on mac must not require identical implementation on both ends. negotiate codec, dimensions, scaling, refresh rate, cursor mode and input features per session. keep three explicit modes: keyboard/mouse sharing; existing-display mirroring/control; desktop extension via a new virtual monitor.

external-display pipeline:

```text
host virtual monitor → gpu capture → hardware encode
                       ↓ network ↓
viewer hardware decode → native gpu presentation
viewer input ───────────────→ host injection
```

native rendering surface should sit alongside settings ui. minimize gpu-to-cpu copies, bound frame queues, discard obsolete frames where codec dependencies permit, and keep input from waiting behind video. independent logical streams alone do not remove head-of-line blocking if they share one tcp byte stream. transport choice for media needs fresh benchmarking; previous tcp evidence supports input mode only.

virtual monitor creation is a separate feasibility task from screen capture:

- mac: ScreenCaptureKit provides capture. virtual-display creation needs separate investigation; chromium's mac test utility declares `CGVirtualDisplay` interfaces reconstructed from binaries. isolate that dependency and verify compatibility before promising support. [apple capture api](https://developer.apple.com/documentation/screencapturekit), [chromium implementation](https://chromium.googlesource.com/chromium/src/+/HEAD/ui/display/mac/test/virtual_display_util_mac.mm).
- windows: documented user-mode indirect display driver model provides a virtual/indirect monitor path. driver packaging and deployment are distinct work from ordinary app packaging. [microsoft overview](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview).
- linux: ScreenCast portal exposes virtual sources when supported; available capabilities depend on compositor/backend. query support rather than promise one universal linux display path. [portal specification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).

proposed milestones: (1) mac virtual-display and hardware-video feasibility spike early; (2) shared rust session engine with mac input adapter; (3) second-os adapter to exercise real cross-platform compatibility; (4) encrypted low-latency mirror/viewer pipeline; (5) extension mode and topology/reconnect polish. prototype media separately early so an input-only architecture does not dictate an unsuitable video design.

benchmark input latency, video latency, text clarity, sustained frame pacing and battery use separately. local cursor rendering may improve perceived movement but does not eliminate delayed application response. a smooth pointer alone cannot validate display streaming quality.

## conclusion

evidence points to careful native input handling over a direct, latency-tuned lan connection. matching this experience looks technically feasible with public macos apis. no proprietary video codec needed for keyboard/mouse sharing: remote mac renders its own apps and cursor.

strongest concrete findings: direct tcp over wi-fi, interactive-video traffic classification, native event taps and event posting, explicit multi-display handoff logic, and an awdl-stutter mitigation feature. exact contribution of each remains unmeasured. no evidence found establishing predictive cursor rendering or exotic transport as the explanation.

## verified observations

| finding | evidence | interpretation / limit |
|---|---|---|
| direct lan tcp | `nettop` shows established private-address peer connection on `en0`; duet logs identify `tcp-direct` | input session avoids cloud relay in this run |
| cloud separate | distinct https connections on `utun4`; peer stays on `en0` | cloud and session paths separated; does not prove offline startup works |
| low normal rtt | 139 app telemetry samples from same session: min 6 ms, median 7 ms, max 222 ms; live kernel snapshot 6.34 ms | round-trip telemetry, not physical input-to-photon latency; app estimator undocumented |
| responsive during manual test | nearby 30-second telemetry interval rose from ~5 to 70.5 sent packets/sec; ~631 to 13,235 sent bytes/sec; reported rtt 6 ms | consistent with input-event traffic; app counters are not necessarily tcp segment counts or mouse polling rate |
| latency-sensitive traffic class | live peer connection reports `VI`; connection framework imports `NWParameters.ServiceClass.interactiveVideo` and service-class setter | corroborated runtime classification; router prioritization and latency benefit not established |
| tcp no-delay support | framework imports `NWProtocolTCP.Options.noDelay` setter | static evidence only: value and activation on this session not proven |
| native input pipeline | imports include `CGEventTapCreate`, `CGEventPost`, keyboard/mouse/scroll creation and field access; `KeyboardMouseObserver` and `KMSManager` symbols | native capture/injection architecture strongly supported; exact event fields remain to verify |
| explicit handoff state | `localDisplays`, `remoteDisplays`, `globalLocation`, nearest-display state, `skipNextOffset`; cursor hide/show/disassociation imports; start-tracking and dock-position log entries | geometry and cursor ownership receive explicit treatment; precise edge thresholds unknown |
| structured kms messages | `StartKMSSession`, `HideCursorKMSRequest`, `DuetKMSProto`, protobuf serialization diagnostic | structured session/handoff messages; exact wire schema not recovered |
| awdl mitigation exists | binary ui string describes pausing awdl to reduce wireless stutters; helper methods include taking awdl down and restoring it | feature existence verified; `awdl0` currently lacks `UP` and reports inactive. cannot establish who disabled it or benefit without controlled comparison |
| discovery | bundle declares `_duetrds._tcp`; network framework browser/publisher symbols | bonjour discovery support; current session discovery mechanism not conclusively traced |

initial `lsof` showed no peer tcp socket. `nettop` resolved this: peer uses channel-backed networking (`arch=ch`) and appears there. absence from that `lsof` snapshot was not absence of a peer connection.

## why it feels close to wired

likely causal explanation, not a completed latency attribution study:

1. **small input messages.** remote screen renders locally. no capture → video encoding → transmission → decoding pipeline needed for this mode.
2. **short route.** observed session stays on local wi-fi. normal network round-trip ~6–9 ms. half-rtt would suggest a few milliseconds one way only under symmetry; this is not a measured one-way delay.
3. **latency-sensitive scheduling.** interactive-video service class is active. no-delay setter exists; confirming its value would strengthen this part.
4. **native cursor and event handling.** native capture/posting and explicit display ownership avoid much application-level translation overhead. correct acceleration, scroll semantics, and edge coordinates matter as much as bandwidth.
5. **jitter mitigation.** duet explicitly includes awdl suppression. plausible contributor on mac wi-fi, not yet isolated experimentally.

apple describes interactive-video service class as intended for low-delay, low-loss flows; it is not proof of guaranteed priority across a particular access point. apple documents `noDelay` as disabling nagle’s algorithm. sources: [service class](https://developer.apple.com/documentation/network/nwparameters/serviceclass-swift.enum/interactivevideo), [tcp options](https://developer.apple.com/documentation/network/nwprotocoltcp/options).

## measurement limits

manual test involved crossing between screens, circles, and scrolling. counters changed around test window; this was not synchronized high-speed measurement. no input contents recorded. 30-second reporting intervals obscure individual stalls and bursts. `rttvariancems` is duet’s field name; formula unknown, so not interpreted as statistical variance.

one historical app sample reached 222 ms. a separate 20-packet icmp probe returned all packets but started at 896, 695, 492, 287, and 83 ms, then ~4.6–7.7 ms. overall average 126.75 ms. cause unknown; could involve transient network/host scheduling or probe treatment. do not substitute those icmp timings for duet input latency or omit startup outliers from benchmark results.

no packet payload capture, memory dump, credential extraction, app patching, or network setting changes used. inspection covered app metadata, bundled symbol/diagnostic strings, imported apis, duet-specific logs, socket/interface metadata, and peer ping. app’s cryptographic implementation not audited; public encryption claim is not a verification result. full disassembly did not resolve `noDelay` call arguments in this pass.

## recommended open-source implementation

for this mac↔mac use case, start with a small swift native agent using core graphics + network.framework. build own protocol; no need to reproduce duet’s wire format.

| component | proposed behavior |
|---|---|
| discovery/pairing | bonjour on lan; explicit two-device pairing; persist authenticated device identity in keychain |
| transport | authenticated encrypted connection, direct lan first; tcp with `noDelay = true`; benchmark service classes rather than assuming a winner |
| capture | event tap with minimal callback work; timestamp, enqueue, return; avoid ui thread work on hot path |
| event protocol | bounded, versioned messages: key up/down, button up/down, pointer position/motion, scroll, topology, ownership, heartbeat, release-all |
| motion | pick and test one coordinate/acceleration policy; avoid applying acceleration twice; preserve subpixel precision and map display points/pixels correctly |
| scrolling | preserve precision, units, horizontal/vertical deltas, phase and momentum where supported; validate against native scrolling |
| handoff | explicit ownership state and transfer acknowledgment; preserve entry point across different display sizes/scales; stable corner/edge policy; prevent immediate bounce-back |
| injection | native event posting; tag synthetic events to avoid feedback loops; preserve modifiers, repeat, click counts and drag ordering |
| queues | separate bulk clipboard data from input; bounded input queue; coalesce only safe adjacent motion, preserving clicks and accumulated relative displacement |
| recovery | release held keys/buttons on disconnect or expired lease; restore local cursor and capture; local escape shortcut; recover disabled taps |
| instrumentation | sequence numbers, monotonic timestamps, queue depth and delay, connection rtt, event delivery gaps, reconnect duration; no raw key-content logging |

tcp is a reasonable first baseline: observed duet session already feels excellent with tcp. consider datagrams only after tests show tcp retransmission stalls matter. unreliable motion needs sequence handling and absolute/cumulative state recovery; keys/buttons need reliable ordering and state reconciliation. changing transport alone does not fix event semantics.

keep awdl setting untouched by default. first compare ethernet versus wi-fi, then a separately authorized awdl on/off experiment. a causally proven improvement could justify an explicit optional mitigation with restoration on exit. not a requirement for first prototype.

apple exposes scroll momentum and customizable scroll events directly: [momentum phase](https://developer.apple.com/documentation/coregraphics/cgeventfield/scrollwheeleventmomentumphase), [scroll event creation](https://developer.apple.com/documentation/coregraphics/cgeventcreatescrollwheelevent).

## build vs improve existing project

benchmark these before choosing a full rewrite:

- [deskflow](https://github.com/deskflow/deskflow): established cross-platform keyboard/mouse sharing, tls enabled by default, clipboard support. good baseline for finding specific mac fidelity gaps.
- [lan mouse](https://github.com/feschber/lan-mouse): rust implementation with mac capture/emulation, dtls-encrypted traffic, and device fingerprint authorization. useful alternative architecture to compare.

neither was installed or benchmarked here. current project descriptions do not establish performance parity with duet. for mac-first polish, native swift is a reasonable focused experiment; for broad platform support, contributing measured fixes to an existing project may deliver more sooner.

## next experiments, in priority order

1. compare duet and prototype/deskflow on same devices, direction, pointer speed and wi-fi conditions. use phone high-speed video framing physical input and destination display. report median, p95/p99 and worst stall; account for camera frame resolution.
2. instrument prototype from capture through enqueue/send/receive/post. one-way comparisons require clock synchronization or explicit uncertainty; use local-stage timings plus round-trip echo otherwise.
3. test scroll momentum, diagonal motion, fast reversals, drag across edge, mixed display scaling, modifiers held through transfer, disconnect with button held, sleep/wake, and tap disable/re-enable.
4. vary tcp no-delay and service class one at a time under idle and competing lan traffic. measure tail latency and correctness, not only averages.
5. compare ethernet and wi-fi; only then isolate awdl with on/off trials. current inactive state alone cannot prove duet caused it or that disabling it helps.

success criterion: match perceived duet responsiveness while preserving input semantics and keeping worst-case stalls/recovery visible. this investigation establishes a credible architecture and several concrete tuning clues; it does not recover duet’s full protocol or prove every latency mechanism.


## 2026-09-18 onboarding evidence

[Full onboarding capture and assessment](research/duet-onboarding-2026-09-18/README.md): twelve user-provided screenshots record screen-recording approval, explicit Performance Boost helper installation with native administrator authentication, Accessibility approval, and completion. This adds direct installation-flow evidence to the earlier investigation; it does not establish helper internals.
