import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'campaign'))
from context import peer_match, selected_udp_endpoint, network
from metrics import classify_route, network_delta
from stages import summarize
from compare import compare
from run import campaign_scenes, process_captures, retry_saved_scenes
import run as campaign
from progress import Progress
from types import SimpleNamespace


class EvidenceTests(unittest.TestCase):
    def test_comparison_rejects_debug_and_unproven_builds(self):
        from compare import comparable
        import copy
        flags={'status':'observed_compile_flags','debug_assertions':False,'development_identity':False,'benchmark_feature':True}
        host={'model':'M3','chip':'M3','memory_bytes':1,'os':'27','architecture':'arm64'}
        context={role:{'host':host,'application':{'instrumentation':{'build':dict(flags)}}} for role in ['source','receiver']}
        context['source_display']={key:1 for key in ['width','height','scale','pixel_width','pixel_height','refresh_hz']}
        report={'product':'extend','context':context,'status':'complete_with_declared_limits','phases':[],
            'settings':{**{key:1 for key in ['renderer','seconds','repetitions','mode','workload_sha256']},'scene_selection':['static']}}
        report['phases']=[{'analysis_pipeline':{'frame_selection':'zero-tolerance-v1',
            'camera_sources':{k:'a'*64 for k in ['camera_run.py','capture_phone.py','find_patches.py']},
            'native_sources':{k:'a'*64 for k in ['probe','extract','read-cells','optical-patches']},
            'metric_sources':{k:'a'*64 for k in ['metrics.py','optical_cursor.py','optical_motion.py','optical_signal.py']}},'scene':'static','repetition':1,'status':'measured','network_summary':{role:{'dominant_flow_matches_peer':True} for role in ['source','receiver']}}]
        report['phases'][0]['camera']={'capture_profile':dict(helper_apk_sha256='b'*64,camera='0',width=1920,height=1080,
            requested_capture_fps=240,requested_exposure_compensation_steps=0,exposure_compensation_step_ev=1/6)}
        self.assertFalse(comparable(report,report))
        for role in ['source','receiver']:
            changed=copy.deepcopy(report);changed['context'][role]['application']['instrumentation']['build']['debug_assertions']=True
            self.assertIn('extend '+role+' non-debug build not confirmed',comparable(report,changed))
            changed['context'][role]['application']['instrumentation']['build']={'status':'unavailable'}
            self.assertIn('extend '+role+' non-debug build not confirmed',comparable(report,changed))

    def test_build_flags_require_explicit_current_launch_evidence(self):
        from context import build_evidence
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'launch.log'
            path.write_text('Display build {"debug_assertions":true,"development_identity":false,"benchmark_feature":true}\n')
            self.assertTrue(build_evidence(path)['debug_assertions'])
            path.write_text('Display build {"debug_assertions":false,"development_identity":false,"benchmark_feature":true}\n')
            self.assertFalse(build_evidence(path)['debug_assertions'])
            for value in ['Display build {"debug_assertions":"false"}\n','unrelated log\n']:
                path.write_text(value);self.assertEqual(build_evidence(path)['status'],'unavailable')

    def test_selected_udp_tuple_requires_fresh_owned_state_and_os_socket(self):
        import bootstrap,time
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'.cache').mkdir();log=root/'app.log'
            state=root/'.cache/extend-session.json';state.write_text(json.dumps({'pid':42,'log':str(log)}))
            row={'wall_ns':time.time_ns(),'protocol':'udp','local':'10.0.0.2:45000','remote':'10.0.0.3:46000','responses_received':1}
            app={'product':'extend','main_pid':42,'pids':[42]}
            log.write_text('Display route '+json.dumps(row)+'\n')
            with patch.object(bootstrap,'ROOT',root):
                self.assertTrue(selected_udp_endpoint(app,'10.0.0.3')['matches_peer'])
                csv=',interface,state,bytes_in,bytes_out,re-tx,rtt_avg\nudp4 10.0.0.2:45000<->*:*,en0,,9000,1000,,\n'
                with patch('context.command',return_value=csv):
                    result=network(app,'10.0.0.3')['flows'][0]
                    self.assertTrue(result['matches_peer']);self.assertEqual(result['matches_peer_basis'],'selected_candidate_pair_and_os_local_endpoint')
                with patch('context.command',return_value=csv.replace(':45000',':45001')):
                    self.assertIsNone(network(app,'10.0.0.3')['flows'][0]['matches_peer'])
                state.write_text(json.dumps({'pid':43,'log':str(log)}));self.assertIsNone(selected_udp_endpoint(app,'10.0.0.3'))
                state.write_text(json.dumps({'pid':42,'log':str(log)}))
                row['wall_ns']-=13_000_000_000;log.write_text('Display route '+json.dumps(row)+'\n')
                self.assertIsNone(selected_udp_endpoint(app,'10.0.0.3'))

    def test_selected_tuple_change_does_not_confirm_stable_peer_route(self):
        before={'errors':[],'flows':[{'flow_sha256':'socket','bytes_in':'100','bytes_out':'0','re-tx':'0','matches_peer':True,'selected_tuple_sha256':'old','protocol':'udp','interface':'en0'}]}
        after={'errors':[],'flows':[{**before['flows'][0],'bytes_in':'5000','selected_tuple_sha256':'new'}]}
        self.assertIsNone(network_delta(before,after)['dominant_flow_matches_peer'])
    def test_mapped_ice_candidates_require_the_actual_peer_socket(self):
        import bootstrap,time
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'.cache').mkdir();log=root/'app.log'
            (root/'.cache/extend-session.json').write_text(json.dumps({'pid':42,'log':str(log)}))
            row={'wall_ns':time.time_ns(),'protocol':'udp','responses_received':1,
                 'local':'10.254.1.2:45000','remote':'10.254.1.3:46000',
                 'socket_local':'169.254.226.32:45000','socket_remote':'169.254.173.134:46000'}
            app={'product':'extend','main_pid':42,'pids':[42]}
            csv=',interface,state,bytes_in,bytes_out,re-tx,rtt_avg\nudp4 169.254.226.32:45000<->*:*,en10,,9000,1000,,\n'
            log.write_text('Display route '+json.dumps(row)+'\n')
            with patch.object(bootstrap,'ROOT',root),patch('context.command',return_value=csv):
                selected=selected_udp_endpoint(app,'169.254.173.134')
                self.assertTrue(selected['matches_peer'])
                self.assertNotEqual(selected['tuple_sha256'],selected['ice_tuple_sha256'])
                result=network(app,'169.254.173.134')['flows'][0]
                self.assertTrue(result['matches_peer'])
                self.assertEqual(result['matches_peer_basis'],'selected_socket_pair_and_os_local_endpoint')
                self.assertEqual(result['ice_tuple_sha256'],selected['ice_tuple_sha256'])
                row['socket_remote']='10.0.100.23:46000';log.write_text('Display route '+json.dumps(row)+'\n')
                self.assertFalse(network(app,'169.254.173.134')['flows'][0]['matches_peer'])
                row['socket_remote']=None;log.write_text('Display route '+json.dumps(row)+'\n')
                self.assertIsNone(selected_udp_endpoint(app,'169.254.173.134'))
    def test_source_cadence_respects_selected_refresh_rate(self):
        display={'id':1,'builtin':False,'mirrored':False,'refresh_hz':30}
        result={'context':{'displays':[display],'application':{'executable_sha256':'build'}},
                'fixture':{'source_callback_hz':29},'scene':'motion'}
        campaign.verify_source(result,display,'auto','build')
        result['fixture']['source_callback_hz']=24
        with self.assertRaisesRegex(RuntimeError,'cadence'):
            campaign.verify_source(result,display,'auto','build')
        display['refresh_hz']=60;result['fixture']['source_callback_hz']=49
        with self.assertRaisesRegex(RuntimeError,'cadence'):
            campaign.verify_source(result,display,'auto','build')
    def diagnostic_args(self,**kwargs):
        return SimpleNamespace(**{'smoke':False,'quick':False,'check':False,'process':None,'include_cursor':False,
            'scene':None,'seconds':None,'repetitions':None,**kwargs})

    def test_diagnostic_selection_order_and_distinct_profile(self):
        args=self.diagnostic_args(scene=['panel','cursor'],seconds=8)
        self.assertEqual(campaign.campaign_mode(args),'diagnostic')
        settings={'campaign_profile':'diagnostic','scene_selection':args.selected_scenes}
        self.assertEqual(campaign.scene_order(settings,1),['panel','cursor'])
        self.assertEqual(campaign.scene_order(settings,2),['cursor','panel'])
        self.assertEqual(settings['scene_selection'],['panel','cursor'])
        self.assertEqual(args.repetitions,1)
        self.assertEqual(campaign.campaign_mode(self.diagnostic_args(seconds=30)),'diagnostic')
        self.assertEqual(campaign.campaign_mode(self.diagnostic_args()),'full')

    def test_optional_optical_cursor_phases_join_full_and_quick_suites(self):
        settings={'include_cursor':True,'include_optical_cursor':True}
        self.assertEqual(campaign.campaign_scenes(settings),
            ['static','scroll','panel','motion','recovery','cursor','cursor-optical','cursor-sweep'])
        args=self.diagnostic_args(quick=True,include_optical_cursor=True)
        self.assertEqual(campaign.campaign_mode(args),'quick')
        self.assertEqual(args.repetitions,1)
        with self.assertRaisesRegex(ValueError,'--scene'):
            campaign.campaign_mode(self.diagnostic_args(scene=['cursor-sweep'],include_optical_cursor=True))

    def test_diagnostic_validation_and_resume_overrides(self):
        invalid=[{'scene':['unknown']},{'scene':['motion','motion']},{'scene':['cursor'],'include_cursor':True},
            {'scene':['motion'],'check':True},{'seconds':10,'smoke':True},{'seconds':10,'process':'saved'}]
        for flags in invalid:
            with self.subTest(flags=flags),self.assertRaises(ValueError):campaign.campaign_mode(self.diagnostic_args(**flags))
        saved={'settings':{'campaign_profile':'diagnostic','scene_selection':['scroll'],'seconds':8,'repetitions':2}}
        args=self.diagnostic_args();self.assertEqual(campaign.campaign_mode(args,saved),'diagnostic')
        self.assertEqual(args.selected_scenes,['scroll']);self.assertEqual(args.repetitions,2)
        for flags in [{'scene':['scroll']},{'seconds':8}]:
            with self.subTest(flags=flags),self.assertRaisesRegex(ValueError,'inherits saved'):
                campaign.campaign_mode(self.diagnostic_args(**flags),saved)

    def test_cli_rejects_diagnostic_conflicts_before_hardware(self):
        for options in [['--scene','unknown'],['--scene','motion','--scene','motion'],['--check','--scene','motion'],
                ['--smoke','--seconds','5'],['--smoke','--include-optical-cursor'],['--process','saved','--include-optical-cursor'],['--process','saved','--scene','motion'],['--scene','motion','--seconds','55']]:
            with self.subTest(options=options),patch.object(sys,'argv',['campaign','--product','extend',*options]),\
                    patch.object(campaign,'Client',side_effect=AssertionError('Hardware touched')),patch('sys.stderr'):
                with self.assertRaises(SystemExit):campaign.main()
    def test_marker_zero_wrap_repeat_and_invalid_samples(self):
        rows=[{'stage':'capture_marker','frame_ns':i+1,'at_ns':(i+1)*1000000,'bytes':value}
              for i,value in enumerate([4095,4095,0,0,1])]
        rows.append({'stage':'capture_marker_invalid','at_ns':6000000})
        result=summarize(rows,0,10000000)['synthetic_marker']['stages']['capture_marker']
        self.assertEqual(result['valid_samples'],5)
        self.assertEqual(result['invalid_samples'],1)
        self.assertEqual(result['content_changes'],2)
        self.assertEqual(result['repeated_samples'],2)
        self.assertEqual(result['content_change_hz'],200)
        self.assertEqual(summarize([],0,10)['synthetic_marker']['status'],'unavailable')

    def test_bad_sensor_scene_does_not_block_independent_processing(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            report={'status':'failed','settings':{'seconds':30,'repetitions':1},'phases':[
                {'repetition':1,'scene':'scroll','status':'captured'},
                {'repetition':1,'scene':'motion','status':'captured'}]}
            for row in report['phases']:
                (root/('01-'+row['scene'])/'camera').mkdir(parents=True)
            calls=[]
            def process(root,report,row,adb,phone,ui):
                calls.append(row['scene'])
                if row['scene']=='scroll':
                    (root/'01-scroll/camera/failure.json').write_text(json.dumps({'error':'Non-increasing sensor timestamps','retryable':False}))
                    raise subprocess.CalledProcessError(1,['camera'])
                row['status']='measured'
            with patch.object(campaign,'process_capture',side_effect=process),patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError,'other scenes retained'):
                    process_captures(root,report,None,None,Progress())
                self.assertEqual(calls,['scroll','motion'])
                self.assertEqual(report['phases'][1]['status'],'measured')
                self.assertFalse(report['phases'][0]['processing_failure']['retryable'])
                calls.clear()
                with self.assertRaises(RuntimeError):process_captures(root,report,None,None,Progress())
                self.assertEqual(calls,[])
            manifest=json.loads((root/'manifest.json').read_text())
            self.assertEqual(manifest['artifacts']['report.json'],campaign.digest(root/'report.json'))

    def test_transient_processing_failure_can_retry_same_capture(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'01-motion/camera').mkdir(parents=True)
            row={'repetition':1,'scene':'motion','status':'captured'}
            report={'settings':{'seconds':30,'repetitions':1},'phases':[row]}
            with patch.object(campaign,'process_capture',side_effect=subprocess.CalledProcessError(1,['adb'])),patch('builtins.print'):
                with self.assertRaises(RuntimeError):process_captures(root,report,None,None,Progress())
            self.assertTrue(row['processing_failure']['retryable'])
            def success(root,report,row,adb,phone,ui):row['status']='measured'
            with patch.object(campaign,'process_capture',side_effect=success),patch('builtins.print'):
                process_captures(root,report,None,None,Progress())
            self.assertEqual(row['status'],'measured');self.assertNotIn('processing_failure',row)

    def test_explicit_retry_archives_only_failed_scene_and_checkpoint(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'01-scroll').mkdir();(root/'01-scroll/evidence').write_text('original invalid evidence')
            failed={'repetition':1,'scene':'scroll','status':'captured','processing_failure':{'retryable':False}}
            valid={'repetition':1,'scene':'motion','status':'measured'}
            report={'phases':[failed,valid]};completed={(1,'scroll'),(1,'motion')}
            retry_saved_scenes(root,report,completed,['1:scroll'])
            self.assertEqual(report['phases'],[valid]);self.assertEqual(completed,{(1,'motion')})
            archive=root/report['retried_scenes'][0]['evidence']
            self.assertEqual((archive/'evidence').read_text(),'original invalid evidence')
            self.assertEqual(json.loads((archive/'checkpoint.json').read_text()),failed)
            with self.assertRaises(ValueError):retry_saved_scenes(root,report,completed,['motion'])

    def test_optional_cursor_leaves_default_scene_list_unchanged(self):
        standard = ['static', 'scroll', 'panel', 'motion', 'recovery']
        self.assertEqual(campaign_scenes({}), standard)
        self.assertEqual(campaign_scenes({'include_cursor': True}), standard + ['cursor'])
        self.assertEqual(campaign_scenes({'campaign_profile': 'setup_check'}), ['motion'])

    def test_cursor_zero_frame_ids_are_counted_without_fake_latency(self):
        rows = [{'stage': 'cursor_layer_submit', 'frame_ns': 0, 'at_ns': value}
                for value in [1000000, 11000000, 21000000]]
        result = summarize(rows, 0, 30000000)['cursor']
        stage = result['stages']['cursor_layer_submit']
        self.assertEqual(stage['events'], 3)
        self.assertEqual(stage['hz'], 100)
        self.assertEqual(stage['inter_update_ms']['p95'], 10)
        self.assertEqual(stage['last_update_age_ms'], 9)
        self.assertIsNone(result['delivery_latency_ms'])
        self.assertEqual(summarize([], 0, 10)['cursor']['status'], 'unavailable')

    def test_actual_hardware_observation_retains_false_and_latest(self):
        rows = [{'stage': 'encoder_hardware', 'at_ns': 10, 'bytes': 1},
                {'stage': 'encoder_hardware', 'at_ns': 20, 'bytes': 0},
                {'stage': 'decoder_hardware', 'at_ns': 40, 'bytes': 1}]
        config = summarize(rows, 0, 30)['configuration']
        self.assertIs(config['encoder_hardware'], False)
        self.assertNotIn('decoder_hardware', config)

    def test_unconnected_udp_does_not_prove_non_lan(self):
        self.assertIsNone(peer_match('udp4 10.0.0.2:45000<->*:*', '10.0.0.3'))
        self.assertIsNone(peer_match('udp4 *:45000<->*:*', '10.0.0.3'))
        def sample(n):
            return {'errors': [], 'flows': [{'flow_sha256': 'one', 'bytes_in': str(n),
                'bytes_out': '0', 're-tx': '', 'matches_peer': None,
                'protocol': 'udp', 'interface': 'en0'}]}
        side = network_delta(sample(0), sample(10000))
        self.assertEqual(classify_route({'source': side, 'receiver': side})['classification'], 'unknown')

    def test_only_remote_address_is_peer_evidence(self):
        self.assertTrue(peer_match('tcp4 10.0.0.2:123->10.0.0.3:456', '10.0.0.3'))
        self.assertFalse(peer_match('tcp4 10.0.0.3:123->10.0.0.4:456', '10.0.0.3'))
        self.assertIsNone(peer_match('tcp4 10.0.0.2:123->10.0.0.3:456', None))

    def test_stale_trace_reports_actual_measurement_window(self):
        result = summarize([{'stage': 'encoded', 'at_ns': 100, 'frame_ns': 1}], 1000, 2000)
        self.assertEqual(result['samples'], 0)
        self.assertEqual(result['window']['trace_last_ns'], 100)
        self.assertIn('session freshness', result['reason'])

    def test_gpu_completion_and_drawable_presentation_are_distinct(self):
        rows = [{'stage': stage, 'frame_ns': 7, 'at_ns': value}
                for stage, value in [('decoded', 1000000), ('gpu_completed', 2000000), ('presented', 5000000)]]
        result = summarize(rows, 0, 10000000)
        self.assertEqual(result['decode_to_gpu_complete_ms']['median'], 1)
        self.assertEqual(result['decode_to_drawable_present_ms']['median'], 4)
        legacy = summarize(rows[:1] + rows[2:], 0, 10000000)
        self.assertIsNone(legacy['decode_to_drawable_present_ms'])
        self.assertIn('legacy', legacy['presentation_evidence'])

    def test_comparison_retains_tail_metrics_and_unavailable(self):
        host = dict(model='test', chip='test', memory_bytes=1, os='test', architecture='test')
        display = dict(width=1, height=1, scale=1, pixel_width=1, pixel_height=1, refresh_hz=60)
        report = {'product': 'extend', 'status': 'complete_with_declared_limits',
            'context': {'source': {'host': host}, 'receiver': {'host': host}, 'source_display': display},
            'settings': dict(renderer='test', seconds=30, repetitions=1, mode='extend', workload_sha256='test'),
            'phases': [{'scene': 'motion', 'optical': {'screen_to_screen_transition_ms': {'median': 10, 'p95': 30},
                'receiver_update_intervals_ms': {'p95': 40, 'max': 100}, 'intervals_over_50ms': 3}}]}
        result = compare(report, {**report, 'phases': [{'scene': 'motion'}]})
        rows = {r['metric']: r for r in result['metrics'] if r['scene'] == 'motion'}
        self.assertEqual(rows['screen_to_screen_p95_ms']['left']['median'], 30)
        self.assertEqual(rows['receiver_frame_gap_max_ms']['left']['median'], 100)
        self.assertIsNone(rows['screen_to_screen_p95_ms']['right'])


@unittest.skipUnless(sys.platform == 'darwin' and shutil.which('xcrun'), 'macOS Swift compiler required')
class NativeTraceTests(unittest.TestCase):
    def test_cursor_path_selftest_without_app_or_pointer_access(self):
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder) / 'cursor-test'
            workload = Path(__file__).resolve().parents[1] / 'workloads/campaign-workload.swift'
            subprocess.run(['xcrun', 'swiftc', str(workload), '-o', str(executable)], check=True, capture_output=True)
            result = subprocess.run([str(executable), '--cursor-path-selftest'], check=True, capture_output=True, text=True)
            self.assertIn('self-test passed', result.stdout)

    def test_restarted_helper_appends_without_truncating_or_following_symlink(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            main = root / 'main.swift'
            main.write_text('DisplayMetrics.shared.event("encoded", timestamp: 7, bytes: 12)\nDisplayMetrics.shared.flush()\n')
            executable = root / 'trace-test'
            metrics = Path(__file__).resolve().parents[3] / 'native/macos/Display/Metrics.swift'
            subprocess.run(['xcrun', 'swiftc', str(metrics), str(main), '-o', str(executable)], check=True, capture_output=True)
            trace = root / 'trace.jsonl'
            env = {**os.environ, 'EXTEND_DISPLAY_METRICS': str(trace)}
            subprocess.run([str(executable)], env=env, check=True)
            subprocess.run([str(executable)], env=env, check=True)
            self.assertEqual(len([json.loads(line) for line in trace.read_text().splitlines()]), 2)
            trace.chmod(0o644)
            subprocess.run([str(executable)], env=env, check=True)
            self.assertEqual(len(trace.read_text().splitlines()), 2)
            link = root / 'link.jsonl'
            link.symlink_to(trace)
            subprocess.run([str(executable)], env={**env, 'EXTEND_DISPLAY_METRICS': str(link)}, check=True)
            self.assertEqual(len(trace.read_text().splitlines()), 2)


if __name__ == '__main__':
    unittest.main()
