#!/usr/bin/env python3
"""Summarize existing Android optical diagnostics without pooling camera frames."""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import re
import statistics
import subprocess

METRICS = {
    "median_ms": ("screen_to_screen_transition_ms", "median"),
    "p95_ms": ("screen_to_screen_transition_ms", "p95"),
    "visible_hz": ("receiver_visible_hz",),
    "frame_gap_p95_ms": ("receiver_update_intervals_ms", "p95"),
}
NOTICE = ("Within-product resolution tradeoff only. Geometry is intentionally unmatched across resolutions; "
          "marker size and panel scanout / camera row bias can change. These are optical screen-to-screen "
          "observations, not calibrated transport latency, input latency, or a cross-product ranking. "
          "Visible update rate is a lower bound. Statistics summarize clip values; camera frames are never pooled.")


def read_json(path):
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("JSON exceeds 16 MiB")
    return json.loads(path.read_text())


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def camera_signature(folder):
    """Stable capture/tool identity only; host timestamps and run IDs are excluded."""
    for name in ("summary.json", "provenance.json"):
        path = (folder / "camera" / name).resolve()
        if not path.is_relative_to(folder.resolve()):
            raise ValueError("Camera metadata escapes clip directory")
    summary = read_json(folder / "camera" / "summary.json")
    tools = read_json(folder / "camera" / "provenance.json")
    profile = summary.get("capture_profile")
    if summary.get("status") != "camera_check_passed" or not isinstance(profile, dict):
        raise ValueError("Missing validated camera capture profile")
    def valid_hash(value):
        return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value)
    if not valid_hash(profile.get("helper_apk_sha256")) or not valid_hash(tools.get("fixture_sha256")):
        raise ValueError("Missing camera helper or source fixture hash")
    for key in ("requested_orientation", "orientation_policy", "focus_policy", "camera"):
        if not isinstance(profile.get(key), str) or not profile[key]:
            raise ValueError("Missing camera capture field: " + key)
    for key in ("width", "height", "requested_capture_fps", "requested_zoom"):
        if not finite(profile.get(key)) or profile[key] <= 0:
            raise ValueError("Invalid camera capture field: " + key)
    for key in ("requested_exposure_compensation_steps", "exposure_compensation_step_ev"):
        if not finite(profile.get(key)):
            raise ValueError("Missing camera exposure field: " + key)
    for key in ("requested_focus_regions_normalized", "requested_focus_distance_range_diopters", "requested_crop_sensor"):
        if key not in profile:
            raise ValueError("Missing camera capture field: " + key)
    if not finite(summary.get("capture_fps")) or summary["capture_fps"] <= 0:
        raise ValueError("Invalid camera FPS")
    tool_sources = tools.get("tool_source_sha256")
    native_tools = tools.get("native_tools")
    if not isinstance(tool_sources, dict) or not tool_sources or not all(valid_hash(v) for v in tool_sources.values()):
        raise ValueError("Missing camera tool source hashes")
    if not isinstance(native_tools, dict) or not native_tools:
        raise ValueError("Missing native camera tool hashes")
    natives = {}
    for name, tool in native_tools.items():
        if not isinstance(tool, dict) or not all(valid_hash(tool.get(k)) for k in ("source_sha256", "binary_sha256")):
            raise ValueError("Invalid native camera tool hashes")
        natives[name] = {k: tool[k] for k in ("source_sha256", "binary_sha256")}
    # Reject nested NaN/Infinity too. JSON round-trip preserves requested ROI values.
    json.dumps(profile, allow_nan=False)
    return {"capture_profile": profile, "capture_fps": summary["capture_fps"],
            "fixture_sha256": tools["fixture_sha256"], "tool_source_sha256": tool_sources,
            "native_tools": natives, "packages": tools.get("packages")}


def provenance(folder):
    path = folder / "campaign-profile.json"
    if not path.exists():
        return {"status": "missing", "route_verified": False, "issues": ["No campaign-profile.json"]}
    try:
        data = read_json(path)
        issues = []
        for key in ("apk_sha256", "mac_binary_sha256"):
            if not isinstance(data.get(key), str) or not re.fullmatch(r"[0-9a-f]{64}", data[key]):
                issues.append("Missing or invalid " + key)
        if not data.get("helper_cli") or not isinstance(data.get("source_config"), dict) or not data["source_config"]:
            issues.append("Missing helper CLI or source configuration")
        route = data.get("route", {})
        if not isinstance(route, dict):
            route = {}
        if route.get("kind") != "native-usb":
            issues.append("Route is not declared native-usb")
        receipts = {}
        for key, reference in [("route_before", route.get("before")), ("route_after", route.get("after"))] + [
                (key, data.get(key)) for key in ("media_before", "media_after", "usb_before", "usb_after")]:
            if not isinstance(reference, str) or not reference:
                issues.append("Missing receipt: " + key)
                continue
            receipt = (folder / reference).resolve()
            # Local clip artifacts only; do not read arbitrary paths from report data.
            if not receipt.is_relative_to(folder.resolve()) or not receipt.is_file():
                issues.append("Invalid receipt: " + key)
                continue
            if receipt.stat().st_size > 16 * 1024 * 1024:
                issues.append("Oversized receipt: " + key)
                continue
            receipts[key] = {"path": reference, "sha256": hashlib.sha256(receipt.read_bytes()).hexdigest()}
        signature = {key: data.get(key) for key in ("apk_sha256", "mac_binary_sha256", "helper_cli", "source_config")}
        signature["route_kind"] = route.get("kind")
        try:
            signature["camera"] = camera_signature(folder)
        except (ValueError, OSError, TypeError, AttributeError) as error:
            issues.append("Camera provenance: " + str(error))
        return {"status": "documented" if not issues else "incomplete", "route_verified": False,
                "route_note": "Receipt presence and hashes checked; route contents require operator review.",
                "issues": issues, "receipts": receipts, "configuration": signature}
    except (ValueError, OSError, TypeError) as error:
        return {"status": "invalid", "route_verified": False, "issues": [str(error)]}


def clip(path, root):
    result = {"clip": str(path.parent.relative_to(root)), "classification": "invalid", "metrics": {},
              "provenance": provenance(path.parent)}
    try:
        data = read_json(path)
        if not isinstance(data, dict):
            raise ValueError("Diagnostic must be an object")
        result.update(product=data.get("product"), scene=data.get("scene"), diagnostic_status=data.get("status"))
        result["reason"] = data.get("error", "")
        if data.get("status") in ("preparing", "preparation", "prep"):
            result["classification"] = "prep"
            return result
        if data.get("status") != "passed":
            return result
        optical = data.get("optical", {})
        if optical.get("source_visible_updates") == 0 and optical.get("receiver_visible_updates") == 0:
            result.update(classification="prep", reason="Stationary preparation only")
            return result
        target = next((s for s in data.get("screens", []) if s.get("id") == data.get("target_display_id")), None)
        if not target or target.get("mirrored"):
            raise ValueError("Missing unmirrored target geometry")
        geometry = {key: target.get(key) for key in ("width", "height", "pixel_width", "pixel_height", "scale", "refresh_hz")}
        if not all(finite(v) and v > 0 for v in geometry.values()):
            raise ValueError("Invalid target geometry")
        if not isinstance(data.get("product"), str) or not data.get("scene"):
            raise ValueError("Missing product or scene")
        for name, keys in METRICS.items():
            value = optical
            for key in keys:
                value = value[key]
            if not finite(value) or (name in ("visible_hz", "frame_gap_p95_ms") and value < 0):
                raise ValueError("Invalid optical metric: " + name)
            result["metrics"][name] = value
        matched = optical.get("screen_to_screen_transition_ms", {}).get("count")
        if not finite(matched) or matched < 20 or optical.get("source_visible_updates", 0) < 2 or optical.get("receiver_visible_updates", 0) < 2:
            raise ValueError("Insufficient moving matched transitions")
        result.update(classification="accepted", geometry=geometry, matched_transitions=matched,
                      measurement_context={**{k: data.get(k) for k in ("seconds", "patch_sides", "camera_orientation", "brightness_policy", "connection_setup")},
                                           "focus_calibration": {k: {f: v.get(f) for f in ("sha256", "value")} for k, v in (data.get("focus_calibration") or {}).items()}},
                      coverage=optical.get("counter_readable_coverage"), reason="Accepted by android_display optical analysis")
    except (ValueError, OSError, KeyError, TypeError, AttributeError) as error:
        result.update(classification="invalid", reason=str(error), metrics={})
    return result


def summarize(root):
    root = root.resolve()
    clips = [clip(path, root) for path in sorted(root.rglob("diagnostic.json"))]
    groups = {}
    for item in clips:
        if item["classification"] != "accepted":
            continue
        p = item["provenance"]
        # Missing provenance cannot establish matched repeats, even with identical geometry.
        signature = p.get("configuration") if p["status"] == "documented" else {"unmatched_clip": item["clip"]}
        key = json.dumps([item["product"], item["scene"], item["geometry"], item["measurement_context"], signature], sort_keys=True)
        group = groups.setdefault(key, {"product": item["product"], "scene": item["scene"], "geometry": item["geometry"],
                                       "provenance_status": p["status"], "clips": []})
        group["clips"].append(item)
    output = []
    for group in groups.values():
        n = len(group["clips"])
        group["n"] = n
        group["repeat_note"] = "At least three documented matched clips" if n >= 3 else "Fewer than three matched clips; limited repeat evidence"
        group["statistics"] = {}
        for name in METRICS:
            values = [c["metrics"][name] for c in group["clips"]]
            group["statistics"][name] = {"values": values, "median": statistics.median(values), "min": min(values), "max": max(values)}
        output.append(group)
    return {"scope": "within-product-resolution-diagnostic", "notice": NOTICE, "clips": clips, "groups": output,
            "counts": {kind: sum(c["classification"] == kind for c in clips) for kind in ("prep", "invalid", "accepted")}}


def render(report):
    esc = lambda value: html.escape(str(value))
    sections = []
    for group in report["groups"]:
        geometry = group["geometry"]
        rows = "".join("<tr><td>" + esc(key) + "</td><td>" + esc(', '.join(f'{v:.2f}' for v in s['values'])) +
                       f"</td><td>{s['median']:.2f}</td><td>{s['min']:.2f}</td><td>{s['max']:.2f}</td></tr>"
                       for key, s in group["statistics"].items())
        sections.append(f"<section><h2>{esc(group['product'])}: {esc(group['scene'])} · {geometry['width']} × {geometry['height']}</h2>"
                        f"<p>n={group['n']} · {esc(group['repeat_note'])} · provenance: {esc(group['provenance_status'])}</p>"
                        "<table><tr><th>Metric</th><th>Every clip value</th><th>Median</th><th>Min</th><th>Max</th></tr>" + rows + "</table></section>")
    rows = ''.join(f"<tr><td>{esc(c['clip'])}</td><td>{esc(c['classification'])}</td><td>{esc(c.get('reason',''))}</td>"
                   f"<td>{esc(c['provenance']['status'])}: {esc('; '.join(c['provenance'].get('issues',[])))}</td></tr>" for c in report['clips'])
    details = esc(json.dumps(report, indent=2, allow_nan=False))
    return ("<!doctype html><html><meta charset='utf-8'><title>Android resolution campaign</title><style>"
            "body{font:16px system-ui;max-width:1200px;margin:40px auto;padding:0 20px;color:#18212b}"
            "table{border-collapse:collapse;width:100%;margin:20px 0}td,th{border:1px solid #ccd;padding:9px;text-align:left}"
            "section{margin:32px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere}</style><h1>Android resolution campaign</h1>"
            f"<p>{esc(NOTICE)}</p><p>{esc(report['counts'])}</p><p>Accepted means optical diagnostic accepted. "
            "Route receipts are documented evidence, not automatically verified native USB. Missing provenance prevents matched aggregation.</p>" +
            ''.join(sections) + "<h2>All clips, including preparation and invalid attempts</h2><table><tr><th>Clip</th><th>Status</th><th>Reason</th><th>Provenance</th></tr>" +
            rows + f"</table><details><summary>Complete geometry, receipt hashes and clip metrics</summary><pre>{details}</pre></details></html>")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path, help="Output directory (default: ROOT/resolution-summary)")
    parser.add_argument("--show-report", action="store_true")
    args = parser.parse_args()
    report = summarize(args.root)
    output = args.output or args.root / "resolution-summary"
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.json").write_text(json.dumps(report, indent=2, allow_nan=False))
    page = output / "report.html"
    page.write_text(render(report))
    print(page.resolve())
    if args.show_report:
        subprocess.run(["open", str(page.resolve())], check=True)


if __name__ == "__main__":
    main()
