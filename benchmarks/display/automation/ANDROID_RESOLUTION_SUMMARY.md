# Android resolution campaign summary

Summarize existing `android_display.py` clip folders without running devices or recomputing camera analysis:

```sh
python3 benchmarks/display/automation/summarize_android_resolution.py native/android/.local/my-campaign --show-report
```

Writes `resolution-summary/summary.json` and `resolution-summary/report.html`; `--output DIRECTORY` overrides the destination. Opening the HTML is optional. This is a within-product diagnostic of intentionally different display geometries, not a calibrated transport comparison or a cross-product ranking.

Each clip's `diagnostic.json` supplies actual target logical/raster geometry and optical results. Preparing clips, failed analysis, missing geometry, insufficient moving transitions, and nonfinite metrics are shown separately. Passed optical clips remain accepted even when repeat count is less than three. Every clip contributes exactly one value to median/min/max summaries; frame counts never weight clips.

Add `campaign-profile.json` beside each clip's diagnostic:

```json
{
  "apk_sha256": "64 lowercase hex characters",
  "mac_binary_sha256": "64 lowercase hex characters",
  "helper_cli": ["actual", "helper", "arguments"],
  "source_config": {"codec": "hevc", "fps": 60, "bitrate": 20000000},
  "route": {"kind": "native-usb", "before": "route-before.json", "after": "route-after.json"},
  "media_before": "media-before.json",
  "media_after": "media-after.json",
  "usb_before": "usb-before.json",
  "usb_after": "usb-after.json"
}
```

Receipt paths must reference existing files within that clip directory. The summarizer records their SHA-256 hashes and checks presence, not artifact semantics. Even complete provenance is labeled **documented**, never automatically **route verified**. Review route and media receipts before drawing native USB conclusions. Missing or incomplete provenance prevents aggregation into matched repeats; optical values remain visible individually.

Groups require identical product, scene, actual geometry, APK/Mac hashes, helper CLI, source config, route kind, duration, marker sides, camera orientation/focus settings, brightness policy, and connection setup. Validated `camera/summary.json` and `camera/provenance.json` are also required for matched aggregation: the full capture profile (helper APK hash, requested FPS/exposure/zoom/crop/orientation/focus ROI values), capture FPS, fixture hash, tool source/native binary hashes, and package versions must match. Host timestamps/run IDs are ignored. Diagnostic requested focus file values and hashes are included; file paths do not affect matching. Supply the same complete capture configuration for repeats; differing configurations deliberately split groups. Three repeats are informative evidence, not an automatic pass/fail threshold. Different resolutions, marker sizes, scanout positions, panel behavior, and camera row bias limit interpretation. No calibrated transport latency or product ranking is produced.

Tests:

```sh
python3 -m unittest discover -s benchmarks/display/automation -p test_android_resolution_summary.py -v
```

Only folders containing `diagnostic.json` are counted. A failure before that file is created (for example pre-capture setup failure) is absent from automatic totals; record such attempts separately in campaign notes.
