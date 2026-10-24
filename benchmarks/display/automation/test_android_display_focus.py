import hashlib, json, tempfile, unittest
from pathlib import Path
from android_display import focus_options

class FocusForwardingTests(unittest.TestCase):
    def test_default_has_no_calibration(self):
        self.assertEqual(focus_options(),([],{}))

    def test_calibration_bytes_validated_and_retained(self):
        with tempfile.TemporaryDirectory() as root:
            regions=Path(root)/'regions.json'; distance=Path(root)/'distance.json'
            regions.write_text(json.dumps([[.1,.2,.3,.4]]));distance.write_text(json.dumps([1,3]))
            argv,receipt=focus_options(regions,distance)
            self.assertEqual(argv,['--focus-regions',str(regions.resolve()),'--focus-distance-range',str(distance.resolve())])
            self.assertEqual(receipt['focus-regions']['value'],[[.1,.2,.3,.4]])
            self.assertEqual(receipt['focus-distance-range']['value'],[1,3])
            self.assertEqual(receipt['focus-regions']['sha256'],hashlib.sha256(regions.read_bytes()).hexdigest())
            regions.write_text('[[0,0,2,1]]')
            with self.assertRaises(ValueError):focus_options(regions,distance)
            regions.write_text('[[0,0,1,1]]');distance.write_text('[3,1]')
            with self.assertRaises(ValueError):focus_options(regions,distance)
