# Physical display input acceptance

Use the native observer's actual delivered AppKit events, not transport `Applied` acknowledgements, to decide whether tablet input reached the intended source window. The offline validator never opens a device, injects input or changes a display. Run the observer and gestures explicitly through the existing tools; keep each receipt interval limited to one gesture.

## Receipt contract

For each gesture, retain:

- The exact source capture descriptor for the current configuration. Both the capture helper's `CaptureTarget` (`configuration_id`, `display_id`, `bounds`, `canvas`, `content`, `raster`) and scoped injector's `DisplayInputTarget` (`session`, `epoch`, `revision`, `display_id`, `bounds`, `canvas`, `content`) are accepted. Do not substitute the main display or reuse a descriptor from before a resize/reconnect.
- Native observer snapshots immediately before injection and after delivery/release has settled. The same target and main-guard windows must remain in the same geometry. Retain the full snapshot, including counters and event sequences. Incomplete retained history fails validation even if counters look correct.
- The Android observer injection `result.json` produced by `ui.py --capture-dir`. This is required for HOME and blocked-input attempts and recommended for every gesture. Retain before/after screenshots and the command's observed pixel coordinates. Injection acceptance alone cannot establish remote delivery.
- The initial native `displayInputState` JSON when available. `--input-state` checks that the grant uses this capture revision and exact geometry; `--binding` checks an independently retained expected session/epoch/revision. A scoped source descriptor also supplies the expected binding automatically. Retain the final native state separately for lifecycle tests.
- The APK SHA256, Mac build identifier, actual capture mode, transport route, observed Android display metrics and fitted video rectangle. These are campaign provenance, not inferred by this validator.

The observer must target a distinct extended display and include a main guard. Before each gesture, take a fresh baseline and leave the Mac untouched. Guard move events are reported but only down/up/drag/scroll changes fail the guard check: incidental physical pointer movement must not be mistaken for a remote click. Every target action must land in the chosen visible observer zone. Keep both fingers and drag endpoints inside that zone and the fitted source content, away from tablet controls.

```sh
python3 scripts/android/validate_display_input.py \
  --before run/drag-before.json --after run/drag-after.json \
  --descriptor run/source-input-target.json \
  --input-state run/initial-grant.json \
  --kind hold-drag --zone 4 --min-drag-distance 60 \
  --injection run/drag/result.json \
  --viewport 0 0 2560 1600
```

`--viewport` is the separately observed *fitted video rectangle* in Android pixels, not blindly the `wm size` or foreground window bounds. The result reports the source zone center, negotiated-canvas normalized center and mapped Android pixel center. Retina backing scale does not multiply source logical coordinates again. The mapping is:

```text
sourceNormalized = (zoneGlobalCenter - sourceBoundsOrigin) / sourceBoundsSize
canvasNormalized = (contentOrigin + sourceNormalized * contentSize) / canvasSize
androidPixel = fittedVideoOrigin + canvasNormalized * fittedVideoSize
```

As an example, a historical fixture's display had bounds `(1728,0,1920,1200)` and observer content `(1728,30,1920,1170)`. Its zone centers normalized to source display coordinates were tap `(0.1729167,0.3154167)`, double tap `(0.5,0.3154167)`, right click `(0.8270833,0.3154167)`, drag `(0.1729167,0.74625)` and scroll `(0.5,0.74625)`. Those would map to approximately `(443,505)`, `(1280,505)`, `(2117,505)`, `(443,1194)` and `(1280,1194)` only for a verified full `2560×1600` fitted viewport with full-content canvas. Derive fresh values for each run; this historical geometry is not acceptance evidence.

## Gesture and lifecycle checks

| Interval | Required native window evidence | Additional receipt |
| --- | --- | --- |
| `tap`, zone 1 | One balanced left down/up pair, click count 1 | Actual Android tap injection |
| `double-tap`, zone 2 | Two balanced left pairs, click counts 1 then 2 | Double-tap injection |
| `hold-drag`, zone 4 | One left pair and actual drag displacement above explicit logical-point threshold | Long hold and move injection trace |
| `two-finger-tap`, zone 3 | One balanced right pair, click count 1 | Two-finger injection trace |
| `two-finger-scroll`, zone 5 | Nonzero native scroll delta, no button or drag action | Both-contact movement trace |
| `cancel-drag`, zone 4 | Held drag movement followed by actual native release | Android ACTION_CANCEL trace; final native state |
| `home-drag`, zone 4 | Held drag movement followed by actual native release | `--injection`, `--package`: successful HOME and different foreground package; final suspended/disabled grant state |
| Touch Off attempt, `blocked` | Zero target down/up/drag/scroll changes and no main-guard action | Accepted Android tap attempt required; screenshot/UI switch Off and final native state |
| Configuration change | Release old hold; no stale actions; new geometry receives a new correctly mapped tap | Separate old/new descriptor and grant receipts; revision change, old binding retired, new rendered configuration granted |

The validator cannot prove Touch Off, foreground loss or stale-revision rejection from zero observer events alone. Preserve the corresponding UI/native state transition. After HOME or a configuration change, restore the display normally, confirm a fresh grant and run a separate successful tap interval. That recovery confirms the input path is usable again; it must not reuse the previous grant receipt. Configuration transitions require fresh observer geometry when the window/display changes. HOME may accept a final `display-changed` snapshot only when its retained target/window geometry and event interval still match exactly; other invalid-target endings fail.

Outputs describe native delivery and guard isolation. Event timestamps are not optical latency. The historical regression fixture and synthetic corruption tests only validate the offline tool; they do not establish a physical pass for the current app.

```sh
python3 scripts/android/test_validate_display_input.py
```
