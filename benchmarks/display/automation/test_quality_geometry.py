import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'campaign'))
from digital_quality import predicted_crop


class QualityGeometryTests(unittest.TestCase):
    def setUp(self):
        self.fixture = {
            'scale': 2,
            'display_logical_size': [1728, 1084],
            'target_in_display_top_left_px': [448, 364],
        }

    def test_exact_backing_pixels(self):
        self.assertEqual(predicted_crop(self.fixture, [3456, 2168]), [448, 364])

    def test_integral_letterbox_without_scaling(self):
        self.assertEqual(predicted_crop(self.fixture, [3476, 2168]), [458, 364])

    def test_one_pixel_height_mismatch_rejects_rescaling(self):
        self.fixture['display_logical_size'][1] = 1085
        with self.assertRaisesRegex(ValueError, 'scales'):
            predicted_crop(self.fixture, [3456, 2168])

    def test_half_pixel_letterbox_rejects(self):
        with self.assertRaisesRegex(ValueError, 'fractional'):
            predicted_crop(self.fixture, [3457, 2168])
