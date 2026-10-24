#!/usr/bin/env python3
"""Generic local Mac-to-Android optical diagnostic using the standard workload.

Product-specific connection setup belongs in external adapters. --established
measures an already connected display; otherwise start Extend's USB dev bridge.
"""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import signal
import subprocess
import sys
import time

from bootstrap import build_swift
from brightness import maximum

ROOT = Path(__file__).resolve().parents[3]


def focus_options(regions_path=None, distance_path=None):
    from capture_phone import validate_focus_regions, validate_focus_distance_range
    argv, provenance = [], {}
    for name, path, validate in [('focus-regions',regions_path,validate_focus_regions),
                                 ('focus-distance-range',distance_path,validate_focus_distance_range)]:
        if path is None:continue
        path=Path(path).resolve()
        raw=path.read_bytes()
        value=validate(json.loads(raw))
        argv += ['--'+name,str(path)]
        provenance[name]={'path':str(path),'sha256':hashlib.sha256(raw).hexdigest(),'value':value}
    return argv, provenance


def inventory(tool):
    output = subprocess.check_output([str(tool), "--context"], text=True, timeout=10)
    return json.loads(output)


def validate_scale(scale, established):
    if isinstance(scale, bool) or scale not in (1, 2):
        raise ValueError("Display scale must be 1 or 2")
    if scale == 2 and not established:
        raise ValueError("Scale 2 requires --established; the development bridge supports raster geometry only")


def target(screens, width, height, scale=1):
    validate_scale(scale, True)
    if any(isinstance(value, bool) or not isinstance(value, int) or value <= 0 for value in (width, height)):
        raise ValueError("Positive logical display dimensions are required")
    candidates = [s for s in screens if not s["builtin"] and not s["mirrored"]]
    if len(candidates) != 1:
        raise ValueError("Exactly one unmirrored extended display is required")
    screen = candidates[0]
    expected = {"width": width, "height": height, "pixel_width": width * scale, "pixel_height": height * scale, "scale": scale}
    if any(screen.get(key) != value for key, value in expected.items()):
        raise ValueError("Connected display does not match the required backing and logical geometry")
    if not isinstance(screen.get("refresh_hz"), (int, float)) or not math.isfinite(screen["refresh_hz"]) or abs(screen["refresh_hz"] - 60) > .5:
        raise ValueError("Connected display must use the matched 60 Hz profile")
    if len([s for s in screens if s["builtin"]]) != 1:
        raise ValueError("A visible built-in Mac source reference is required")
    return screen


def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tablet", required=True)
    parser.add_argument("--camera", required=True)
    parser.add_argument("--camera-orientation", choices=("sensor-landscape", "reverse-landscape"), default="sensor-landscape")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--focus-regions", type=Path)
    parser.add_argument("--focus-preparation", action="store_true", help="Show static timing patches before camera autofocus")
    parser.add_argument("--focus-distance-range", type=Path)
    parser.add_argument("--established", action="store_true")
    parser.add_argument("--product", default="extend-android-usb")
    parser.add_argument("--scene", choices=("motion", "scroll", "panel", "cursor-optical", "cursor-sweep"), default="motion")
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=800)
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1, help="Logical-to-backing scale; 2 requires --established")
    parser.add_argument("--codec", choices=("hevc", "h264"), default="hevc")
    parser.add_argument("--patch-sides", choices=("left,right", "right,left"), default="right,left")
    parser.add_argument("--show-report", action="store_true")
    args = parser.parse_args()
    if args.tablet == args.camera or not 5 <= args.seconds <= 54:
        parser.error("Use separate tablet/camera serials and 5..54 second scenes")
    try:
        validate_scale(args.scale, args.established)
    except ValueError as error:
        parser.error(str(error))
    focus_argv, focus_provenance = focus_options(args.focus_regions, args.focus_distance_range)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    tools = build_swift(ROOT)
    workload = tools / "campaign-workload"
    bridge = None
    report = {"status": "preparing", "profile": "diagnostic", "measurement_scope": "development-only",
              "product": args.product, "scene": args.scene, "seconds": args.seconds,
              "requested_geometry": {"width": args.width, "height": args.height, "scale": args.scale,
                                     "pixel_width": args.width * args.scale, "pixel_height": args.height * args.scale, "refresh_hz": 60},
              "patch_sides": args.patch_sides.split(","), "brightness_policy": "fixed-maximum-v1",
              "connection_setup": "operator-established" if args.established else "adb-usb-loopback",
              "camera_orientation": args.camera_orientation, "focus_calibration": focus_provenance,
              "physical_latency_calibrated": False, "cross_product_comparison": False}
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        with maximum([args.tablet], True, args.output / "brightness-original.json"):
            with (args.output / "connection.log").open("w") as connection_log:
                try:
                    if not args.established:
                        if any(not s["builtin"] for s in inventory(workload)):
                            raise ValueError("Disconnect existing external displays before USB diagnostic")
                        bridge = subprocess.Popen([sys.executable, str(ROOT / "scripts/android/usb-display.py"),
                            "--serial", args.tablet, "--skip-install", "--width", str(args.width),
                            "--height", str(args.height), "--codec", args.codec,
                            "--report", str(args.output / "connection.json")], stdout=connection_log, stderr=subprocess.STDOUT)
                        deadline = time.monotonic() + 60
                        while time.monotonic() < deadline:
                            if bridge.poll() is not None:
                                raise RuntimeError("USB session failed; inspect connection.log")
                            try:
                                receipt = json.loads((args.output / "connection.json").read_text())
                                receiver = receipt.get("receiver", {})
                                if receiver.get("frames", receiver.get("rendered", 0)) > 0:
                                    break
                            except (OSError, ValueError):
                                pass
                            time.sleep(.2)
                        else:
                            raise TimeoutError("Tablet did not confirm a rendered frame")
                    before = inventory(workload)
                    selected = target(before, args.width, args.height, args.scale)
                    report["screens"] = before
                    report["target_display_id"] = selected["id"]
                    preparation_argv=[]
                    if args.focus_preparation:
                        path=args.output/'preparation-command.json'
                        argv=[str(workload),str(selected["id"]),'static',str(args.seconds+34),str(args.output/'prefocus-source.json'),'--patch-sides',args.patch_sides]
                        path.write_text(json.dumps(argv,indent=2))
                        report['focus_preparation']={'command':argv,'receipt':'camera/preparation.json','log':'camera/preparation.log','scope':'Static preparation excluded from measured scene'}
                        preparation_argv=['--preparation-command-json',str(path)]
                    # The same standard scene, counters, camera capture and
                    # analysis are used for any operator-established product.
                    capture = [sys.executable, str(Path(__file__).with_name("camera_run.py")),
                        "--serial", args.camera, "--product", args.product, "--seconds", str(args.seconds + 4),
                        "--phase-seconds", str(args.seconds), "--output", str(args.output / "camera"),
                        "--camera-orientation", args.camera_orientation, *focus_argv, *preparation_argv,
                        "--ready-command", str(workload), str(selected["id"]), args.scene, str(args.seconds),
                        str(args.output / "source.json"), "--patch-sides", args.patch_sides]
                    subprocess.run(capture, check=True)
                    if bridge and bridge.poll() is not None:
                        raise RuntimeError("Display disconnected during optical recording")
                    if inventory(workload) != before:
                        raise RuntimeError("Source display geometry changed during recording")
                    fixture = json.loads((args.output / "source.json").read_text())
                    if not fixture.get("valid_fixture") or not fixture.get("valid_geometry"):
                        raise ValueError("Source fixture invalid")
                    sys.path.insert(0, str(ROOT / "benchmarks/display/campaign"))
                    from metrics import optical, source_patch
                    patches = json.loads((args.output / "camera/patches.json").read_text())
                    source_index = source_patch(args.output / "camera/raw/registration.png", patches)
                    report["optical"] = optical(args.output / "camera", source_index)
                    if args.scene == "cursor-optical":
                        from optical_cursor import optical_cursor
                        report["physical_cursor"] = optical_cursor(args.output / "camera", source_index)
                    elif args.scene == "cursor-sweep":
                        from optical_motion import optical_motion
                        report["physical_cursor_motion"] = optical_motion(args.output / "camera", source_index)
                    report["status"] = "passed"
                finally:
                    stop(bridge)
    except BaseException as error:
        report.update(status="invalid", error=str(error))
        raise
    finally:
        (args.output / "diagnostic.json").write_text(json.dumps(report, indent=2))
        title = "Android display diagnostic: " + report["status"]
        body = html.escape(json.dumps(report, indent=2))
        page = args.output / "report.html"
        page.write_text(f'<!doctype html><meta charset="utf-8"><title>{html.escape(title)}</title>'
            '<style>body{font:15px system-ui;max-width:900px;margin:40px auto;padding:0 20px}pre{white-space:pre-wrap}</style>'
            f'<h1>{html.escape(title)}</h1><p>Development diagnostic. Includes panel scanout and camera row bias; '
            f'not calibrated input latency or a cross-product ranking.</p><pre>{body}</pre>')
        if args.show_report:
            subprocess.run(["open", str(page)], check=False)
        print(f"{title}. Report: {page}", flush=True)


if __name__ == "__main__":
    main()
