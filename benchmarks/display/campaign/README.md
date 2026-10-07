# Two-host campaign

The shared runner drives a 20-second connected-idle baseline, 30-second warm-up,
and three repetitions of static, scroll, moving-panel, motion and recovery scenes.
Set up both Macs with `./benchmarks/display/run setup` (source: add `--source-only`).

On both Macs, `./benchmarks/display/run launch` opens the installed Extend build
with numeric hooks. Establish an extended-display connection normally, with
matching geometry and a fullscreen receiver.

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

The smoke command runs one five-second setup scene and is never accepted as a
comparison result. The runner brings the receiving viewer forward automatically.

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

Results are new ignored directories under `automation/runs`, or the explicitly
selected private runner location. Each contains report.html, report.json,
manifest.json and original per-scene evidence. Allow about ten minutes of raw
capture plus transfer and analysis time, and at least 13 GiB for default runs.
Camera cell positions track a common translation of up to six pixels per frame.
All four markers must agree within three pixels; at least 95% of scene frames
must retain markers on each panel. Larger movement and hidden patches are rejected.
Per-frame coordinates and tracking coverage are retained for review.

Compare two completed folders using `compare LEFT RIGHT --output NEW_FOLDER`.

Measured: visible update cadence/gaps, paired optical transitions, synthetic
strip quality, product CPU/memory, whole-host CPU/GPU, OS flow counters and
available numeric frame stages. Unknown metrics stay unavailable. Recovery is a
two-second sender-process pause, not a network drop. Optical delay includes
physical screens and camera timing; it is not input latency. Optical SSIM is not
codec-only quality. Energy is unavailable. Camera sensor timestamps, exposure
and rolling-shutter skew are retained when provided; row bias and panel scanout
remain uncalibrated. Comparison requires reviewing mount/settings evidence.

A product-specific runner can explicitly supply DISPLAY_BENCH_ADAPTER and
DISPLAY_BENCH_RUNS. No private competitor adapter or binary is bundled here.
Raw camera pre/post margins are retained. Optical coverage uses the first through
last readable source counter, never the receiver; a scene visible for less than
90% of its requested duration is rejected. Receiver stalls remain in that window.

Unit tests include a synthetic complete campaign through all 15 scenes; they do
not substitute for a real connected two-host run.
