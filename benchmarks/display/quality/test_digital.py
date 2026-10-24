import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

import digital


class DigitalQualityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        fonts = [Path('/System/Library/Fonts/Supplemental/Arial.ttf'),
                 Path('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf')]
        cls.font = next((path for path in fonts if path.exists()), None)
        if cls.font is None:
            raise unittest.SkipTest('Tests require a real font for the generated text target')
        cls.directory = tempfile.TemporaryDirectory()
        cls.manifest = digital.generate(cls.directory.name, cls.font, scale=1)
        cls.image = Image.open(Path(cls.directory.name) / 'target.png').convert('RGB')
        cls.box = [0, 0, *cls.image.size]

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def score(self, image):
        return digital.compare(self.image, image, self.manifest, self.box, self.box,
                               'Test: same physical pixel size, aligned origins, unchanged RGB pipeline')['regions']

    def test_identity_and_reproducible_target(self):
        with tempfile.TemporaryDirectory() as other:
            regenerated = digital.generate(other, self.font, scale=1)
        self.assertEqual(self.manifest, regenerated)
        for result in self.score(self.image).values():
            self.assertAlmostEqual(result['luma_ssim_box11'], 1)
            self.assertTrue(result['psnr_infinite'])
            self.assertIsNone(result['rgb_psnr_db'])
            self.assertEqual(result['chroma_rmse_bt709'], 0)
            self.assertEqual(result['edge_gradient_rmse'], 0)
        json.dumps(self.score(self.image), allow_nan=False)

    def test_blur_reduces_text_detail_and_edge_quality(self):
        mild = self.score(self.image.filter(ImageFilter.GaussianBlur(.5)))
        strong = self.score(self.image.filter(ImageFilter.GaussianBlur(1.5)))
        for size in (10, 12, 14, 18, 24):
            name = f'text_{size}pt'
            self.assertLess(strong[name]['luma_ssim_box11'], mild[name]['luma_ssim_box11'])
            self.assertLess(strong[name]['rgb_psnr_db'], mild[name]['rgb_psnr_db'])
            self.assertGreater(strong[name]['edge_gradient_rmse'], mild[name]['edge_gradient_rmse'])

    def test_chroma_subsampling_is_detected_separately(self):
        rgb = np.asarray(self.image, dtype=float) / 255
        y, c = digital.luma(rgb), digital.chroma(rgb)
        # A reproducible 4:2:0-like chroma degradation, keeping full-resolution Y.
        half = c.reshape(c.shape[0] // 2, 2, c.shape[1] // 2, 2, 2).mean(axis=(1, 3))
        c = half.repeat(2, axis=0).repeat(2, axis=1)
        r, b = y + 1.5748 * c[..., 1], y + 1.8556 * c[..., 0]
        g = (y - .2126 * r - .0722 * b) / .7152
        degraded = Image.fromarray(np.clip(np.stack((r, g, b), axis=-1) * 255, 0, 255).round().astype('uint8'))
        score = self.score(degraded)
        self.assertGreater(score['colored_edges_1px']['chroma_rmse_bt709'], .1)
        self.assertLess(score['colored_edges_1px']['rgb_psnr_db'], 20)
        self.assertLess(score['grayscale_gradient']['chroma_rmse_bt709'], .005)

    def test_geometry_and_registration_are_not_silently_corrected(self):
        with self.assertRaisesRegex(ValueError, 'registration'):
            digital.compare(self.image, self.image, self.manifest, self.box, self.box, '')
        smaller = [0, 0, self.image.width - 1, self.image.height]
        with self.assertRaisesRegex(ValueError, 'backing dimensions'):
            digital.compare(self.image, self.image, self.manifest, self.box, smaller, 'verified')
        with self.assertRaisesRegex(ValueError, 'outside'):
            digital.crop(self.image, [1, 0, self.image.width, self.image.height])

    def test_matched_crops_allow_different_screenshot_origins(self):
        canvas = Image.new('RGB', (self.image.width + 60, self.image.height + 80))
        canvas.paste(self.image, (30, 40))
        report = digital.compare(self.image, canvas, self.manifest, self.box,
                                 [30, 40, *self.image.size], 'Fiducials verified; same backing scale')
        self.assertTrue(all(value['psnr_infinite'] for value in report['regions'].values()))


if __name__ == '__main__':
    unittest.main()
