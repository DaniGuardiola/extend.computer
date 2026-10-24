# Android high-speed benchmark camera

Dedicated Camera2 helper; no stock-camera UI navigation is required. It requests only camera permission and saves clips inside its own app sandbox.

Build with Python 3, JDK and the extracted official Android platform 35 and build-tools 35 archives:

```sh
python3 build.py --sdk /path/to/extracted-sdk --java-bin /path/to/jdk/bin
adb -s SERIAL install -r benchmark-camera.apk
adb -s SERIAL shell pm grant computer.extend.benchmarkcamera android.permission.CAMERA
```

The archive layout expected by this initial builder is `android-35/android.jar` and `android-15/` for build-tools. Keep the SDK, signing key and APK local.

For a bounded framing check without recording video, use `--es operation preview`
instead of `record`, with a fresh run ID and `--ei seconds 20`. This opens the
same selected high-speed camera and crop, writes a `preview.json` readiness receipt,
and closes the camera after the requested interval. It creates no video or sensor
recording. Use an ADB screen capture while the preview is live to inspect both
display planes; preview readiness does not validate optical timing or readability.

Start a fresh capture using a unique run ID:

```sh
adb -s SERIAL shell am start -S -n computer.extend.benchmarkcamera/.CaptureActivity --es operation record --es run_id UNIQUE --ei fps 240 --ei seconds 5 --ei width 1920 --ei height 1080
```

Poll `files/UNIQUE/ready.json` using `adb exec-out run-as computer.extend.benchmarkcamera cat ...` before starting the workload. Poll `result.json` for completion, then retrieve that file, `sensor.jsonl` and `video.mp4` from the same directory. Raw videos belong in ignored private capture storage.

Both landscape orientations are supported. Preview rotation and recorded orientation metadata follow the display orientation at capture setup; keep the phone stationary during a run. The recording uses 240 fps capture and 30 fps slow-motion playback. Playback fps is not capture fps.

High-speed Camera2 capture forces automatic exposure; manual shutter and ISO control are unavailable in this mode. For an explicit acquisition diagnostic, `capture_phone.py --exposure-compensation-steps N` requests a supported exposure compensation value. The default remains zero. The helper verifies the camera's range and records the EV step size, requested compensation, installed APK SHA-256, and per-frame actual compensation, ISO and exposure time. A darker request may lower ISO rather than shorten exposure: inspect the recorded sensor data before drawing conclusions. Deferred collection preserves the original capture profile. Use the same helper build and requested exposure profile for matched comparisons; do not mix these diagnostic recordings with an existing campaign.

## Setup verification

A reference phone (Pixel 10a) exposes fixed 1920×1080 at 240 fps through the public Camera2 high-speed API; five-second captures reach a measured callback cadence of about 239.7 fps. Frame numbers repeat across high-speed batches, so they must not be used alone to infer missing frames.

This verifies camera acquisition setup, not optical latency accuracy or a matched product benchmark. Sensor/video boundary alignment, visible test patches and optical calibration remain necessary.


Capture now uses `settle-and-lock-v1`: preview autofocus must report focused,
stationary lens telemetry with stable distance for 300 ms before the helper
issues an AF lock trigger. It waits another 300 ms for confirmed focus lock,
then starts recording and publishes workload readiness. Each recorded frame
must remain focused and stationary within 0.02 diopters of the locked distance;
loss of lock aborts the recording. Fixed-focus cameras use verified lens
characteristics instead. Preparation time, locked distance, actual AF mode/state
and lens state are recorded. Deferred collection independently verifies them.

Android constrained high-speed capture forces continuous-video autofocus, so
manual distance sweeping is not supported in this mode. Supported AF triggers
can lock the settled autofocus state. Focus lock is not proof that both screens
are optimally sharp: patch readability and optical validity checks still apply.
Camera helper hash and focus policy gate matched acquisition profiles; older
continuous-autofocus recordings retain their original evidence and limits.

### Region-based autofocus

`capture_phone.py --focus-regions regions.json` accepts one or two normalized
`[left, top, right, bottom]` rectangles in the 1920×1080 sensor output. This
currently requires orientation hint 0. The helper maps them into the sensor's
center crop and records requested regions. Collection checks the camera's actual
AF-region result on every recorded frame; ignored or changed regions fail validation.
The camera still chooses the lens distance, then locks it before recording.

Calibrate using the fixed high-contrast fixture on both screen planes. Compare
several frames for each candidate region, select the region with the strongest
weaker-screen edge response, and retain rejected attempts. This is a bounded
sharpness selection, not a guarantee of globally optimal focus or a codec-quality
measurement. If neither screen is readable, reposition the camera/screens.

Pass the selected JSON to `campaign --focus-regions regions.json`. The campaign
copies its contents into the run, preserves them on resume, and rejects changes
on resume. Acquisition comparisons also require matching region settings. Older
continuous-autofocus clips remain separately labeled; they are not silently
reclassified as locked-focus recordings.

### Repeatable camera focus selection

With the phone fixed and the display connection active, run:

```sh
./benchmarks/display/run focus-camera --product extend --output out/focus-check
./benchmarks/display/run campaign --product extend --focus-regions out/focus-check/selected-focus-regions.json
```

The first command records a baseline and tries left, right and both timing-patch
regions without opening the camera UI. Each candidate uses three frames, verifies
both patches and stable framing, and checks actual autofocus-region and lens-lock
telemetry. It selects the strongest weaker-screen coarse-edge score among valid
candidates. Failed candidates remain in the report. Calibration is specific to
that fixed camera/screen arrangement and does not guarantee optimal focus at every
possible lens distance. Re-run it after repositioning the rig.

The campaign keeps the calibration patches visible while focus settles, locks the
lens, then starts the measured workload. Recordings with focus loss fail. Selected
region contents are copied into the campaign and retained on resume.

### Timestamp evidence

Camera2 constrained high-speed capture can dispatch framework-derived timestamps
for requests within a batch, repeating the batch's frame number. The collector
reports this as `sensor_timestamp_evidence`; callback cadence is not independent
proof of every exposure or encoded frame time. Original callback order is retained
and non-increasing timestamps still fail acceptance. Failed telemetry now retains
the original video locally before raising, so diagnosis does not depend on the
phone remaining available. No sorting, deduplication, or timestamp repair is used.
See Android's [CameraDeviceImpl batch-result dispatch](https://android.googlesource.com/platform/frameworks/base/+/9a0ec8c4563bd5ac2404b12b27c99621be6a15cd/core/java/android/hardware/camera2/impl/CameraDeviceImpl.java).

### Reusing calibrated focus

`focus-camera` also writes `selected-focus-distance-range.json`, centered on the
selected trial's observed lens distance (±0.1 diopter). This optional experimental
guard is not included in the recommended command: separate autofocus sessions
may choose different distances, and agreement does not establish sharper pixels.
Use `campaign --focus-distance-range PATH` only when investigating reproducibility.
This is an acceptance range,
not a manual lens setting: the high-speed camera retries autofocus when a settled
lock falls outside it, up to six rejected locks or the startup deadline. The
workload starts only after an accepted lock and the collector independently
verifies the acknowledged range and every recorded focus sample. An unreachable
range fails preparation; it does not silently pick another distance. Recalibrate
when the rig moves. Distance agreement alone does not prove optimal image sharpness.

### Explicit camera magnification

`capture_phone.py --zoom 1.4` (or `campaign --camera-zoom 1.4`) requests a centered sensor crop while retaining the same video size and capture FPS. It does not enlarge the benchmark cursor or alter the product. The camera must acknowledge the requested crop on every recorded callback; unsupported or ignored requests fail. Crop and zoom are part of comparison provenance and cannot change on resume. Keep both timing patches and their preparation badges fully visible, and recalibrate autofocus regions for the new field of view. Zoom may magnify existing blur rather than recover detail, so verify readability before a full run.

Android documents `SCALER_CROP_REGION` among controls available in constrained high-speed recording: https://developer.android.com/reference/android/hardware/camera2/CameraMetadata#REQUEST_AVAILABLE_CAPABILITIES_CONSTRAINED_HIGH_SPEED_VIDEO
