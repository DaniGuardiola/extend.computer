import tempfile
import unittest
from pathlib import Path
from PIL import Image,ImageCms
from digital import generate
from registration import normalized_capture,register


class RegistrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        font=Path('/System/Library/Fonts/Supplemental/Arial.ttf')
        if not font.exists():raise unittest.SkipTest('Synthetic fixture font unavailable')
        cls.folder=tempfile.TemporaryDirectory()
        cls.manifest=generate(cls.folder.name,font,1)
        cls.reference=Image.open(Path(cls.folder.name)/'target.png').convert('RGB')

    @classmethod
    def tearDownClass(cls):cls.folder.cleanup()

    def test_integer_shift_recovers_without_resampling(self):
        received=Image.new('RGB',(1400,820));received.paste(self.reference,(42,51))
        box,evidence=register(self.reference,received,self.manifest,[40,50])
        self.assertEqual(box,[42,51,1280,720])
        self.assertEqual(evidence['resampling'],'none')

    def test_wrong_scale_and_large_translation_reject(self):
        received=Image.new('RGB',(1400,820));received.paste(self.reference.resize((1248,702)),(40,50))
        with self.assertRaises(ValueError):register(self.reference,received,self.manifest,[40,50])
        received.paste(self.reference,(52,50))
        with self.assertRaises(ValueError):register(self.reference,received,self.manifest,[40,50])

    def test_missing_corner_rejects(self):
        received=Image.new('RGB',(1400,820));received.paste(self.reference,(40,50))
        received.paste((128,128,128),(44,54,72,82))
        with self.assertRaises(ValueError):register(self.reference,received,self.manifest,[40,50])

    def test_color_profile_is_required_and_recorded(self):
        with self.assertRaisesRegex(ValueError,'ICC'):normalized_capture(self.reference)
        image=self.reference.copy()
        image.info['icc_profile']=ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()
        result,evidence=normalized_capture(image)
        self.assertEqual(result.size,image.size)
        self.assertEqual(evidence['output_space'],'sRGB')
        self.assertFalse(evidence['geometry_resampled'])
