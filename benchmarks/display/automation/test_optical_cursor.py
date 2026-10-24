"""Physical cursor observations fail closed on stale or ambiguous transitions."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from optical_cursor import lane,summarize

def rows(delay=.12):
    result=[]
    for i in range(1200):
        t=i/200
        labels=[int(t/.25)%4,None if t<delay else int((t-delay)/.25)%4]
        result.append({'playback_s':t,'cursor_scores':[[1 if j==v else 0 for j in range(4)] for v in labels]})
    return result

class OpticalCursorTests(unittest.TestCase):
    def test_known_delay_and_swapped_source(self):
        values=rows();r=summarize(values,1,200,0)
        self.assertAlmostEqual(r['state_reference_to_cursor_ms']['median'],120,places=4)
        self.assertGreater(r['matched_transition_coverage'],.95)
        swapped=[{**row,'cursor_scores':row['cursor_scores'][::-1]} for row in values]
        other=summarize(swapped,1,200,1)
        self.assertEqual(other['state_reference_to_cursor_ms'],r['state_reference_to_cursor_ms'])
        self.assertEqual(other['readable_coverage'],r['readable_coverage'][::-1])
    def test_ambiguous_color_and_missing_evidence(self):
        self.assertIsNone(lane([1,1,0,0]));self.assertIsNone(lane([float('nan'),0,0,0]))
        with self.assertRaisesRegex(ValueError,'coverage'):summarize([{'playback_s':i/200} for i in range(200)],1,200,0)
    def test_stall_cannot_match_later_cycle_or_be_averaged_away(self):
        for delay in [.6,1.12]:
            with self.assertRaisesRegex(ValueError,'loss|coverage'):summarize(rows(delay),1,200,0)
        values=rows()
        for row in values[400:550]:row['cursor_scores'][1]=[1,0,0,0]
        with self.assertRaises(ValueError):summarize(values,1,200,0)
    def test_timestamps_and_bad_source_cadence(self):
        values=rows();values[200]['playback_s']=values[199]['playback_s']
        with self.assertRaisesRegex(ValueError,'timestamps'):summarize(values,1,200,0)
        with self.assertRaises(ValueError):summarize(rows(),1,50,0)

from optical_cursor import event_value,summarize_events

def code(event):
    crc=0
    for bit in reversed(range(12)):
        feedback=((crc>>3)&1)^((event>>bit)&1);crc=(crc<<1)&15
        if feedback:crc^=3
    return (event<<4)|crc

def event_patch(event):
    events=[{'border_score':0,'cells':[20]*12} for _ in range(4)]
    if event is not None:
        word=code(event)
        events[event%4]={'border_score':1,'cells':[220 if (word>>(11-bit))&1 else 20 for bit in range(12)]}
    return events

def coded_rows(delay=.12):
    result=[]
    for i in range(1600):
        t=i/200
        source=int(t/.25);receiver=None if t-delay<0 else int((t-delay)/.25)
        result.append({'playback_s':t,'cursor_events':[event_patch(source),event_patch(receiver)],
            'patches':[[[20,220]*8,[220,20]*8] for _ in range(2)]})
    return result

class CodedCursorTests(unittest.TestCase):
    def test_bounded_sampling_keeps_ambiguous_id_failure(self):
        row=coded_rows()[100];selected=row['cursor_events'][0][2]
        alternative=event_patch(6)[2]
        selected['candidates']=[{**alternative,'sample_offset_camera_px':[1,0]}]
        self.assertIsNone(event_value(row,0))
        selected['candidates']=[{**event_patch(2)[2],'sample_offset_camera_px':[3,0]}]
        self.assertIsNone(event_value(row,0))
        selected['candidates']=[{**event_patch(2)[2],'sample_offset_camera_px':[1,0]}]
        self.assertEqual(event_value(row,0),2)
    def test_unobserved_source_cursor_is_not_accepted(self):
        import json,tempfile
        from optical_cursor import optical_cursor
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);camera=root/'camera';camera.mkdir()
            (root/'source.json').write_text(json.dumps({'fixture':{'cursor':{'pattern':'coded-four-lane-v2','shape_logical_size':[48,32]}}}))
            (camera/'summary.json').write_text(json.dumps({'measured_sensor_hz':200,'video_metadata':{'nominal_playback_fps':40}}))
            (camera/'optical-frames.jsonl').write_text('\n'.join(json.dumps(row) for row in coded_rows(-.025)))
            with self.assertRaisesRegex(ValueError,'raster verification'):optical_cursor(camera,0)

    def test_candidate_border_is_individually_validated(self):
        for score in [float('nan'), float('inf'), -.01, 1.01]:
            row=coded_rows()[100]
            row['cursor_events'][0][2]['candidates']=[{**event_patch(2)[2], 'border_score':score, 'sample_offset_camera_px':[0,1]}]
            self.assertIsNone(event_value(row,0))

    def test_nested_campaign_fixture_selects_coded_matcher(self):
        import json,tempfile
        from optical_cursor import optical_cursor
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);camera=root/'camera';camera.mkdir()
            (root/'source.json').write_text(json.dumps({'fixture':{'cursor':{'pattern':'coded-four-lane-v2','shape_logical_size':[48,32], 'errors':0, 'position_restored':True, 'steps':16, 'observed_events':[{'event':i, 'matches':True, 'expected_raster_sha256':'a'*64, 'observed_raster_sha256':'a'*64, 'observed_size':[48,32], 'observed_hotspot':[24,16]} for i in range(16)]}}}))
            (camera/'summary.json').write_text(json.dumps({'measured_sensor_hz':200,'video_metadata':{'nominal_playback_fps':40}}))
            rows=coded_rows(-.025)
            for row in rows:row['playback_s']*=5
            (camera/'optical-frames.jsonl').write_text('\n'.join(json.dumps(row) for row in rows))
            self.assertEqual(optical_cursor(camera,0)['pattern'],'coded-four-lane-v2')
            source=json.loads((root/'source.json').read_text())
            source['fixture']['cursor'].update(pattern='coded-four-lane-v3',center_y_logical=148,reference_origin_y_logical=132)
            (root/'source.json').write_text(json.dumps(source))
            self.assertEqual(optical_cursor(camera,0)['pattern'],'coded-four-lane-v3')
            source['fixture']['cursor'].update(pattern='coded-four-lane-v4',preparation_event_id=0,first_measured_event_id=1,preparation_s=.5)
            (root/'source.json').write_text(json.dumps(source))
            self.assertEqual(optical_cursor(camera,0)['excluded_preparation_event_id'],0)
            source['fixture']['cursor']['preparation_s']=0
            (root/'source.json').write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError,'preparation'):optical_cursor(camera,0)
            source['fixture']['cursor']['preparation_s']=.5
            source['fixture']['cursor'].update(pattern='coded-four-lane-v5',reference_center_y_logical=164,reference_shape_logical_size=[96,64])
            (root/'source.json').write_text(json.dumps(source))
            self.assertEqual(optical_cursor(camera,0)['pattern'],'coded-four-lane-v5')
            source['fixture']['cursor']['reference_shape_logical_size']=[48,32]
            (root/'source.json').write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError,'enlarged cursor reference'):optical_cursor(camera,0)
            source['fixture']['cursor']['reference_shape_logical_size']=[96,64]
            source['fixture']['cursor']['center_y_logical']=140
            (root/'source.json').write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError,'placement'):optical_cursor(camera,0)
            (root/'source.json').write_text(json.dumps({'fixture':{'cursor':{'pattern':'coded-four-lane-v2','shape_logical_size':[40,24]}}}))
            with self.assertRaisesRegex(ValueError,'geometry'):optical_cursor(camera,0)
    def test_preparation_identity_does_not_relax_measured_cadence(self):
        rows=coded_rows(.025)
        for row in rows[:20]:row['cursor_events'][0]=event_patch(None)
        with self.assertRaisesRegex(ValueError,'cadence'):summarize_events(rows,1,200,0)
        self.assertEqual(summarize_events(rows,1,200,0,0)['status'],'measured')
        for row in rows[200:220]:row['cursor_events'][0]=event_patch(None)
        with self.assertRaisesRegex(ValueError,'cadence'):summarize_events(rows,1,200,0,0)
        with self.assertRaisesRegex(ValueError,'preparation'):summarize_events(coded_rows(),1,200,0,1)

    def test_preparation_reset_preserves_stale_state_and_rejects_later_regression(self):
        rows=coded_rows(.025)
        for row in rows[:10]:row['cursor_events'][1]=event_patch(41)
        result=summarize_events(rows,1,200,0,0)
        self.assertEqual(result['preparation_observation']['preexisting_ids_left_to_right'][1],[41])
        with self.assertRaisesRegex(ValueError,'regressed'):summarize_events(rows,1,200,0)
        for row in rows[250:270]:row['cursor_events'][1]=event_patch(0)
        with self.assertRaisesRegex(ValueError,'regressed'):summarize_events(rows,1,200,0,0)
        rows=coded_rows(.025)
        for row in rows[:80]:row['cursor_events'][1]=event_patch(None)
        with self.assertRaisesRegex(ValueError,'preparation identity'):summarize_events(rows,1,200,0,0)

    def test_positive_and_negative_signed_reference_delay(self):
        for delay in [.12,-.025]:
            result=summarize_events(coded_rows(delay),1,200,0)
            self.assertAlmostEqual(result['state_reference_to_cursor_ms']['median'],delay*1000,places=3)
            self.assertTrue(result['unique_event_ids_verified'])
            self.assertGreater(result['matched_transition_coverage'],.95)
    def test_crc_and_position_corruption_rejected(self):
        row=coded_rows()[100]
        self.assertEqual(event_value(row,0),2)
        row['cursor_events'][0][2]['cells'][0]=220
        self.assertIsNone(event_value(row,0))
        row=coded_rows()[100];row['cursor_events'][0]=row['cursor_events'][0][1:]+row['cursor_events'][0][:1]
        self.assertIsNone(event_value(row,0))
    def test_stale_later_cycle_cannot_match_same_lane(self):
        for delay in [.6,1.12,-.2]:
            with self.assertRaises(ValueError):summarize_events(coded_rows(delay),1,200,0)
        rows=coded_rows()
        for row in rows[600:900]:row['cursor_events'][1]=event_patch(8)
        with self.assertRaises(ValueError):summarize_events(rows,1,200,0)

    def test_missing_camera_evidence_and_regression_fail(self):
        rows=coded_rows()
        for row in rows:row.pop('cursor_events')
        with self.assertRaises(ValueError):summarize_events(rows,1,200,0)
        rows=coded_rows()
        for row in rows[600:700]:row['cursor_events'][0]=event_patch(1)
        with self.assertRaises(ValueError):summarize_events(rows,1,200,0)

if __name__=='__main__':unittest.main()
