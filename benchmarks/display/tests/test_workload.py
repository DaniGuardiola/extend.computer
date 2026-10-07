import math
import unittest

from benchmarks.display.serve import validate


def phase():
    return {"kind": "phase_start", "data": {
        "run_id": "01234567-89ab-4cde-8f01-23456789abcd",
        "scene": "motion", "measured": False,
        "started_utc": "2026-10-05T00:00:00.000Z",
        "viewport_width": 1024, "viewport_height": 768,
        "canvas_pixel_width": 2048, "canvas_pixel_height": 1280,
        "device_pixel_ratio": 2,
    }}


class WorkloadMetadataTests(unittest.TestCase):
    def test_all_four_reviewed_scenes_are_accepted(self):
        for scene in ("static", "scroll", "panel", "motion"):
            event = phase()
            event["data"]["scene"] = scene
            self.assertEqual(validate(event), event)

    def test_unexpected_text_cannot_be_written_as_metadata(self):
        for field in ("clipboard", "headers", "url", "browser_history", "account"):
            event = phase()
            event["data"][field] = "PRIVATE_SENTINEL"
            with self.assertRaises(ValueError):
                validate(event)

    def test_numeric_fields_reject_nonfinite_values_and_strings(self):
        for value in (math.nan, math.inf, -1, True, "PRIVATE_SENTINEL"):
            event = phase()
            event["data"]["viewport_width"] = value
            with self.assertRaises(ValueError):
                validate(event)

    def test_cancelled_phase_end_keeps_missing_static_cadence_unknown(self):
        event = phase()
        event["kind"] = "phase_end"
        event["data"].update({
            "scene": "static", "elapsed_seconds": 5,
            "source_callbacks": 0, "source_callback_hz": None,
            "source_interval_p50_ms": None, "source_interval_p95_ms": None,
            "source_interval_max_ms": None, "source_intervals_over_33_33_ms": 0,
            "hidden_during_phase": False, "resized_during_phase": False,
            "cancelled": True,
        })
        self.assertIsNone(validate(event)["data"]["source_callback_hz"])
        del event["data"]["cancelled"]
        with self.assertRaises(ValueError):
            validate(event)


if __name__ == "__main__":
    unittest.main()
