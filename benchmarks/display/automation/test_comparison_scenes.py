"""Comparisons retain new observable phases and reject incomplete evidence."""
import copy
import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from compare import compare, comparable


def report(scene='cursor-sweep'):
    host=dict(model='test',chip='test',memory_bytes=1,os='test',architecture='test')
    display=dict(width=1728,height=1084,scale=2,pixel_width=3456,pixel_height=2168,refresh_hz=60)
    phase={'camera':{'capture_profile':dict(helper_apk_sha256='b'*64,camera='0',width=1920,height=1080,
               requested_capture_fps=240,requested_exposure_compensation_steps=0,exposure_compensation_step_ev=1/6)},
           'analysis_pipeline':{'frame_selection':'zero-tolerance-v1',
            'camera_sources':{k:'a'*64 for k in ['camera_run.py','capture_phone.py','find_patches.py']},
            'native_sources':{k:'a'*64 for k in ['probe','extract','read-cells','optical-patches']},
            'metric_sources':{k:'a'*64 for k in ['metrics.py','optical_cursor.py','optical_motion.py','optical_signal.py']}},'scene':scene,'repetition':1,'status':'measured',
           'network_summary':{role:{'dominant_flow_matches_peer':True} for role in ['source','receiver']},
           'physical_cursor_motion':{'longest_observed_receiver_hold_ms':250,'joint_readable_coverage':.95,
               'receiver':{'inter_resolvable_change_ms':{'p95':60},
                           'change_gaps_containing_unreadable_frames_ms':{'max':400}}}}
    return {'product':'fixture','status':'complete_with_declared_limits',
            'context':{'source':{'host':host},'receiver':{'host':host},'source_display':display},
            'settings':dict(renderer='test',seconds=10,repetitions=1,mode='extend',
                            workload_sha256='test',scene_selection=[scene]),'phases':[phase]}


class ComparisonSceneTests(unittest.TestCase):
    def test_warp_and_posted_mouse_events_cannot_be_compared(self):
        a=report();b=copy.deepcopy(a)
        b['settings']['cursor_input_method']='mouse-events'
        self.assertIn('Cursor input method differs',comparable(a,b))
        a['settings']['cursor_input_method']='mouse-events'
        self.assertFalse(comparable(a,b))
    def test_camera_build_and_exposure_must_match(self):
        a=report()
        for field,value in [('helper_apk_sha256','c'*64),('requested_exposure_compensation_steps',-6),
                            ('requested_capture_fps',120)]:
            b=copy.deepcopy(a);b['phases'][0]['camera']['capture_profile'][field]=value
            with self.subTest(field=field):self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        b=copy.deepcopy(a);b['phases'][0]['camera'].pop('capture_profile')
        self.assertIn('fixture camera acquisition provenance unavailable or legacy',comparable(a,b))

    def test_automatic_iso_does_not_change_requested_profile(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['camera']['capture_profile']['observed_iso']=800
        self.assertFalse(comparable(a,b))

    def test_focus_regions_must_match(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['camera']['capture_profile']['requested_focus_regions_normalized']=[[.1,.1,.4,.4]]
        self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        a['phases'][0]['camera']['capture_profile']['requested_focus_regions_normalized']=[[.1,.1,.4,.4]]
        self.assertFalse(comparable(a,b))

    def test_calibrated_focus_distance_ranges_cannot_be_mixed(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['camera']['capture_profile']['requested_focus_distance_range_diopters']=[1.8,2]
        self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        a['phases'][0]['camera']['capture_profile']['requested_focus_distance_range_diopters']=[1.8,2]
        self.assertFalse(comparable(a,b))
        b['phases'][0]['camera']['capture_profile']['requested_focus_distance_range_diopters']=[2,1]
        self.assertIn('fixture camera acquisition provenance unavailable or legacy',comparable(a,b))

    def test_zoom_and_sensor_crop_must_match(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['camera']['capture_profile'].update(requested_zoom=1.4,requested_crop_sensor=[100,200,900,700])
        self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        a['phases'][0]['camera']['capture_profile'].update(requested_zoom=1.4,requested_crop_sensor=[100,200,900,700])
        self.assertFalse(comparable(a,b))
        b['phases'][0]['camera']['capture_profile']['requested_crop_sensor']=[100,200,900,701]
        self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        b['phases'][0]['camera']['capture_profile']['requested_crop_sensor']=None
        self.assertIn('fixture camera acquisition provenance unavailable or legacy',comparable(a,b))

    def test_focus_policy_must_match_even_with_same_helper_hash(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['camera']['capture_profile'].update(focus_policy='settle-and-lock-v1',fixed_focus_lens=False)
        self.assertIn('Camera acquisition profiles differ',comparable(a,b))
        a['phases'][0]['camera']['capture_profile'].update(focus_policy='settle-and-lock-v1',fixed_focus_lens=False)
        self.assertFalse(comparable(a,b))

    def test_selected_motion_and_uncertainty_are_comparable_metrics(self):
        a=report();b=copy.deepcopy(a)
        self.assertFalse(comparable(a,b))
        result=compare(a,b)
        rows={v['metric']:v for v in result['metrics']}
        self.assertTrue(result['comparable_context'])
        self.assertEqual(rows['receiver_confirmed_hold_max_ms']['left']['median'],250)
        self.assertEqual(rows['receiver_unresolved_motion_gap_max_ms']['left']['median'],400)
        self.assertEqual({v['scene'] for v in result['metrics']},{'cursor-sweep'})

    def test_complete_label_does_not_excuse_missing_duplicate_or_failed_phase(self):
        a=report()
        for phases in [[],a['phases']*2,[{**a['phases'][0],'status':'captured'}],
                       [{**a['phases'][0],'repetition':2}]]:
            with self.subTest(phases=phases):
                b=copy.deepcopy(a);b['phases']=phases
                self.assertIn('fixture requested scenes not fully measured',comparable(a,b))

    def test_analysis_changes_or_missing_provenance_reject_parity(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['analysis_pipeline']['frame_selection']='legacy'
        self.assertIn('Optical analysis pipelines differ',comparable(a,b))
        self.assertIn('fixture optical analysis provenance unavailable or legacy',comparable(a,b))
        b=copy.deepcopy(a);b['phases'][0].pop('analysis_pipeline')
        self.assertIn('fixture optical analysis provenance unavailable or legacy',comparable(a,b))

    def test_unknown_metric_is_not_zero_and_boolean_is_not_numeric(self):
        a=report();b=copy.deepcopy(a)
        b['phases'][0]['physical_cursor_motion']['longest_observed_receiver_hold_ms']=False
        rows={v['metric']:v for v in compare(a,b)['metrics']}
        self.assertIsNone(rows['receiver_confirmed_hold_max_ms']['right'])

if __name__=='__main__':unittest.main()
