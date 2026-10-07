import unittest
from capture_phone import validate_sensor

class CameraValidation(unittest.TestCase):
    def test_real_cadence(self):
        rows=[{'sensor_ns':round(i*1e9/240)} for i in range(481)]
        self.assertAlmostEqual(validate_sensor(rows,240)['measured_sensor_hz'],240)
    def test_wrong_cadence(self):
        with self.assertRaises(ValueError):validate_sensor([{'sensor_ns':i*10000000} for i in range(481)],240)
    def test_duplicate_timestamp(self):
        with self.assertRaises(ValueError):validate_sensor([{'sensor_ns':0}]*240,240)
    def test_short_run(self):
        with self.assertRaises(ValueError):validate_sensor([{'sensor_ns':0}],240)

class MarkerDetection(unittest.TestCase):
    def test_washed_corner_colors_still_require_four_corner_geometry(self):
        try:
            import numpy as np
            from PIL import Image
            from find_patches import find
        except ImportError:self.skipTest('Camera environment required')
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as root:
            image=np.zeros((200,500,3),dtype=np.uint8)
            for x,y,color in [(10,10,[255,40,255]),(374,10,[219,250,255]),(10,150,[255,27,0]),(374,150,[187,250,255])]:image[y:y+20,x:x+20]=color
            p=Path(root)/'corners.png';Image.fromarray(image).save(p)
            self.assertEqual(len(find(p)),1)
            image[150:170,374:394]=255;Image.fromarray(image).save(p)
            self.assertEqual(find(p),[])

    def test_cyan_corner_connected_to_blue_background(self):
        try:
            import numpy as np
            from PIL import Image
            from find_patches import find
        except ImportError:self.skipTest('Camera environment required')
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as root:
            image=np.full((200,500,3),[190,220,225],dtype=np.uint8)
            for x,y,color in [(10,10,[255,40,255]),(374,10,[0,227,251]),(10,150,[255,27,0]),(374,150,[100,229,47])]:image[y:y+20,x:x+20]=color
            p=Path(root)/'corners.png';Image.fromarray(image).save(p)
            self.assertEqual(len(find(p)),1)

class SceneWindowTests(unittest.TestCase):
    def test_margins_removed_without_hiding_receiver_stall(self):
        from camera_run import scene_window
        rows=[{'playback_s':i/10,'counter_ids':[i if 10<=i<=60 else None,i if 10<=i<=40 else None]} for i in range(80)]
        selected,window=scene_window(rows,0,5,1)
        self.assertEqual(len(selected),51)
        self.assertEqual(window['observed_seconds'],5)
        self.assertIsNone(selected[-1]['counter_ids'][1])
    def test_short_source_scene_rejected(self):
        from camera_run import scene_window
        with self.assertRaises(RuntimeError):scene_window([{'playback_s':i,'counter_ids':[i,None]} for i in range(4)],0,5,1)

class TrackingCoverageTests(unittest.TestCase):
    def test_continuous_markers_accepted(self):
        from camera_run import validate_tracking
        rows=[{'tracked_corners':[[[1,2]]*4,[[3,4]]*4]} for _ in range(100)]
        self.assertEqual(validate_tracking(rows,2)['coverage_per_patch'],[1,1])
    def test_hidden_receiver_rejected(self):
        from camera_run import validate_tracking
        rows=[{'tracked_corners':[[[1,2]]*4,[[3,4]]*4 if i<90 else []]} for i in range(100)]
        with self.assertRaises(RuntimeError):validate_tracking(rows,2)

class NativeTrackingTests(unittest.TestCase):
    def test_marker_tracking_geometry(self):
        import subprocess
        from pathlib import Path
        tool=Path(__file__).parent/'android-camera/tools/read-cells'
        if not tool.exists():self.skipTest('Build native camera tools first')
        subprocess.run([str(tool),'--tracking-selftest'],check=True,capture_output=True)
