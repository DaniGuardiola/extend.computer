import copy
import unittest
from android_display import target, validate_scale


class DisplayScaleTests(unittest.TestCase):
    def screens(self, scale=1):
        return [{"builtin": True, "mirrored": False, "id": 1},
                {"builtin": False, "mirrored": False, "id": 2, "width": 1138, "height": 710,
                 "pixel_width": 1138 * scale, "pixel_height": 710 * scale, "scale": scale, "refresh_hz": 60}]

    def test_logical_geometry_and_actual_backing_at_both_scales(self):
        for scale in (1, 2):
            screens = self.screens(scale)
            self.assertEqual(target(screens, 1138, 710, scale), screens[1])
        self.assertEqual(target(self.screens(), 1138, 710)["scale"], 1)

    def test_scale_two_cannot_use_development_bridge(self):
        validate_scale(1, False)
        validate_scale(2, True)
        with self.assertRaisesRegex(ValueError, "--established"):
            validate_scale(2, False)
        for invalid in (0, 3, True):
            with self.assertRaises(ValueError): validate_scale(invalid, True)

    def test_geometry_fields_must_all_match(self):
        for key, value in (("width", 2276), ("height", 1420), ("pixel_width", 1138),
                           ("pixel_height", 710), ("scale", 1)):
            screens = self.screens(2)
            screens[1][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                target(screens, 1138, 710, 2)
        for dimensions in ((0, 710), (1138, -1), (True, 710)):
            with self.assertRaises(ValueError): target(self.screens(), *dimensions)

    def test_mirror_ambiguous_inventory_and_refresh_fail(self):
        screens = self.screens(2)
        screens[1]["mirrored"] = True
        with self.assertRaises(ValueError): target(screens, 1138, 710, 2)
        screens = self.screens(2)
        screens.append(copy.deepcopy(screens[1]))
        with self.assertRaises(ValueError): target(screens, 1138, 710, 2)
        for hz in (30, 120, float("nan"), float("inf")):
            screens = self.screens(2)
            screens[1]["refresh_hz"] = hz
            with self.assertRaises(ValueError): target(screens, 1138, 710, 2)
        with self.assertRaises(ValueError): target(self.screens(2)[1:], 1138, 710, 2)


if __name__ == "__main__":
    unittest.main()
