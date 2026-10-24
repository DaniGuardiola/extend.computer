# Display acceptance coverage

A completed campaign is evidence for its selected scenes and declared context,
not a claim that the entire display product matches a reference implementation.
Use matched resolution, scale, refresh target, physical displays, renderer,
network route and recording schedule. Preserve invalid attempts and unavailable
measurements. Do not infer latency or quality from delivery FPS alone.

Matched comparisons also require per-scene camera acquisition provenance: the
installed helper APK hash, camera ID, recorded dimensions, requested capture FPS
and exposure compensation profile. Missing legacy evidence is not assumed equal.
Automatic ISO and exposure telemetry remain recorded observations, not settings
that must equal across changing scenes. Acquisition diagnostics are separate from
campaigns; changing exposure compensation does not retroactively validate old
optical measurements.

Constant-delay alignment of observed grayscale cells provides an additional
diagnostic when counter readability is limited. It keeps the original tracking
gate and labels weak or competing peaks ambiguous. It does not recover missing
counter IDs, establish frame rate, or replace transition bounds or physical
input-to-photon measurements.

Optical frame cadence excludes the stationary preparation counter zero, which
the fixture displays before the measured workload starts. Its observed hold is
retained separately as `preparation_to_first_update_ms`; active positive-counter
gaps, observation gaps and latency uncertainty remain reported. Historical
reports require preserved reanalysis before comparison with this cadence rule.

| User experience | Current automated evidence | Remaining acceptance work |
|---|---|---|
| Visible smoothness | Camera counter transitions, frame gaps and tails; scroll, moving panels and motion scenes | Repeated matched runs under current conditions |
| Screen-to-screen lag | Paired physical timing markers in one camera timeline | Does not measure hardware input-to-photon latency |
| Pointer lag | `cursor-optical`: physical source reference to actual receiver cursor | Four-position synthetic movement; input method recorded; hardware input remains unmeasured |
| Pointer motion | `cursor-sweep`: projectively registered yellow cursor, movement gaps, confirmed holds and unresolved gaps | Continuous motion with other shapes, normal pointer size and hardware input |
| Pointer appearance | Digital cursor shape/visibility and coordinate fixtures | Physical shape-change timing and mixed display densities |
| Image fidelity | Exact digital fixtures, text/line/color registration and codec scores; paired optical strip | Repeated preset runs; physical panels and lighting limit optical quality comparisons |
| Settings | In-session resolution, Retina, FPS and bitrate changes; both helper acknowledgments and observed phase configuration | Repeated motion/scroll/static/cursor sweeps of all selected presets |
| Recovery | Controlled two-second source process pause with independent resume | Network outage, Wi-Fi route changes and automatic reconnect scenarios |
| Reliability | Recorded route, geometry, source timing and errors; invalid sessions fail acceptance | Long-duration sessions, sleep/wake and repeated connect/disconnect |
| Resource cost | Two-host connected-idle baseline, CPU, RSS and socket deltas | Energy and thermal measurements with suitable independent instrumentation |
| Window/display behavior | Live source/backing geometry and receiver geometry gates; digital coordinate checks | Window resize, fullscreen transitions, alternate aspect ratios and densities |
| Input interaction | `source-interaction` verifies six real source OS outcomes; `interaction-visual` registers and compares final button/text/drag/scroll states in the receiver compositor | Transient per-action receiver observation, repeated comparison campaign integration and physical input-to-photon timing remain required |

Optional scene commands:

```sh
./benchmarks/display/run campaign --product extend --scene cursor-optical --seconds 10
./benchmarks/display/run campaign --product extend --scene cursor-sweep --seconds 10
./benchmarks/display/run settings-sweep --help
```

These short diagnostic runs develop and validate individual measurements. They
cannot replace the repeated full campaign or the unimplemented acceptance work
in the table. Comparison retains selected optical cursor metrics and rejects
missing, duplicated or unmeasured requested scenes even when a report claims
completion. Zero confirmed pointer holds is only a lower bound when camera
observations contain unreadable intervals.

Digital fidelity also has an explicit `digital-quality --allow-scaled` mode for
the actual fullscreen viewport. It verifies four fiducials against the observed
viewport transform and scores unchanged receiver pixels against a declared
bilinear sRGB source reference. It is separate from exact-raster fidelity and
does not quantify detail lost solely to scaling. Default exact mode still rejects
fractional scaling; do not mix the two modes in comparisons.

`--calibrate-viewport` adds an independent synthetic calibration fixture for
internal content transforms: four fitting markers, two held-out markers,
repeated captures, bounded residuals, and a subsequent independent checker
validation on the quality fixture. Scored pixels never fit the mapping.
Calibration does not make the resampling model an exact physical reference.

The discrete optical cursor reference is drawn after synthetic pointer movement.
Its physical panel can display later than the receiver cursor, producing signed
negative reference-to-cursor deltas. Legacy four-lane captures retain their
original positive-delay acceptance and failures. The new coded fixture adds
unique eight-bit event identities and a four-bit checksum to the cursor image
and source reference. Its signed matching bounds are -125 to +500 ms; an event
cannot match a later repetition of the same position. It measures discrete
shape-and-position updates, not movement-only latency or hardware input delay.
Camera readability and independent live cross-product validation remain gates.
Fresh live acceptance has passed for one implementation after delayed source
raster verification and a cursor-revision fix. Earlier invalid readability and
cadence attempts remain preserved. Cross-product coded cursor parity still
requires matched live evidence; one accepted diagnostic does not establish it.


Coded cursor acceptance also requires delayed source-system raster verification
for each event: matching raster hashes, dimensions and hotspot, zero fixture
errors and restored pointer position. An immediate `NSCursor.set()` or hotspot
check does not establish that the requested image remains active. Missing legacy
verification cannot be inferred from a readable drawn reference badge.

`cursor-quality --allow-scaled` provides a separate declared viewport diagnostic.
It maps the known source logical dimensions to the receiver viewport and resamples
only the expected cursor reference. Source position comes from the observed Quartz
pointer coordinates, including OS rounding. This static compositor check does not
measure physical cursor lag or establish parity at other display densities.

Cursor fixtures also offer an explicit `mouse-events` input method, posting OS
mouse-move events rather than only warping the pointer. This exercises event-driven
applications without pretending to be physical hardware input. Reports retain
the selected method; campaign comparison and resume reject mixing methods.
Use `cursor-quality --input-method mouse-events` or
`campaign --cursor-input-method mouse-events` for selected cursor phases.

Coded fixture v4 displays event zero during an explicit preparation period before
starting the measured clock. Its source-system raster is verified along with the
measured events. The preparation identity is excluded only when the v4 metadata
explicitly declares event zero, first measured event one, and the preparation
period. All measured source intervals, identities and receiver loss gates remain
unchanged. Legacy v2/v3 recordings keep their original acceptance behavior.
Camera focus preparation likewise keeps the timing patches visible until the lens
locks, then replaces only the caller-owned warm-up with the measured phase.

Coded fixture v5 enlarges only the drawn physical source reference to 96×64
logical pixels at center y=164. The actual native cursor remains 48×32 at y=148,
with the same image, hotspot, source input method and transmitted payload. Sparse
six-point yellow reference landmarks retain strict four-lane isolation; a solid
expanded border would overlap adjacent lane checks. Reader selection uses the
independent source/receiver white identification marker and declared geometry.

Preparation is an explicit stream reset. The observer requires event zero twice
on each screen before accepting measured events; preexisting cursor IDs before
that reset are retained in preparation observations. Once established, identity
regression, missing measured IDs, source cadence and loss gates remain strict.
V2/v3 legacy records retain their original behavior. Mixed fixture versions or
camera acquisition profiles do not establish a matched comparison.
