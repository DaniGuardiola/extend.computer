# Display benchmark core

This directory owns shared workloads, measurement contracts, sensors, validity
rules, analysis, Camera2 helper and portable optical setup/capture tools. It runs
independently of the private research repository. Version-specific competitor
binaries and instrumentation, product adapters and private campaign orchestration
stay in that repository. The shared runner accepts an explicitly selected local
product adapter; it does not import private tools by default.

For a complete two-host optical campaign (Python 3.12+, macOS, Xcode Command Line Tools), see [campaign instructions](campaign/README.md). For individual camera checks:

```sh
./benchmarks/display/run setup --source-only # Source Mac
./benchmarks/display/run setup               # Receiver Mac, with Android phone
./benchmarks/display/run check               # Six-second camera readiness check
./benchmarks/display/run source              # Source: after display connection exists
./benchmarks/display/run record --product extend # Receiver: 30-second recording
```

See [camera setup and limits](automation/CAMERA.md). Generated host configuration,
phone signing keys, dependencies and raw recordings are ignored by Git. No precise
physical-latency claim is made before optical calibration. The optical command
labels an operator-established connection; it does not configure product settings
or verify matching route/codec/geometry.

## Commands

Python 3.10+; core analysis and sensors use the standard library.

```sh
python3 -m benchmarks.display.cli analyze /path/to/normalized-run
python3 -m benchmarks.display.cli compare /path/to/run-a /path/to/run-b
python3 -m unittest discover -s benchmarks/display/tests -v
python3 benchmarks/display/serve_controlled.py --output /private/tmp/new-phases.jsonl
python3 benchmarks/display/sensors/collect.py --pid 123 --port 24800 --role source --duration 60 --output /private/tmp/new-resources.jsonl
```

Each normalized run has `manifest.json` and `events.jsonl`. Output is immutable:
new directories only, hashed artifacts, no silently appended runs. The private
runner converts sensors into the public format before invoking this analyzer.

`core.py` is the executable v1 contract. Manifests identify both hosts (opaque
hardware/OS configuration labels), exact binary hash and version, adapter version,
workload version/hash, renderer, network scenario, idle status and geometry.
Profiles include logical/physical sizes, scale, codec, capture FPS and bitrate.
Clock domains are host-local monotonic nanoseconds. Frame IDs are opaque and only
joined within a clock domain. Adapter records contain numeric observations;
unknown fields and capabilities are rejected. Recording an advertised capability
without samples does not produce a zero-valued result.

Declare required measured scenes in `workload.scenes`; a missing scene invalidates
the campaign rather than accepting a partial run. One phase record per scene and clock defines the measured window and observed
geometry. Frames and resource samples are assigned by timestamp in that clock.
Warm-up/settle samples stay outside these windows. CPU samples must use CPU-time
deltas over known intervals, not `ps`'s longer averaging window. Declare the exact
process group in `scope` so app-only and app-plus-helper totals cannot silently
match. Preserve sample duration/coverage in the adapter's evidence; medians of
samples are not time-weighted CPU averages.

Comparison rejects different hosts, renderer, workload/hash, geometry, route,
requested FPS or bitrate. Codec differences remain visible context rather than a
hard rejection. Metrics must share definition, unit and scope: layer submission
cannot be substituted for GPU completion. Runs may still contain diagnostic data
with no comparable rows. Invalid phases produce reasons and no performance metrics:
hidden/resized/cancelled workload, route changes, disconnects, wrong duration,
non-idle hosts or low animated source cadence. Both input runs must be independently
valid. Static source cadence is intentionally not evaluated.

## Measurement boundaries

| Metric | Definition | Current support |
| --- | --- | --- |
| Delivery/cadence | Unique frame IDs and inter-frame gaps at a named stage | Analyzer implemented |
| CPU/memory/TCP | PID resource time and exact selected connection counters | macOS generic collector; private normalization required |
| Encoded bitrate | Encoded payload bytes per measured second | Analyzer implemented |
| Wire bitrate/RTT | Connection bytes and sampled transport estimate | Contract; adapter observations required |
| Decode/render latency | Same-frame, same-host monotonic stage difference | Analyzer implemented |
| Physical latency | Camera-observed source change to matching receiver change | Contract; camera calibration/decoder pending |
| Image quality | Registered synthetic image PSNR/SSIM | Contract; image acquisition/registration pending |
| GPU load/energy | Device counters with declared sampling scope | Not yet in v1 contract |
| Recovery | Controlled fault to resumed valid delivery | Phase invalidation exists; dedicated recovery protocol pending |

Extend's current optional `EXTEND_DISPLAY_METRICS` hook emits capture, encode,
receive, decode and GPU-completion observations. Those hooks are useful, but do
not yet provide a stable cross-host frame ID, verified clock mapping, physical
scanout or all resource metrics. The private Extend adapter handles the current
wire format; the public normalized schema is independent of that format.

## Suite policy

`suite.json` describes the v2 campaign: warm-up, settle, measured scenes,
three repetitions and counterbalanced order. It is a protocol specification,
not a claim that the current private runner launches both hosts automatically.
Before accepting a campaign, verify all requested capabilities, exact target
screen/window placement, idle hosts, source cadence and instrumentation overhead.
Keep failed attempts as failed evidence. Report per-run values and spread across
repeats; never mix old CPU measurements with a new build's FPS results.

## Camera preparation

Use a stable phone filming source and receiver patches at the same sensor row.
Record the original high-speed file, not a re-encoded slow-motion export. Verify
unique captured frames, frame timestamps, exposure, orientation, dropped frames
and actual capture rate. Record uncertainty, including rolling shutter and screen
scanout. A nominal 240 FPS mode alone does not prove 4.17 ms measurement accuracy.
Use a synthetic optical sequence with a known frame ID; calibration precedes any
latency claims. The generic ADB/Camera2 helper lives here; product-specific instrumentation remains private.

## Artifacts

Numeric events and synthetic fixture metadata can be retained by default. Raw
camera video/screenshots, raw app logs and host configuration stay in ignored
private run storage. Never collect personal display content as a quality fixture.
Do not store credentials, keys, clipboard text, account/device identities or input
contents in normalized telemetry. Explicit operator-authored build/hardware labels
must be non-secret. The hashing inventory verifies integrity, not privacy.

## Workload control v2

`serve_controlled.py` binds IPv4 loopback only. Open `/workload.html` on the source
virtual display. `/status` reports whether the page polls for work; POST `/start`
with `{run_id: UUID, duration_s: integer}` queues one run; POST `/stop` with `{}`
cancels it. The page retrieves `/command` and acknowledges phases to `/metrics`.
Completion requires all four measured phase ends. The private workload controller
waits for readiness and completion with a deadline. Placement and geometry checks
still belong to the runner. v2 uses elapsed time for static phases, fixing the old
accumulated sleep-duration problem; delayed static phases still fail validity.
The v1 fixture remains byte-identical. No authentication is supplied for this
loopback test server: do not expose it on LAN or use it as an app control endpoint.
