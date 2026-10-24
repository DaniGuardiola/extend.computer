import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

from find_patches import find


class MarkerComponentTests(unittest.TestCase):
    def image(self):
        image = Image.new('RGB', (700, 400), 'black')
        draw = ImageDraw.Draw(image)
        for (x, y), color in [((200, 100), (255, 0, 255)),
                              ((500, 100), (0, 230, 230)),
                              ((200, 220), (255, 0, 0)),
                              ((497, 228), (0, 255, 0))]:
            draw.rectangle((x - 9, y - 9, x + 10, y + 10), fill=color)
        return image

    def detect(self, image):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'synthetic.png'
            image.save(path)
            return find(path)

    def test_tinted_fragment_does_not_replace_whole_cyan_marker(self):
        image = self.image()
        baseline = self.detect(image)
        # This subset is shifted toward the bottom-left. The old geometric
        # score preferred it over the complete square on a skewed patch.
        ImageDraw.Draw(image).rectangle((491, 103, 498, 110), fill=(0, 230, 255))
        observed = self.detect(image)
        self.assertEqual(len(observed), 1)
        np.testing.assert_allclose(observed, baseline)
        np.testing.assert_allclose(observed[0][1], [500.5, 100.5])

    def test_strict_fallback_survives_rejected_large_component(self):
        image = self.image()
        draw = ImageDraw.Draw(image)
        draw.rectangle((480, 70, 540, 130), fill=(0, 230, 230))
        draw.rectangle((491, 91, 510, 110), fill=(0, 230, 255))
        observed = self.detect(image)
        self.assertEqual(len(observed), 1)
        np.testing.assert_allclose(observed[0][1], [500.5, 100.5])

    def test_small_text_and_cell_fragments_cannot_replace_equal_size_corners(self):
        # Observed camera failure: 6x9 cyan text and 19x10 cyan/green cell
        # fragments beat the genuine 24x23/25x24 perspective quad's score.
        image=Image.new('RGB',(1600,700),'black');draw=ImageDraw.Draw(image)
        boxes=[((1046,373,1069,395),(222,45,243)),
               ((1442,403,1465,425),(7,199,210)),
               ((1029,514,1053,537),(235,30,25)),
               ((1432,552,1456,575),(12,183,6))]
        for box,color in boxes:draw.rectangle(box,fill=color)
        expected=self.detect(image)
        self.assertEqual(len(expected),1)
        draw.rectangle((1391,300,1396,308),fill=(173,195,207))
        draw.rectangle((1366,429,1384,438),fill=(154,192,215))
        np.testing.assert_allclose(self.detect(image),expected)

    def test_rotated_patch_keeps_all_four_marker_centers(self):
        image = Image.new('RGB', (700, 400), 'black')
        points = [(200, 100), (500, 140), (184, 220), (484, 260)]
        colors = [(255, 0, 255), (0, 230, 230), (255, 0, 0), (0, 255, 0)]
        draw = ImageDraw.Draw(image)
        for (x, y), color in zip(points, colors):
            draw.rectangle((x-9, y-9, x+10, y+10), fill=color)
        observed = self.detect(image)
        self.assertEqual(len(observed), 1)
        np.testing.assert_allclose(observed[0], np.array(points) + .5)

        # A stray bottom-right marker must not complete the tilted patch.
        draw.rectangle((474, 250, 495, 271), fill='black')
        draw.rectangle((574, 290, 595, 311), fill=(0, 255, 0))
        self.assertEqual(self.detect(image), [])


if __name__ == '__main__':
    unittest.main()
