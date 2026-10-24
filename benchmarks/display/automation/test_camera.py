import unittest
from capture_phone import validate_sensor, validate_focus, validate_focus_regions, validate_focus_distance_range, wait_receipt, camera_zoom, validate_crop

class CameraReceiptTests(unittest.TestCase):
    def test_preparation_failure_is_retained_and_reported_without_timeout(self):
        import json, subprocess, tempfile
        from pathlib import Path
        terminal={'status':'failed','error':'Autofocus could not reach calibrated distance range','rejected_focus_distances_diopters':[1.7]}
        def remote(name):
            if name=='result.json':return json.dumps(terminal)
            raise subprocess.CalledProcessError(1,['adb'])
        with tempfile.TemporaryDirectory() as root:
            output=Path(root)
            with self.assertRaisesRegex(ValueError,'Autofocus could not reach'):
                wait_receipt(remote,'ready.json',10,output,clock=lambda:0,sleep=lambda _:self.fail('Failure should be immediate'))
            self.assertEqual(json.loads((output/'capture.json').read_text()),terminal)

    def test_ready_receipt_does_not_require_terminal_result(self):
        import json, tempfile
        from pathlib import Path
        ready={'status':'recording'}
        def remote(name):
            self.assertEqual(name,'ready.json')
            return json.dumps(ready)
        with tempfile.TemporaryDirectory() as root:
            self.assertEqual(wait_receipt(remote,'ready.json',10,Path(root),clock=lambda:0),ready)

class CameraValidation(unittest.TestCase):
    def test_zoom_requires_observed_crop_on_every_frame(self):
        for v in [0,3,True,float('nan'),float('inf')]:
            with self.subTest(value=v),self.assertRaises(ValueError):camera_zoom(v)
        capture={'requested_zoom':1.4,'requested_crop_sensor':[100,200,900,700]}
        self.assertEqual(validate_crop([{'crop_sensor':[100,200,900,700]}],capture)['status'],'crop_verified')
        for rows in [[],[{}],[{'crop_sensor':[100,200,901,700]}]]:
            with self.assertRaises(ValueError):validate_crop(rows,capture)
        self.assertEqual(validate_crop([{}],{})['status'],'unverified_legacy_crop')
        self.assertEqual(validate_crop([{}],{'requested_zoom':1})['status'],'default_crop')

    def test_focus_lock_requires_stationary_focused_lens_every_frame(self):
        capture={'focus_policy':'settle-and-lock-v1','locked_focus_distance_diopters':1.5,'fixed_focus_lens':False}
        rows=[{'focus_distance_diopters':1.5,'af_mode':3,'af_state':4,'lens_state':0}]*10
        self.assertEqual(validate_focus(rows,capture)['status'],'autofocus_lock_verified')
        for key,value in [('focus_distance_diopters',1.6),('af_state',2),('lens_state',1),('af_mode',0),('focus_distance_diopters',float('nan'))]:
            changed=[*rows[:-1],{**rows[-1],key:value}]
            with self.subTest(key=key,value=value),self.assertRaises(ValueError):validate_focus(changed,capture)
        with self.assertRaises(ValueError):validate_focus([],capture)
        self.assertEqual(validate_focus(rows,{})['status'],'unverified_legacy_autofocus')

    def test_focus_regions_require_valid_geometry_and_sensor_ack(self):
        self.assertEqual(validate_focus_regions([[0.1,0.2,0.3,0.4]]),[[0.1,0.2,0.3,0.4]])
        for regions in [[],[[0,0,0,1]],[[False,0,1,1]],[[0,0,float('nan'),1]],[[0,0,2,1]],[[0,0,1,1]]*3]:
            with self.subTest(regions=regions),self.assertRaises(ValueError):validate_focus_regions(regions)
        capture={'focus_policy':'settle-and-lock-v1','locked_focus_distance_diopters':1.5,'requested_focus_regions_sensor':[[20,30,40,50]]}
        row={'focus_distance_diopters':1.5,'af_mode':3,'af_state':4,'lens_state':0,'af_regions_sensor':[[20,30,40,50]]}
        self.assertEqual(validate_focus([row],capture)['status'],'autofocus_lock_verified')
        with self.assertRaises(ValueError):validate_focus([{**row,'af_regions_sensor':[[0,0,0,0]]}],capture)

    def test_calibrated_distance_range_requires_every_recorded_focus_inside(self):
        self.assertEqual(validate_focus_distance_range([1.4,1.6]),[1.4,1.6])
        for bounds in [[1,1],[-1,2],[0,float('inf')],[False,2],[0,101],[1]]:
            with self.subTest(bounds=bounds),self.assertRaises(ValueError):validate_focus_distance_range(bounds)
        capture={'focus_policy':'settle-and-lock-v1','locked_focus_distance_diopters':1.5,'requested_focus_distance_range_diopters':[1.49,1.51]}
        row={'focus_distance_diopters':1.5,'af_mode':3,'af_state':4,'lens_state':0}
        self.assertEqual(validate_focus([row],capture)['status'],'autofocus_lock_verified')
        with self.assertRaisesRegex(ValueError,'calibrated'):validate_focus([row],{**capture,'requested_focus_distance_range_diopters':[2,3]})
        with self.assertRaisesRegex(ValueError,'calibrated'):validate_focus([{**row,'focus_distance_diopters':1.515}],capture)

    def test_fixed_focus_requires_camera_characteristic(self):
        capture={'focus_policy':'settle-and-lock-v1','locked_focus_distance_diopters':0,'fixed_focus_lens':True,'minimum_focus_distance_diopters':0}
        self.assertEqual(validate_focus([{'focus_distance_diopters':0}],capture)['status'],'fixed_focus_verified')
        with self.assertRaises(ValueError):validate_focus([{'focus_distance_diopters':0}],{**capture,'minimum_focus_distance_diopters':2})
    def test_real_cadence(self):
        rows=[{'sensor_ns':round(i*1e9/240)} for i in range(481)]
        self.assertAlmostEqual(validate_sensor(rows,240)['measured_sensor_hz'],240)
    def test_batched_callback_identity_is_disclosed_without_claiming_exposures(self):
        rows=[{'sensor_ns':round(i*1e9/240),'frame_number':i//8} for i in range(481)]
        evidence=validate_sensor(rows,240)['sensor_timestamp_evidence']
        self.assertEqual(evidence['kind'],'batched_camera2_callbacks')
        self.assertEqual(evidence['unique_frame_numbers'],61)
        self.assertEqual(evidence['adjacent_repeated_frame_numbers'],420)
        rows[200]['sensor_ns']=rows[199]['sensor_ns']-140000
        with self.assertRaisesRegex(ValueError,'Non-increasing'):validate_sensor(rows,240)

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
    def test_preparation_zero_excluded_but_measured_gaps_preserved(self):
        from camera_run import scene_window
        rows=[{'playback_s':i/10,'counter_ids':[0 if i<10 else i if i<=60 else None,None], 'tracked_corners':[[],[]]} for i in range(80)]
        rows[30]['counter_ids'][0]=None
        selected,window=scene_window(rows,0,5,1)
        self.assertEqual(window['first_sample'],10)
        self.assertEqual(window['last_sample'],60)
        self.assertEqual(len(selected),51)
        self.assertIsNone(selected[20]['counter_ids'][0])

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
