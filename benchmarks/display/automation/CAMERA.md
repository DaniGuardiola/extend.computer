# Portable optical capture

Use `./benchmarks/display/run` from the Extend repository. Requires macOS,
Python 3.12+ and Xcode Command Line Tools. On the source run `setup --source-only`;
on the receiver run `setup`, then `check`. Setup creates isolated dependencies and
installs a camera-only Android helper. It does not replace system Python.

Enable Android Wireless debugging. If unpaired, run `pair IP:PAIRING_PORT` and
enter the six-digit code interactively; then run `setup --phone IP:DEBUGGING_PORT`.
Pairing and debugging ports differ. Phone identity is remembered privately;
duplicate ADB transports for the same phone are handled automatically.

Keep phone unlocked, charging, steady and landscape with both timing patches in
view. Establish the display connection with the source supplying the desktop.
Source runs `source` (three minutes); receiver runs `record --product extend`
(30 seconds). `source --list-screens` and `source --screen NAME` handle more than
one additional display. `doctor` checks dependencies and phone without recording.

Results live in ignored `automation/runs/` unless an explicit output is supplied.
They include original video, sensor timestamps, hashes, coverage and decoded frame
IDs. Failed runs are retained and exit nonzero. Capture validates sensor cadence
and counter readability, not calibrated physical latency or complete visual
quality. Full static/scroll/motion campaigns and optical scanout calibration remain
separate work. No account login, peer selection or settings UI is automated.

Third-party builds and product adapters are supplied from a separate repository;
no third-party executable or instrumentation is part of this public suite.

Camera orientation is acquired after the preview attaches and the landscape activity
window and display rotation remain stable for 250 ms. Capture receipts retain the
sensor orientation, display rotation, window bounds and playback hint. Rotation or
window-size changes invalidate capture. Raw camera buffers remain unchanged;
90°/270° playback transforms are still unsupported by the optical cell decoder.
Changing Android user rotation lock alone does not override the helper's
`sensorLandscape` activity policy. Inspect a fresh preview and both timing patches
before a campaign; historical captures without this provenance remain unverified
for orientation acquisition.

For a fixed reverse-landscape camera rig, pass `--camera-orientation
reverse-landscape` to `capture_phone.py`, `camera_run.py` or `android_display.py`.
This requests the Android activity orientation and waits for actual display
rotation 3; it does not override the computed recorder hint or modify raw pixels.
The default remains `sensor-landscape`. Collection/analysis cannot change the
saved acquisition request. Keep the same request throughout compared captures.

`android_display.py` forwards `--focus-regions FILE` and
`--focus-distance-range FILE`, validates their JSON before capture, and retains
the calibration paths, bytes' SHA-256 hashes and requested values in the receipt.
Focus rectangles use **sensor-output normalized coordinates**, including when
playback orientation is 180°. For a rectangle `[x0,y0,x1,y1]` selected from an
upright 180° registration image, supply `[1-x1,1-y1,1-x0,1-y0]`. The helper does
not invert this existing sensor-coordinate API; it maps coordinates through the
centered video crop and records the coordinate basis and sensor rectangles.
0° and 180° acquisition support calibrated AF regions; 90°/270° remain rejected.
Display static timing patches before autofocus preparation so it focuses on the
actual screen planes. Start the measured scene only after camera readiness;
preparation time must not shorten the declared eight-second scene.

Pass `android_display.py --focus-preparation` to show the same static timing
patches on the selected target/reference before camera autofocus. The capture
process owns this preparation child, waits up to ten seconds for its `ready`
line, and terminates/reaps it before launching the measured scene after camera
readiness. Cleanup also runs on camera failure. `preparation-command.json`,
`camera/preparation.log` and `camera/preparation.json` retain the argv and lifecycle;
termination is explicitly intentional, not a completed measured fixture. The
measured scene retains its declared duration and normal optical acceptance gates.
Camera recording may include preparation counter zero; analysis excludes it.
