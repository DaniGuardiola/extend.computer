# Two-host campaign

The shared runner drives a 20-second connected-idle baseline, 30-second warm-up,
and three repetitions of static, scroll, moving-panel, motion and recovery scenes.
Recordings now run back-to-back, with footage temporarily buffered on the phone.
Live resource, route and stage evidence is saved before advancing each scene.
Once recordings finish (about ten minutes for default settings), the terminal
announces that both Macs and the phone can be used again. Keep the phone plugged
in until transfers finish. Transfer and optical analysis then proceed without
focusing either product or sampling live performance. Processing can use the
receiver's CPU/GPU while you work; it does not change already recorded evidence.
Both host disk space and phone storage are checked before capture.

This capture schedule removes the older inter-scene transfer/analysis idle gaps.
Reports identify it as `capture-first-v1`; comparison rejects mixing it with older
interleaved runs because thermal conditions can differ. Optical framing/cadence
failures are discovered during processing; use a smoke check first after setup
changes.
Set up both Macs with `./benchmarks/display/run setup` (source: add `--source-only`).

On both Macs, `./benchmarks/display/run launch` opens the installed Extend build
with numeric hooks. Establish an extended-display connection normally, with
matching geometry and a fullscreen receiver.

For a fixed Retina benchmark profile, the source can use a build with the
`display-benchmark` feature instead of automatic sizing:

```sh
./benchmarks/display/run launch --app /path/to/extend.computer.app --connect account --display-profile 1728x1085@2
```

This requests a 1728×1085 logical display with 3456×2170 backing pixels, HEVC,
60 FPS and 20 Mbps. `account` requires exactly one available account peer.
The test connection ends after one hour; `--connection-seconds` can shorten it.
The override is a development hook, not a product settings UI.
On the receiver, add `--display-profile 1728x1085@2` to both preflight and
campaign commands. The runner rejects a mismatch in logical size, scale or
backing pixels before starting capture. Reports retain the required profile.

On the source:

```sh
./benchmarks/display/run host --bind SOURCE_LAN_IP --show-pairing
```

On the receiver, pair once through the hidden prompt, then run:

```sh
./benchmarks/display/run pair-source
./benchmarks/display/run campaign --product extend --check
./benchmarks/display/run campaign --product extend --smoke
./benchmarks/display/run campaign --product extend
```

Private IPv4 and IPv4 link-local source addresses are supported. For a direct
cable link when a VPN redirects traffic, pin only the benchmark sockets to each
host's link interface. Start the source with
`host --bind SOURCE_LINK_IP --interface SOURCE_LINK_INTERFACE`, then copy its
new certificate-pinned pairing file to the receiver using the existing private
setup path. Add this optional field to the receiver's ignored pairing JSON:

```json
"routing": {"source_ipv4": "RECEIVER_LINK_IP", "interface": "RECEIVER_LINK_INTERFACE"}
```

Use actual IPv4 addresses and interface names. The client binds its source
address before applying the macOS interface scope, disables proxy use for this
connection, and retains certificate verification and authentication. This also
applies to the camera readiness callback. It changes no system routing or VPN
settings and does not select the product's media transport. Save the product's
selected route and physical interface evidence separately; matching peer sockets
alone do not distinguish Wi-Fi from a cable.

Timing patches default to the source screen's left edge and virtual screen's
right edge. If the physical screens face the opposite way, start the source
host with `--patch-sides right,left` to keep both inner corners visible. This
uses the existing fixture placement option; the workload source stays unchanged.
Source context and fixture results retain the selected edges. Verify both patches
in the phone preview before recording, and keep placement fixed across a campaign.

For a controlled encoder diagnostic, `launch --fixed-bitrate` requests
`EXTEND_DISPLAY_FIXED_BITRATE=1` in a build with the benchmark feature. The default
keeps adaptive bitrate enabled. Launch receipts retain `fixed_bitrate`; verify the
app's fixed-bitrate startup event and the native bitrate timeline for every measured
phase. The requested bitrate ceiling and encoded payload rate remain separate from
the native encoder target. A fixed target does not require every frame to consume
that many bits.

Stage results retain `encoder_target_bitrate` from successful VideoToolbox
`AverageBitRate` readbacks, including the last snapshot before the measured phase
and subsequent updates. Missing or failed readback stays unverified. Historical
`bitrate` events were emitted before the native setter and cannot prove applied
targets; current getter evidence remains separate from those attempted changes.

Benchmark launches explicitly disable kernel UDP arrival timestamps unless
`--kernel-arrival` requests them. `--auto-kernel-arrival` instead omits the override
and records the requested policy as `auto`, without claiming active timestamps.
The options are mutually exclusive. Verify `kernel_arrival_requested`,
`kernel_arrival_active`, the automatic-policy receipt and applied/fallback packet
counters in each host's stage evidence. Unsupported or invalid timestamps retain
the declared userspace fallback.

`launch --connect PEER --display-profile WIDTHxHEIGHT@SCALE --codec h264`
requests H.264 for an explicit benchmark connection. HEVC remains the default;
the default invocation keeps compatibility with earlier prepared builds. Launch
receipts retain the requested codec; verify the active helper and native codec
events before comparing. Ordinary application sessions keep their codec policy.

For an explicit acquisition diagnostic, use
`campaign --camera-exposure-compensation-steps N`. The camera verifies its supported
range and records the requested EV step plus actual exposure and ISO. The default
remains zero. Nonzero requests prepare the visible fixture before autofocus locks.
The setting is retained on resume; changing it requires a separate campaign.
Matched comparisons still require identical requested camera profiles, and a darker
request does not guarantee a shorter shutter time or readable markers.

The smoke command runs one five-second setup scene and is never accepted as a
comparison result. The runner brings the receiving viewer forward automatically.

Use `campaign --product extend --quick` for development: all five scenes once,
with the same 30-second scenes, baseline, warm-up, camera settings and validation
as the default full run. Reports label this as a quick development campaign.
It is separate from `--smoke`, and cannot be compared as a matching full campaign.
Resume inherits the quick profile automatically. Normal commands still run three
repetitions.

For a focused development check, select a scene instead of running a whole suite:

```sh
./benchmarks/display/run campaign --product extend --scene motion --seconds 10
./benchmarks/display/run campaign --product extend --scene cursor --seconds 10
./benchmarks/display/run campaign --product extend --scene static --scene scroll
```

`--scene` is repeatable, preserves the requested order on the first repetition,
and rejects duplicates or unknown names. Targeted diagnostics default to one
repetition, ten-second scenes, a five-second connected-idle baseline and a
five-second warm-up. Durations may be 5–54 seconds. An explicit `--seconds`, even
30, creates a development diagnostic profile; without `--scene` it runs the
normal scene set at that duration. Default full and `--quick` campaigns retain
their original durations and scene sets.

Diagnostics use normal preflight, viewer focus, camera checks, route/resource
collection and saved HTML/JSON results. Reports explicitly label them
`diagnostic`, retain `scene_selection` and `measurement_scope: development-only`,
and cannot be silently compared with full or quick campaigns. They do not prove
complete display-session parity. `--scene`/`--seconds` cannot accompany `--check`,
`--smoke` or `--process`. Resume inherits the saved selection and duration; omit
both flags when resuming. For selected cursor diagnostics, use `--scene cursor`
instead of combining a selection with `--include-cursor`.

Add `--include-cursor` to full or quick campaigns for a separate pointer-motion
scene. The source fixture moves the pointer at a requested 120 Hz along a
bounded figure-eight inside the virtual display. It restores the original
pointer position on completion and normal cancellation (SIGTERM/SIGINT); forced
process termination cannot run cleanup. Default five scenes stay unchanged.
Reports retain actual movement cadence, input method, errors and restoration status, plus available
source cursor submissions and receiver layer-update cadence and inter-update
gaps. Missing hooks remain unavailable. These host-local measurements are not
cursor input-to-photon or transport latency; the camera timing counters still
measure video. Shape and visibility transitions are not exercised. Both products
must include the optional phase for comparable campaign context; resume inherits
the saved setting. A source fixture build containing this phase is required.

Processing continues through independent captured scenes if one scene fails
sensor or optical validation. The final campaign still fails; no invalid scene
becomes a measurement. `processing_failures` and each scene's `camera.log` retain
diagnostics. Transfer or tool failures can retry the same saved capture with
`--process RUN_FOLDER`. A known invalid sensor/optical capture is retained and
skipped during processing retries; explicitly recapture it with:

```sh
./benchmarks/display/run campaign --product extend --resume RUN_FOLDER --retry-scene 1:scroll
```

Use a scene name without a repetition number to retry its failed captures across
repetitions. Old evidence and its checkpoint move into `attempts/`; successful
scenes stay preserved. Resume still verifies hardware, geometry, builds, power
source and artifact integrity. It does not silently accept changed conditions.

Terminal progress shows each scene and recording, phone transfer and optical
analysis steps, with elapsed times and completion marks. Color is automatic for
terminals; `NO_COLOR=1` or redirected output uses plain text. Detailed camera
diagnostics remain in each scene's `camera.log`.

Prefer a USB data connection to the receiving Mac for faster footage transfer.
When the same phone is reachable over USB and wireless ADB, the runner selects
USB automatically. An explicit phone selection still takes precedence.

Keep the phone fixed, unlocked and charging, in landscape, showing both inner
bottom timing patches at approximately the same sensor row. Keep both Macs idle.
The source service accepts authenticated fixed operations over certificate-pinned
HTTPS. Subsequent `host` commands reuse its paired address. After an IP change,
restart with `--bind NEW_IP --show-pairing` and pair the receiver again.

The HTML report opens in the default browser when a started run finishes,
including failed runs. Use `--no-open` for unattended or headless automation.

Successful full runs print `Benchmark completed successfully.` and exit with code
0. A smoke run prints `Setup check passed.`. Failed or interrupted runs exit
nonzero and retain their report and evidence.

Continue an interrupted run with `campaign --product extend --resume RUN_FOLDER`.
Add `--check` to validate it without recording. Measured scenes are preserved
after artifact integrity and hardware/build/geometry checks. A fresh idle baseline
and warm-up precede only the missing scenes. Failed attempts are archived inside
the same folder; reports disclose the separate measurement sessions. Keep the
same camera placement and product settings. Resume inherits duration and repeats
from the saved report. Captured checkpoints are preserved; pending processing can retry without recording them again.

To retry transfer or analysis without any new recordings or source-host access:

```sh
./benchmarks/display/run campaign --process RUN_FOLDER
```

Captured checkpoints are retained even if processing fails. This command needs
the same phone only while footage remains on it; once collected locally, neither
phone nor product apps are required. If scenes were never captured, use `--resume`
to record only those missing scenes. Capture resume requires matching context and
schedule; older interleaved runs need the runner version that created them.

Results are new ignored directories under `automation/runs`, or the explicitly
selected private runner location. Each contains report.html, report.json,
manifest.json and original per-scene evidence. Allow about ten minutes of raw
capture plus transfer and analysis time, and at least 13 GiB for default runs.
Camera cell positions track a common translation of up to six pixels per frame.
All four markers must agree within three pixels; at least 95% of scene frames
must retain markers on each panel. Larger movement and hidden patches are rejected.
Per-frame coordinates and tracking coverage are retained for review.

Compare two completed folders using `compare LEFT RIGHT --output NEW_FOLDER`.

Each scene reports its route: direct LAN when both hosts' dominant flows match
their opposite LAN peer, non-LAN when neither matches, mixed when they disagree,
or unknown when evidence is missing. Non-LAN alone does not prove relay use.
Retroactively classify a stopped run with `classify-routes RUN_FOLDER`; the prior
report is archived and original measurements remain unchanged. Optional
`--evidence FILE` can confirm relay only when independently verified two-host
socket hashes match that scene's recorded dominant flows.

Measured: visible update cadence/gaps, paired optical transitions, synthetic
strip quality, product CPU/memory, whole-host CPU/GPU, OS flow counters and
available numeric frame stages. Unknown metrics stay unavailable. Recovery is a
two-second sender-process pause, not a network drop. Optical delay includes
physical screens and camera timing; it is not input latency. Optical SSIM is not
codec-only quality. Energy is unavailable. Camera sensor timestamps, exposure
and rolling-shutter skew are retained when provided; row bias and panel scanout
remain uncalibrated. Comparison requires reviewing mount/settings evidence.

The signal-alignment diagnostic also compares observed complementary grayscale
cells across the two screens. It searches a bounded constant delay using the
nearest recorded camera samples; it never reconstructs missing counter IDs.
Tracking must still cover at least 95% of the scene on both screens. A strong
model match requires correlation of at least 0.8 and a margin of at least 0.1
over peaks 33 ms or more away. Weaker or repeated-pattern results remain
`ambiguous_alignment`, with their candidate peak shown only as a diagnostic.
This model is not a per-frame latency distribution, visible FPS measurement or
input-to-photon measurement. Its near-peak score range is not a confidence
interval; camera exposure, rolling shutter and panel scanout remain uncalibrated.

A product-specific runner can explicitly supply DISPLAY_BENCH_ADAPTER and
DISPLAY_BENCH_RUNS. No private competitor adapter or binary is bundled here.
Raw camera pre/post margins are retained. Optical coverage uses the first through
last readable source counter, never the receiver; a scene visible for less than
90% of its requested duration is rejected. Receiver stalls remain in that window.

Unit tests include a synthetic complete campaign through all 15 scenes; they do
not substitute for a real connected two-host run.

## Source interaction fixture

Setup builds `automation/android-camera/tools/interaction-fixture`. Run it on the
source Mac with an active extended display, while both Macs are idle and no other
benchmark fixture is running:

```sh
./benchmarks/display/automation/android-camera/tools/interaction-fixture out/interaction-check.json auto
```

This fixed five-second operation sends real macOS events to its own focused
window: clicking a button, focusing and typing into a native text field,
select-all replacement, dragging and scrolling. It verifies actual control
outcomes, records host-local observation timestamps, releases held mouse input
on cancellation and restores the original pointer position. Restoring the prior
app's focus is recorded as a request, not proof of a completed activation.
It refuses to run without existing Accessibility access or one unmirrored
external display at least 1000 by 700 logical pixels; it never requests new
permissions. Focus or display changes stop it. Exit codes are 0 for passed,
1 for failed/cancelled, and 3 for unavailable, with evidence in the JSON file.

From the receiver, the paired source host exposes the same bounded operation:

```sh
./benchmarks/display/run source-interaction --product extend --output /path/to/new-interaction-run
```

The authenticated host runs only this fixed fixture, rejects overlapping measured
phases, quality/cursor fixtures and settings changes, and returns the native
outcomes, exit status, cleanup evidence and fixture/evidence hashes. The command
saves JSON and HTML, including failed attempts, and requests cleanup on exit.
Restart an older source host after updating the benchmark code.

These are source OS interaction checks, not receiver visual acceptance or
hardware input latency measurements. Independent observation of each result on
the receiver and integration into repeated comparison campaigns remain unfinished.

### Receiver interaction observation

```sh
./benchmarks/display/run interaction-visual --product extend --output /path/to/new-visual-interaction-run
```

Both hosts must be idle. The owned source fixture performs the six OS checks,
holds the completed state, and captures only its synthetic window. The receiver
focuses the prepared product viewer, captures its composed window, validates four
checker corners at exact 1:1 raster coordinates, then checks button, replacement
text, dragged target and scrolled content regions independently. Region luminance
SSIM and foreground-contrast overlap both must pass; blank background alone
cannot establish a matching text field. ICC conversion, screenshot hashes,
fixture identity, source outcomes, geometry and direct-LAN evidence remain in the
report. The fixed fixture appearance is independent of system theme.

The host deadline terminates the fixture even if the caller disappears. A normal
observation stop must confirm all source checks, cursor restoration and native
exit 0. Failed attempts retain JSON/HTML; registered synthetic receiver images
remain available even when a region fails. The report opens after completion
or failure; use `--no-open` for unattended runs. Full-window temporary captures
are deleted. These checks observe final compositor outcomes only, not physical
panel scanout, transient text/focus transitions, hardware input-to-photon latency
or cross-product fidelity parity. They do not replace repeated comparison scenes.

## Display settings coverage

The app's device settings offer a resolution, Retina density, frame rate and
quality setting before connecting and during an outgoing display connection.
Resolution changes retain the same virtual monitor. Settings take effect after
both native helpers acknowledge them; a failed update stays visible in the app.
Quality sets a bitrate ceiling: Lower bandwidth 8 Mbps, Balanced 20 Mbps, Detailed
40 Mbps. Actual bitrate can be lower. These settings do not promise a particular
latency or that a 60 Hz receiving panel can show 120 distinct frames per second.

The launcher accepts the same frame rates and quality presets:

```sh
./benchmarks/display/run launch --app /path/to/extend.computer.app --connect account --display-profile 1728x1085@2 --fps 60 --quality detail
```

For development, run the motion and scroll scenes separately at all three quality
presets, keeping `1728x1085@2` and 60 FPS fixed. Then check 30 and 120 FPS with
Balanced quality. Check Retina off (`1728x1085@1`) and a lower logical resolution
(`1280x800@2`) separately. Keep each settings combination in its own run: relaunch
the source with that configuration, wait for the connection, then run:

```sh
./benchmarks/display/run campaign --product extend --display-profile 1728x1085@2 --scene motion --scene scroll --seconds 10
```

Change the campaign profile argument to match any changed resolution/density.
Each run saves observed native configuration and live display inventory, not just
launcher arguments. A live settings change invalidates cross-mode aggregation;
do not combine results from different modes into one parity claim. Use full
repeated campaigns only for selected final configurations. Reference comparisons
must match geometry, frame rate, hardware, route and capture context; different
quality controls are evaluated by actual bandwidth and measured quality, rather
than assuming equivalent preset names.

### Unattended in-session mode changes

For repeatable mode checks, explicitly enable the development settings bridge
on the source. Both commands use the same local request file:

```sh
./benchmarks/display/run launch --app /path/to/extend.computer.app --connect account --display-profile 1728x1085@2 --settings-file out/display-benchmark-settings.json
./benchmarks/display/run host --display-settings-file out/display-benchmark-settings.json
```

On the receiver, apply a mode between captures:

```sh
./benchmarks/display/run display-settings --display-profile 1728x1085@2 --fps 60 --quality responsive
./benchmarks/display/run campaign --product extend --display-profile 1728x1085@2 --scene motion --seconds 10
```

Repeat with `--quality balanced` and `--quality detail`, then the selected frame
rates and geometry profiles. `display-settings` returns only after both helpers
acknowledge the actual configuration. The certificate-pinned, authenticated host
accepts this fixed operation only when explicitly enabled and refuses settings
changes during a measured phase. The app applies the regular validated settings
path; this is not a transport or authentication bypass. Production builds do not
watch the settings file. The native app still offers its normal settings UI.

### Renderer diagnostics

The native viewer now draws a newly selected frame directly by default. Numeric
launch context records the rendering mode. `launch --notification-render` selects
the previous notification renderer only for development comparisons;
`--paced-render` selects the timed experimental renderer. These are mutually
exclusive with `--explicit-render`. Packet pacing on LAN remains a separate
experiment (`--paced-transport`); it is not enabled by default. Preserve these
flags in run context rather than mixing renderer samples.

### Automated settings sweep

Start the source host with the explicitly enabled settings bridge described above.
Keep both Macs idle and the phone fixed. From the receiver, use:

```sh
./benchmarks/display/run settings-sweep --display-profile 1728x1085@2 --mode responsive --mode balanced --mode detail --scene motion --scene scroll --seconds 10 --output /path/to/new-settings-run
```

Other mode names are `30fps`, `120fps`, `native` (same logical size, scale 1),
and `compact` (1280×800 at scale 2). To check all seven settings conditions,
pass every mode explicitly, using the actual source display profile:

```sh
./benchmarks/display/run settings-sweep --display-profile 1728x1084@2 --mode responsive --mode balanced --mode detail --mode 30fps --mode 120fps --mode native --mode compact --scene motion --scene scroll --scene cursor-sweep --seconds 10 --output /path/to/new-all-settings-run
```

This changes one setting axis at a time; it does not test every combination.
Requested 120 FPS is a capture ceiling, not a guarantee of 120 visible updates
on a 60 Hz receiver. Compare measured results against the panel refresh rate.

Include `--scene static` for optical detail,
`--scene motion` or `--scene scroll` for video cadence, and
`--scene cursor-optical` for discrete software cursor state-reference timing.
The coded cursor fixture changes both shape and position at 4 Hz. Each event
has an eight-bit identity plus CRC, and the physical source reference displays
the same badge. Two consecutive readable samples confirm an appearance;
identities must increase without wrapping. Scenes must be at most 60 seconds.
Reported deltas are signed: the receiver can display before the source's
physical reference panel. This is not negative network latency or hardware
input-to-photon timing. `cursor-sweep` remains the separate movement-only
cadence observation. Older four-lane captures retain their original matcher.
Digital text/image quality remains a separate `digital-quality` measurement;
camera detail is affected by both panels and room lighting.

Digital quality defaults to exact 1:1 compositor presentation. For a declared
aspect-fit viewport diagnostic, use:

```sh
./benchmarks/display/run digital-quality --product extend --allow-scaled --output /path/to/new-digital-quality-run
```

This mode derives scale from observed window and source dimensions, resamples
only the reference with declared bilinear interpolation, and requires four
independent checker corners to agree. It does not measure detail lost solely
through scaling or physical panel fidelity. Renderers with internal content
insets or additional transforms can fail this model. Such failures remain
invalid; `registration_failure` retains individual corner offsets. Do not
fit scale to the scored image or treat failed captures as comparable quality.

For renderers with an internal content transform, add `--calibrate-viewport`.
This first captures a separate black-backed six-marker calibration fixture
twice. Four markers determine an axis-aligned mapping; two held-out markers
must agree within 0.5 backing pixels, and repeated mappings must remain within
0.5 pixels across the target. Scale is bounded to the observed viewport model.
Calibration images, hashes, mapping, and residuals are saved separately. The
different quality fixture then independently validates all four checker
corners before scoring. Quality pixels never determine calibration geometry.
This remains compositor quality with a declared bilinear source reference,
not original-pixel or physical-panel fidelity.
Each mode has its own acknowledged
configuration, geometry validation, baseline, warm-up, scenes, route evidence
and HTML/JSON report. Each campaign is processed before advancing to the next
mode; default full-campaign capture scheduling remains unchanged. This command
runs development diagnostics, not a full comparison campaign or an exhaustive
combination of every setting.

The summary lists the requested frame rate and bitrate ceiling alongside actual
encoded video payload bandwidth, visible update rate, and median/p95 video and
cursor delay when available. Every measured scene must contain the requested
native resolution, density, frame rate, bitrate ceiling, codec and cursor mode;
missing evidence or a mismatch fails the sweep. Preset names are not performance
rankings. Use geometry and frame-rate modes separately to isolate each axis,
then repeated matched campaigns for the configurations selected for comparison.

The sweep stops at the first failed acknowledgment or campaign. It always tries
to restore Balanced / 60 FPS at the supplied base profile, including after failure.
This is an explicit fixed-profile benchmark setting, not restoration of arbitrary
user preferences. Restoration failures appear in `sweep.json` and the summary
HTML page and make the command fail. The summary opens automatically; use
`--no-open` for unattended runs. Existing output folders are never reused.
A failed child's captured footage remains resumable through normal campaign
commands. Do not infer a stable preset ranking from a single sample.

After correcting an analyzer bug, explicitly retry analysis of collected footage:

```sh
./benchmarks/display/run campaign --process /path/to/mode-run --reanalyze
```

This archives the previous failure and analyzer provenance, verifies saved artifact
integrity, and reruns all validity gates on the same footage. It does not record
another scene or require the original live display configuration. Invalid footage
still fails; use `--resume --retry-scene` when a new recording is necessary.

To analyze an already completed campaign with updated tools, preserve the original
report and footage by choosing a new output folder:

```sh
./benchmarks/display/run campaign --process /path/to/completed-run \
  --reanalysis-output /path/to/new-analysis
```

This verifies video hashes, copies the campaign (using APFS clones on macOS),
archives the previous accepted analysis, and reruns the normal validity gates.
The new report records its original campaign and report hash. No apps or phone
are needed, and the original campaign is never modified. The output must not
already exist. This produces a new analysis of the same measurement session,
not another independent benchmark repetition.

### Network priority experiment

`launch --encoder-slots 3` compares a three-frame hardware encoder limit with
the production two-frame limit. Use the same signed build for both controls,
and compare dropped captures, physical update cadence, and tail latency. A wider
pipeline can improve throughput while increasing queued-frame delay. This remains
an opt-in development experiment; `--encoder-slots 2` restores the default.

`launch --video-qos` explicitly marks the IPv4 media socket with AF41 (DSCP 34),
including multiplexed feedback and cursor data. The default remains best effort.
This is a development comparison, not a promised latency improvement: networks
can ignore or reject markings. The launcher records the selected DSCP in session
context; compare only fresh, otherwise matched runs. IPv6 is not supported by
this experiment. The mapping follows [RFC 8837](https://www.rfc-editor.org/rfc/rfc8837.html).

`launch --responsive-service` separately tests Darwin's responsive multimedia
socket service (NET_SERVICE_TYPE_RV), whose public SDK definition explicitly
includes screen sharing. It affects host/Wi-Fi queue classification rather than
just setting an IPv4 DSCP field. The option is macOS-only, opt-in, verified with
socket readback, and recorded in launch context. Compare it independently of
`--video-qos`; neither experiment is enabled by default.

Transport observation also reports per-RTP-stream UDP payload bytes and the
number/bytes of packets carrying the clear RTP padding flag. This helps explain
traffic during unchanged-screen periods. A flagged packet may contain both
media and padding; these counters are not pure-padding bytes or proof of a
retransmission. Padding length and encrypted payload are never inspected.
Unflagged RTX packets may also be bandwidth probes, so payload type alone
cannot classify a packet as useful video. Measure actual socket traffic beside
encoded payload bandwidth when evaluating idle cost and settings.

Instrumented Extend phase snapshots also retain `transport_observation.windows`
from each host's app log. These are complete reporting windows that overlap the
phase, mapped using that host's numeric-event wall/monotonic timestamps. Boundary
windows include activity outside the phase and the final window may not yet have
been emitted when the snapshot is taken. Do not sum them as exact phase packet
totals or interpret their timestamps as cross-host latency. Missing telemetry is
reported as unavailable, not zero traffic. This preserves RTP grouping, padding
flags and assembly-delay evidence after the original live app log is gone.
Each window retains the latest four assembled groups and the four largest
observed holds of at least 25 ms after their last matching original RTP read.
Samples include the assembler's expanded sequence range and continuity flag.
This bounded selection preserves startup holds that later fast frames would
otherwise overwrite; it is not a complete frame history or a loss-rate sample.
Original RTP observations do not identify the encrypted original sequence in
RTX packets, so a large hold alone does not prove a complete frame was already
available or identify where a missing packet was delayed.

`lan_stream_probe.py` isolates burst delivery with bounded synthetic frame trains,
outside product or optical phases. On the source:

```sh
python3 benchmarks/display/campaign/lan_stream_probe.py sender --bind SOURCE_IP --peer RECEIVER_IP --protocol udp --seconds 10 --hz 60 --chunks 8
```

Use its printed port on the receiver:

```sh
python3 benchmarks/display/campaign/lan_stream_probe.py receiver --bind RECEIVER_IP --peer SOURCE_IP --port PORT --protocol udp --seconds 10 --hz 60 --chunks 8 --output out/frame-train.json
```

Repeat sequentially with `--protocol tcp` on both sides. Keep rate, chunks and
duration identical. The default socket class is responsive video; both sides
also accept `--service best-effort` or `--service interactive-video` for explicit
diagnostics. Each frame consists of the selected number of fixed 1200-byte
synthetic packets. The peer is restricted to the configured LAN IP; no product
payloads or user content are collected. Receiver evidence preserves raw packet
IDs and local arrival timestamps. On macOS, UDP also records kernel receive
timestamps and kernel-to-userspace delay; these are OS observations, not radio
or screen timestamps. TCP receive reads can contain multiple records at the
same timestamp. Do not interpret either result as product latency or infer
that one transport wins from a single sequential sample.

For an explicit UDP receive-path activity diagnostic, add `--reverse-hz 60` on
the receiver. It sends bounded eight-byte synthetic messages to the sender's
advertised UDP port during the train; default zero leaves this off. Reports
record the actual count because scheduling may not sustain the requested rate.
This is a network experiment, not a proposed product keepalive policy.

### Video reordering deadline experiment

`launch --reordering-ms 100` explicitly tests earlier video loss recovery. The
default remains 500 ms; valid development values are 20..1000 ms. Apply the same
launch option to both apps, then start the normal matched workload. The option
is recorded in launch context and the transport emits its actual deadline as
`config_video_reordering_ms` in the numeric trace. An older app that ignores
the option cannot prove that the requested experiment took effect.

Shorter deadlines trade tolerance for late packets against earlier keyframe
requests. Observe optical frame gaps, recovery time, PLI/keyframe traffic and
quality together. An earlier recovery request does not prove earlier repaired
presentation. Compare otherwise identical runs, preserve failures, and relaunch
with the default before normal benchmark campaigns.

### Verify automatic display fit

Use `launch --connect account --display-profile 1728x1085@2 --fit-display` to exercise automatic fullscreen sizing. With this flag the profile is an initial hint, not the measured result. The receiver opens its native viewer, waits for confirmed fullscreen content geometry, and negotiates that geometry before source capture starts. A running session's Fit display control uses the same native viewport query. Fixed-size launch remains the default for repeatable comparisons.

Inspect live source display inventory after connection, then supply those actual logical/backing dimensions to the campaign's `--display-profile` gate. Do not assume that panel dimensions or safe-area estimates equal the native fullscreen content size. `scripts/macos/test-display-viewport.py HELPER` opens and closes one owned synthetic viewer to verify the geometry control response; run it only while the desktop is idle.

### Live socket arrival diagnostics

On macOS, `launch --observe-transport` also enables `SO_TIMESTAMP` on the actual
video UDP socket. Numeric five-second transport windows retain kernel ingress
packet/group gaps alongside userspace-read gaps, kernel-to-userspace queue
intervals and assembly holds. The timestamped read uses the same nonblocking
socket and bounded receive loop; diagnostic-disabled builds retain `recv_from`.
The observer records RTP header timing only, never encrypted payload contents.

`kernel_receive` explicitly counts missing timestamp and invalid clock samples.
Darwin timestamps use the host realtime clock at microsecond precision; queue
intervals subtract only receiver-local realtime observations. Kernel gaps can
be affected by realtime clock adjustments. They are socket ingress timestamps,
not Wi-Fi radio arrival, source capture time, whole-frame readiness or physical
screen latency. Do not subtract timestamps across hosts. A burst already present
at kernel ingress differs from a long interval spent queued before userspace or
inside frame assembly; use matched scene windows rather than startup maxima.
Unsupported timestamp setup reports unavailable and preserves normal reception.
The receiver fixture verifies payload/source preservation, IPv4/IPv6 timestamps,
nonblocking reads and encrypted H.264/HEVC idle-resume behavior.

### Low-latency encoder experiment

`launch --low-latency-encoder` explicitly selects VideoToolbox low-latency rate control on the source. A benchmark launch without that option explicitly requests normal mode, preserving controlled comparisons when the ordinary app chooses a USB policy automatically. `launch --auto-low-latency` omits the override and records the requested policy as `auto`, with no actual mode claim in the launch receipt. The two options are mutually exclusive. Native events separately retain requested and selected mode, creation status, automatic-policy selection and fallback; successful frames and driver evidence establish completed setup. Hardware selection, no frame reordering, bounded submission queue and keyframe recovery remain enabled. The H.264 experiment uses High profile because that selection requires it; HEVC retains Main profile.

Compare the same binary with and without this flag at identical geometry, workload, bitrate ceiling and frame rate. Measure optical latency, cadence, encoded bandwidth, resource usage, and digital quality separately. A successful hardware round trip only proves support, not an improvement. Apple's documentation describes the selection: https://developer.apple.com/documentation/videotoolbox/kvtvideoencoderspecification_enablelowlatencyratecontrol

### NV12 transfer-attachment control

The default NV12 capture handoff interprets the Rec.709 transfer attachment as sRGB when ScreenCaptureKit was explicitly configured for sRGB output. Exact registered fixtures establish that this avoids an extra transfer conversion; no CPU pixel conversion is added. `launch --native-capture-transfer` retains the original attachment for a development control. Launch context records `nv12_srgb_input`; numeric events record original capture and corrected encoder-input attachments. RGB and unknown/already-sRGB capture formats remain unchanged. Compare digital quality and motion/resource behavior at the same geometry and settings; one development sample is not a parity or reliability result.

### Cursor shape, hotspot and visibility

With an established fullscreen Retina connection, run:

```sh
./benchmarks/display/run cursor-quality --product extend --output /path/to/new-cursor-results
```

The fixed source fixture covers its selected external display with black, then exercises two synthetic cursor shapes with different hotspots at left/center/right positions, hides the cursor, and moves it outside the streamed display. The runner checks the real receiver composition at matching logical geometry on a 2x receiver, not merely transmitted state or helper counters. Cursor size stays independent of source video density. Both custom raster fidelity and cursor absence are acceptance gates. It rejects stale/duplicate colored cursor pixels, wrong hotspots, changed geometry or product processes, and routes not proven direct LAN. The source acknowledges actual OS visibility before each state. Pointer position and balanced hide/unhide cleanup must succeed.

Currently supports source video scale 1 or 2, with receiver scale 2 and matching fullscreen logical dimensions. It converts source backing coordinates into receiver composition coordinates and keeps the cursor raster at its logical size. This is static cursor behavior, not physical cursor latency or cadence. The existing `--scene cursor` provides separate motion cadence evidence; neither substitutes for input-to-photon measurement. Full-window temporary screenshots are deleted; only synthetic cursor crops and generated cursor references are retained. Results include JSON/HTML, requested and observed state, geometry, application identity, route evidence and restoration status.

Source host exposes only authenticated fixed `/cursor/start`, `/cursor/state`, and `/cursor/stop` operations. Callers cannot provide commands, executable paths or arbitrary cursor images. A fixture expires after 45 seconds and restores its own pointer/hide changes on normal cancellation. Forced termination cannot guarantee cleanup and is a failed result.

### Optical cursor transitions

For physical cursor observations inside the camera-visible timing patches:

```sh
./benchmarks/display/run campaign --product extend --scene cursor-optical --seconds 10 --display-profile 1728x1084@2
```

This optional diagnostic moves a synthetic yellow pointer between four fixed
positions at four steps per second. After each successful synthetic pointer move, the
source draws its matching state reference on the physical source panel. The
camera reads the reference and receiver cursor in one timeline, independently of
the video counters. The report separates state-reference-to-visible-pointer delay
from video latency and host-local cursor event cadence. This excludes physical
mouse hardware and input dispatch latency; it is not an input-to-photon result.

Both patches must resolve the yellow pointer and timing fiducials. Two consecutive
readable camera frames confirm a state. The analyzer rejects ambiguous color,
insufficient readable coverage, matched transition coverage below 90%, invalid source cadence or
receiver gaps over 500 ms. State labels repeat once per second; a 500 ms matching
bound prevents pairing a stale cursor with the next cycle. Over-bound observations
fail the phase instead of disappearing from the latency distribution. Camera
exposure, quantization and rolling-shutter bias remain declared limits.

Default and quick scene sets are unchanged. The pointer is restored on normal
completion or cancellation; a forced termination cannot guarantee cleanup.

### Continuous optical pointer diagnostic

Use `--scene cursor-sweep --seconds 10` to record a 120-Hz software pointer sweep
inside the camera-visible timing patch. A yellow source reference and the actual
receiver cursor traverse the same horizontal path. The report measures visible
movement above its spatial uncertainty, plus holds of at least 100 ms while the
source reference continues moving. This is separate from the four-position
`cursor-optical` transition-latency test and host-local `cursor` telemetry.

A stalled receiver remains a measured bad result. Missing/ambiguous yellow
components and source-reference stalls invalidate evidence; unreadable gaps
remain disclosed. Zero confirmed holds is a lower bound, not proof of no stalls;
change gaps crossing unreadable frames remain unresolved. Change counts are not
cursor FPS, and this software-driven
fixture does not measure physical mouse input latency. Both products must use
the same scene, camera geometry, resolution and sampling configuration.

To add both physical cursor phases to a repeated full campaign:

```sh
./benchmarks/display/run campaign --product extend --include-cursor --include-optical-cursor
```

This runs eight scenes per repetition: the five normal video scenes, the host-hook
pointer phase, optical jump latency and continuous optical pointer motion. Both
products must use the same phase set. `--quick` uses one repetition; the full
command uses three. Smoke checks reject optional phases. Resume inherits the
saved optical-phase selection and rejects changes. These additions cover pointer
observations, not the remaining hardware input, sleep/wake or network-recovery
work listed in `../COVERAGE.md`.

### Independent LAN diagnostic

`lan_probe.py` runs a bounded, synthetic simultaneous TCP/UDP echo diagnostic
outside camera recordings. It records client-clock RTT distributions, actual
send cadence and unanswered messages. It accepts a single configured LAN peer,
echoes only 16-byte messages, and the source server exits after its fixed deadline.
It does not capture product traffic or measure video latency. Never run it during
an optical benchmark: active traffic can change Wi-Fi power and queue behavior.

On the source, choose an explicit LAN address and receiver peer:

```sh
python3 benchmarks/display/campaign/lan_probe.py server --bind SOURCE_IP --peer RECEIVER_IP --seconds 30
```

Use the printed ephemeral port on the receiver:

```sh
python3 benchmarks/display/campaign/lan_probe.py client --peer SOURCE_IP --port PORT --seconds 10 --output out/lan-rtt.json
```

Both endpoints can select the same macOS-only `--service responsive-video` or
`--service interactive-video` for a diagnostic; default is best effort. These
options do not change the product's transport or system-wide network settings.

Registration and stability images use exact frame requests with zero generator
tolerance. Each PNG has a `.timing.json` sidecar recording requested/actual
playback timestamps and the one-sample error bound. Optical counter analysis
still reads the complete decoded video stream. Comparisons require matching,
complete source hashes for the camera/native/metric pipeline and verified exact
frame selection; legacy or mixed analysis pipelines cannot establish parity.
Keep original footage and analysis provenance when upgrading the analyzer.


For final interaction outcomes in a slightly scaled fullscreen viewport:

```sh
./benchmarks/display/run interaction-visual --product extend --allow-scaled --output /path/to/new-scaled-interaction-run
```

This first captures a separate calibration target twice, verifies held-out
markers, then runs the owned interaction fixture and independently verifies its
four checker corners. Only the expected source reference is resampled; receiver
pixels remain unchanged. Changed viewer dimensions, display geometry or missing
foreground fail acceptance. Default mode retains exact 1:1 checks. This records
final visible button, text, drag and scroll outcomes, not transient action latency
or physical input-to-photon timing. Scaled-mode live validation remains required.

Cursor checks can explicitly calibrate viewer chrome with independent repeated
markers using `cursor-quality --product extend --allow-scaled --calibrate-viewport --output RUN_DIRECTORY`.
This mode uses source-only bilinear references and
retains calibration metadata. Default exact-geometry checks remain unchanged.
The bounded destination marker search is 32 pixels; repeated mapping, held-out
marker residuals and the independent interaction markers still gate acceptance.
Interaction fixture markers are inset 40 logical points to avoid protected OS
recording indicators. Old reports retain their declared marker geometry.

Pointer input can be tested through OS mouse-move events as well as pointer
warps. Add `--input-method mouse-events` to `cursor-quality`, or
`--cursor-input-method mouse-events` to campaigns containing cursor scenes.
These are synthetic OS events, not hardware input-to-photon measurements.
Source acknowledgment records the method; comparison and resume reject mixing
it with warp-only captures. Existing reports without a method retain their
legacy warp-only meaning. Posting requires already-authorized OS event access;
the fixture does not request or alter permissions.

For optical cursor scenes the external timing window becomes key after activation
settles. Its tracking area reapplies the selected cursor during AppKit cursor
updates and mouse moves. The fixture checks the actual source raster on the next
tick; mismatches still abort capture. `movement_calls`, `movement_hz` and
`movement_gaps_ms` describe either input method. Legacy `warp_*` aliases are
emitted only for warp mode. Event counts and key-window state aid diagnosis;
they do not establish hardware-input latency.

The current coded optical fixture is `coded-four-lane-v3`. Its 48x32 badge is
centered at logical y=148, with reference origin y=132, leaving separation from
the timing cells ending at y=128. The patch canvas is 168 points high; fiducial
geometry is unchanged. Reader placement is selected from validated source
metadata. Legacy v2 recordings keep y=140 decoding. Coverage, CRC, cadence and
matching gates are unchanged; different fixture hashes prevent mixed comparisons.

### Repeatable camera focus selection

With the phone fixed and the display connection active, run:

```sh
./benchmarks/display/run focus-camera --product extend --output out/focus-check
./benchmarks/display/run campaign --product extend --focus-regions out/focus-check/selected-focus-regions.json
```

The first command records a baseline and tries left, right and both timing-patch
regions without opening the camera UI. Default autofocus competes alongside the
custom regions; follow the printed command. If default autofocus wins, no selected
region file is needed. Each candidate uses three frames, verifies
both patches and stable framing, and checks actual autofocus-region and lens-lock
telemetry. It selects the strongest weaker-screen coarse-edge score among valid
candidates. Failed candidates remain in the report. Calibration is specific to
that fixed camera/screen arrangement and does not guarantee optimal focus at every
possible lens distance. Re-run it after repositioning the rig.

The campaign keeps the calibration patches visible while focus settles, locks the
lens, then starts the measured workload. Recordings with focus loss fail. Selected
region contents are copied into the campaign and retained on resume.

An optional experimental guard can require the selected lens-distance interval:

```sh
./benchmarks/display/run campaign --product extend \
  --focus-regions out/focus-check/selected-focus-regions.json \
  --focus-distance-range out/focus-check/selected-focus-distance-range.json
```

The camera retries automatic focusing before the measured workload starts, then
requires the locked distance to remain inside that interval. Both inputs are
copied into the campaign and preserved on resume; comparisons reject different
requested ranges. This does not manually position the lens or guarantee that
both screen planes are maximally sharp. Failed preparation remains a failure.
Separate autofocus sessions can choose different distances; the guard is not
included in the recommended calibration command. It is a reproducibility
diagnostic, not an image-sharpness acceptance test.

### Tracing overhead checks

Launch both hosts with `launch --context-only` to retain startup compile flags
and periodic selected-route evidence without native per-frame stage timing or
transport timestamp instrumentation. Use the same signed build, display
geometry, workload and camera profile as a normal traced launch.
`--context-only` cannot be combined with `--observe-marker` or
`--observe-transport`. Reports declare stage timing disabled; missing native
stage measurements remain unavailable, never zero. Optical observations and OS
process/socket counters are still collected. Periodic route logging remains, so
this is a reduced-instrumentation check rather than proof of zero observer cost.

Do not change display settings during this check: without native configuration
events, active helper arguments describe startup configuration only.


### Sustained-load observation (deferred)

Long sessions and sleep/wake testing remain pending; short optical results do
not establish sustained-session parity. The separate runner below is prepared
for a later session, but has not yet been validated live. It does not record
the phone or change the standard camera campaign.

```sh
python3 benchmarks/display/campaign/soak.py --product extend \
  --display-profile 1728x1085@2 --seconds 300 \
  --output /absolute/path/to/new-soak-result
```

Use the benchmark Python environment and existing private product-adapter
configuration when comparing another product. Both hosts need the updated
source host code; motion phases alone permit 60–600 seconds. Other scenarios
retain their existing limit. Keep both hosts idle for the entire observation.

The runner saves source evidence, receiver samples, 30-second resource
windows, stage data when available, and route/build/geometry checks. It fails
if the product restarts, the source geometry changes, or the observed dominant
route cannot be verified as direct LAN. On cancellation it stops only its own
source fixture. A failure remains in the report; no successful retry overwrites
it. This observes sustained synthetic load and resource drift, not optical
smoothness, physical input latency, network interruption, or sleep/wake.
