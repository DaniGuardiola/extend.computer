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
