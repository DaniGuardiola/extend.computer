# Android high-speed benchmark camera

Dedicated Camera2 helper; no stock-camera UI navigation is required. It requests only camera permission and saves clips inside its own app sandbox.

Build with Python 3, JDK and the extracted official Android platform 35 and build-tools 35 archives:

```sh
python3 build.py --sdk /path/to/extracted-sdk --java-bin /path/to/jdk/bin
adb -s SERIAL install -r benchmark-camera.apk
adb -s SERIAL shell pm grant computer.extend.benchmarkcamera android.permission.CAMERA
```

The archive layout expected by this initial builder is `android-35/android.jar` and `android-15/` for build-tools. Keep the SDK, signing key and APK local.

Start a fresh capture using a unique run ID:

```sh
adb -s SERIAL shell am start -S -n computer.extend.benchmarkcamera/.CaptureActivity --es operation record --es run_id UNIQUE --ei fps 240 --ei seconds 5 --ei width 1920 --ei height 1080
```

Poll `files/UNIQUE/ready.json` using `adb exec-out run-as computer.extend.benchmarkcamera cat ...` before starting the workload. Poll `result.json` for completion, then retrieve that file, `sensor.jsonl` and `video.mp4` from the same directory. Raw videos belong in ignored private capture storage.

Both landscape orientations are supported. Preview rotation and recorded orientation metadata follow the display orientation at capture setup; keep the phone stationary during a run. The recording uses 240 fps capture and 30 fps slow-motion playback. Playback fps is not capture fps.

## Setup verification, 2026-10-07

Pixel 10a exposes fixed 1920×1080 at 240 fps through the public Camera2 high-speed API. Two automated five-second captures completed. The second had 1,136 sensor callbacks, measured callback cadence 239.688 fps, and an upright preview after the orientation fix. Frame numbers repeat across high-speed batches, so they must not be used alone to infer missing frames.

This verifies camera acquisition setup, not optical latency accuracy or a matched product benchmark. Sensor/video boundary alignment, visible test patches and optical calibration remain necessary.
