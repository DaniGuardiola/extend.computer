import copy
import json
from pathlib import Path
import tempfile
import unittest
from summarize_android_resolution import summarize, render


class ResolutionSummaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.report = {"status": "passed", "product": "extend-native-usb", "scene": "motion", "seconds": 8,
                       "target_display_id": 2, "screens": [{"id": 2, "width": 1280, "height": 800,
                       "pixel_width": 1280, "pixel_height": 800, "scale": 1, "refresh_hz": 60}],
                       "optical": {"screen_to_screen_transition_ms": {"median": 50, "p95": 80, "count": 25},
                       "receiver_visible_hz": 40, "receiver_update_intervals_ms": {"p95": 30},
                       "source_visible_updates": 100, "receiver_visible_updates": 80}}

    def add(self, name, report=None, provenance=True, change=None):
        folder = self.root / name
        folder.mkdir()
        (folder / "diagnostic.json").write_text(json.dumps(report or self.report))
        if provenance:
            profile = {"apk_sha256": "a" * 64, "mac_binary_sha256": "b" * 64,
                       "helper_cli": ["capture", "--codec", "hevc"], "source_config": {"codec": "hevc", "fps": 60},
                       "route": {"kind": "native-usb", "before": "before.log", "after": "after.log"},
                       **{key: "before.log" for key in ("media_before", "media_after", "usb_before", "usb_after")}}
            if change:
                change(profile)
            (folder / "campaign-profile.json").write_text(json.dumps(profile))
            (folder / "before.log").write_text("operator receipt before")
            (folder / "after.log").write_text("operator receipt after")
        camera = folder / "camera"
        camera.mkdir()
        capture = {"helper_apk_sha256": "d" * 64, "requested_orientation": "landscape",
                   "orientation_policy": "stable", "focus_policy": "locked", "camera": "0",
                   "width": 1920, "height": 1080, "requested_capture_fps": 240, "requested_zoom": 1,
                   "requested_exposure_compensation_steps": 0, "exposure_compensation_step_ev": 0.1,
                   "requested_focus_regions_normalized": [[0.2, 0.3, 0.4, 0.5]],
                   "requested_focus_distance_range_diopters": None, "requested_crop_sensor": None}
        (camera / "summary.json").write_text(json.dumps({"status": "camera_check_passed", "capture_fps": 240, "capture_profile": capture}))
        (camera / "provenance.json").write_text(json.dumps({"fixture_sha256": "e" * 64,
                     "tool_source_sha256": {"camera_run.py": "f" * 64},
                     "native_tools": {"extract": {"source_sha256": "1" * 64, "binary_sha256": "2" * 64}}}))
        return folder

    def test_clip_values_not_frame_pooled_and_short_repeats_remain_accepted(self):
        self.add("one")
        second = copy.deepcopy(self.report)
        second["optical"]["screen_to_screen_transition_ms"].update(median=100, count=10000)
        self.add("two", second)
        group = summarize(self.root)["groups"][0]
        self.assertEqual(group["statistics"]["median_ms"], {"values": [50, 100], "median": 75, "min": 50, "max": 100})
        self.assertEqual(group["n"], 2)
        self.assertIn("limited", group["repeat_note"])

    def test_three_repeats_and_geometry_split(self):
        for i in range(3):
            self.add(str(i))
        other = copy.deepcopy(self.report)
        other["screens"][0].update(width=1920, height=1200, pixel_width=1920, pixel_height=1200)
        self.add("large", other)
        report = summarize(self.root)
        self.assertEqual(sorted(g["n"] for g in report["groups"]), [1, 3])
        self.assertIn("marker size", render(report))
        self.assertFalse(report["clips"][0]["provenance"]["route_verified"])

    def test_missing_changed_build_config_route_do_not_match(self):
        self.add("base")
        self.add("missing1", provenance=False)
        self.add("missing2", provenance=False)
        self.add("build", change=lambda p: p.update(apk_sha256="c" * 64))
        self.add("config", change=lambda p: p.update(source_config={"codec": "h264", "fps": 60}))
        self.add("route", change=lambda p: p["route"].update(kind="adb-usb-loopback"))
        report = summarize(self.root)
        self.assertEqual(len(report["groups"]), 6)
        self.assertTrue(all(g["n"] == 1 for g in report["groups"]))

    def test_prep_invalid_nonfinite_and_no_geometry_excluded(self):
        self.add("prep", {"status": "preparing"})
        self.add("invalid", {"status": "invalid", "error": "camera failed"})
        bad = copy.deepcopy(self.report)
        bad["optical"]["receiver_visible_hz"] = float("nan")
        self.add("nan", bad)
        bad = copy.deepcopy(self.report)
        bad["screens"] = []
        self.add("geometry", bad)
        report = summarize(self.root)
        self.assertEqual(report["counts"], {"prep": 1, "invalid": 3, "accepted": 0})
        self.assertEqual(report["groups"], [])
        json.dumps(report, allow_nan=False)

    def test_camera_profile_and_fixture_changes_split_repeats(self):
        self.add("base")
        for name, key, value in (("helper", "helper_apk_sha256", "3" * 64),
                                 ("fps", "requested_capture_fps", 120),
                                 ("focus", "requested_focus_regions_normalized", [[0, 0, 1, 1]]),
                                 ("zoom", "requested_zoom", 2),
                                 ("exposure", "requested_exposure_compensation_steps", 1)):
            folder = self.add(name)
            path = folder / "camera/summary.json"
            data = json.loads(path.read_text())
            data["capture_profile"][key] = value
            path.write_text(json.dumps(data))
        folder = self.add("fixture")
        path = folder / "camera/provenance.json"
        data = json.loads(path.read_text())
        data["fixture_sha256"] = "4" * 64
        path.write_text(json.dumps(data))
        self.assertEqual(len(summarize(self.root)["groups"]), 7)

    def test_timestamps_ignored_missing_camera_not_matched(self):
        self.add("one")
        folder = self.add("two")
        diagnostic = json.loads((folder / "diagnostic.json").read_text())
        diagnostic["requested_geometry"] = {"width": 1280, "height": 800, "scale": 1}
        (folder / "diagnostic.json").write_text(json.dumps(diagnostic))
        path = folder / "camera/provenance.json"
        data = json.loads(path.read_text())
        data["completed_utc"] = "different"
        path.write_text(json.dumps(data))
        folder = self.add("missing")
        (folder / "camera/summary.json").unlink()
        groups = summarize(self.root)["groups"]
        self.assertEqual(sorted(g["n"] for g in groups), [1, 2])

    def test_receipt_escape_rejected_and_html_escaped(self):
        self.add("<script>", change=lambda p: p.update(usb_before="../outside"))
        (self.root / "outside").write_text("secret")
        report = summarize(self.root)
        self.assertEqual(report["clips"][0]["provenance"]["status"], "incomplete")
        self.assertNotIn("<script>", render(report))
        self.assertNotIn("secret", json.dumps(report))


if __name__ == "__main__":
    unittest.main()
