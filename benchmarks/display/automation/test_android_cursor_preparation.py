"""The cursor warmup must be observed, even outside the content scene window."""
import json
import tempfile
import unittest
from pathlib import Path

from camera_run import preserve_cursor_preparation, scene_window
from test_optical_cursor import coded_rows, event_patch
from optical_cursor import optical_cursor


class CursorPreparationTests(unittest.TestCase):
    def test_real_cursor_gate_retains_warmup_excluded_by_content_window(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); camera=root/'camera'; camera.mkdir()
            cursor={'pattern':'coded-four-lane-v5','shape_logical_size':[48,32],
                'center_y_logical':148,'reference_origin_y_logical':132,
                'reference_center_y_logical':164,'reference_shape_logical_size':[96,64],
                'preparation_event_id':0,'first_measured_event_id':1,'preparation_s':.5,
                'errors':0,'position_restored':True,'steps':32,
                'observed_events':[{'event':i,'matches':True,'expected_raster_sha256':'a'*64,
                    'observed_raster_sha256':'a'*64,'observed_size':[48,32],
                    'observed_hotspot':[24,16]} for i in range(32)]}
            (root/'source.json').write_text(json.dumps({'fixture':{'cursor':cursor}}))
            (camera/'summary.json').write_text(json.dumps({'measured_sensor_hz':200,
                'video_metadata':{'nominal_playback_fps':40}}))
            samples=coded_rows(.12)
            for row in samples:
                counter=0 if row['playback_s']<.25 else 1+int(row['playback_s']*60)
                row['counter_ids']=[counter,counter]
                row['playback_s']*=5
            window,_=scene_window(samples,0,7.5,.2)
            (camera/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in window))
            with self.assertRaisesRegex(ValueError,'preparation identity'):
                optical_cursor(camera,0)
            preserve_cursor_preparation(camera,samples,cursor)
            result=optical_cursor(camera,0)
            self.assertAlmostEqual(result['state_reference_to_cursor_ms']['median'],120,places=4)
            self.assertEqual(result['excluded_preparation_event_id'],0)
            # Direct generic fixture output and campaign-wrapped output share
            # the same strict source-raster and preparation acceptance gates.
            (root/'source.json').write_text(json.dumps({'cursor':cursor}))
            self.assertEqual(optical_cursor(camera,0)['state_reference_to_cursor_ms'],
                result['state_reference_to_cursor_ms'])
            # Missing receiver warmup must fail, even though content counters pass.
            for row in samples:
                if row['playback_s']<2:
                    row['cursor_events'][1]=event_patch(None)
            preserve_cursor_preparation(camera,samples,cursor)
            with self.assertRaisesRegex(ValueError,'preparation identity'):
                optical_cursor(camera,0)

    def test_content_capture_does_not_create_a_cursor_evidence_file(self):
        with tempfile.TemporaryDirectory() as directory:
            preserve_cursor_preparation(directory,[],{})
            self.assertFalse((Path(directory)/'cursor-optical-frames.jsonl').exists())


if __name__=='__main__':
    unittest.main()
