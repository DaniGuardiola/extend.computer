import importlib.util,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from metrics import optical,resources,quality,source_patch,network_delta,classify_route
from stages import summarize
from compare import comparable
from host import Session
from run import verify_display_profile

class CampaignTests(unittest.TestCase):
    def test_build_evidence_accepts_startup_metadata_but_requires_boolean_flags(self):
        from context import build_evidence
        flags={'debug_assertions':False,'development_identity':False,'benchmark_feature':False}
        with tempfile.TemporaryDirectory() as folder:
            log=Path(folder)/'app.log'
            metadata={'configured_windows':1,'created_webviews':1,'webview_labels':['main']}
            log.write_text('Display build '+json.dumps({**flags,**metadata})+'\n')
            self.assertEqual(build_evidence(log),{'status':'observed_compile_flags',**flags})
            for rejected in [{**flags,'debug_assertions':0},{'debug_assertions':False},[],None]:
                log.write_text('Display build '+json.dumps(rejected)+'\n')
                self.assertEqual(build_evidence(log),{'status':'unavailable'})
            log.write_text('Display build '+json.dumps({**flags,'debug_assertions':True})+'\n')
            self.assertIs(build_evidence(log)['debug_assertions'],True)

    def test_focus_selection_keeps_sharper_default_autofocus(self):
        from focus_camera import select_focus_candidate
        baseline={'candidate':'baseline','status':'verified','regions':None,'balanced_score':.5}
        region={'candidate':'right','status':'verified','regions':[[.5,.4,.8,.7]],'balanced_score':.4}
        rejected={'candidate':'left','status':'rejected','balanced_score':1}
        self.assertIs(select_focus_candidate(baseline,[region,rejected]),baseline)
        self.assertIs(select_focus_candidate(baseline,[{**region,'balanced_score':.6}])['regions'],region['regions'])
        with self.assertRaises(RuntimeError):select_focus_candidate({'status':'rejected'},[rejected])

    def test_unnumbered_encoder_rejections_have_counts_not_frame_rate(self):
        rows=[{'stage':'encode_busy','at_ns':i*1_000_000,'frame_ns':0,'bytes':0} for i in range(4)]
        rows.append({'stage':'encode_busy','at_ns':100_000_000,'frame_ns':0,'bytes':0})
        result=summarize(rows,0,10_000_000)
        self.assertEqual(result['event_counts']['encode_busy'],4)
        self.assertEqual(result['unique_stage_hz']['encode_busy'],0)

    def test_dtls_packet_diagnostics_remain_windowed_and_not_frame_latency(self):
        rows=[{'stage':'dtls_datagram_received','at_ns':2_000_000,'frame_ns':0,'bytes':157},
              {'stage':'dtls_datagram_received','at_ns':1_000_000,'frame_ns':0,'bytes':65},
              {'stage':'dtls_datagram_received','at_ns':20_000_000,'frame_ns':0,'bytes':1129}]
        result=summarize(rows,0,10_000_000)
        packets=result['dtls_datagrams']['received']
        self.assertEqual(packets['count'],2);self.assertEqual(packets['udp_payload_bytes'],222)
        self.assertEqual(packets['inter_datagram_ms']['median'],1)
        self.assertIsNone(result['dtls_datagrams']['sent'])
        self.assertIsNone(result['cursor']['delivery_latency_ms'])

    def test_cursor_sequences_join_only_observed_host_local_stages(self):
        chain=['cursor_position_transport_received','cursor_position_runtime_received',
               'cursor_position_pipe_started','cursor_position_pipe_written',
               'cursor_position_received','cursor_position_layer_submit']
        rows=[{'stage':stage,'at_ns':(i+1)*1_000_000,'frame_ns':7,'bytes':0} for i,stage in enumerate(chain)]
        rows += [{'stage':chain[0],'at_ns':2_000_000,'frame_ns':8,'bytes':0},
                 {'stage':chain[-1],'at_ns':30_000_000,'frame_ns':8,'bytes':0}]
        result=summarize(rows,0,10_000_000)['cursor']
        for name in ['transport_to_runtime_ms','runtime_to_pipe_ms','pipe_write_ms','pipe_to_native_ms','native_to_layer_ms']:
            self.assertEqual(result[name]['count'],1);self.assertEqual(result[name]['median'],1)
        self.assertIsNone(result['capture_to_transport_ms'])
        self.assertIsNone(result['delivery_latency_ms'])

    def test_shape_revisions_join_transport_and_pipe_without_position_ids(self):
        chain=['cursor_shape_transport_received','cursor_shape_runtime_received','cursor_shape_pipe_started','cursor_shape_pipe_written','cursor_shape_received']
        rows=[{'stage':stage,'at_ns':(i+1)*1_000_000,'frame_ns':7} for i,stage in enumerate(chain)]
        rows.append({'stage':'cursor_position_received','at_ns':6_000_000,'frame_ns':7})
        result=summarize(rows,0,10_000_000)['cursor']
        for key in ['shape_transport_to_runtime_ms','shape_runtime_to_pipe_ms','shape_pipe_write_ms','shape_pipe_to_native_ms']:
            self.assertEqual(result[key]['median'],1)
        self.assertIsNone(result['shape_capture_to_transport_ms'])
        self.assertIsNone(result['pipe_to_native_ms'])
        self.assertIsNone(result['shape_transport_to_visible_layer_ms'])
        rows.append({'stage':'cursor_shape_visible_layer_submit','at_ns':8_000_000,'frame_ns':7})
        measured=summarize(rows,0,10_000_000)['cursor']['shape_transport_to_visible_layer_ms']
        self.assertEqual(measured['count'],1)
        self.assertEqual(measured['median'],7)

    def test_short_scene_registration_excludes_post_capture_margin(self):
        sys.path.insert(0,str(Path(__file__).resolve().parent))
        from camera_run import registration_fractions
        # Ten-second fixture has already gone at 80% of a sixteen-second video.
        old_probe=16*.8
        self.assertGreater(old_probe,10)
        times=[16*f for f in registration_fractions(16,10)]
        self.assertTrue(all(1<time<10 for time in times))
        self.assertEqual(registration_fractions(36,30),[.2,2/3])
        self.assertEqual(registration_fractions(36),[.2,.8])

    def test_retina_profile_requires_backing_pixels(self):
        display={'width':1728,'height':1085,'scale':2,'pixel_width':3456,'pixel_height':2170}
        verify_display_profile(display,'1728x1085@2')
        with self.assertRaises(RuntimeError):
            verify_display_profile({**display,'pixel_width':1728,'pixel_height':1085},'1728x1085@2')
        with self.assertRaises(RuntimeError):
            verify_display_profile({**display,'width':1670},'1728x1085@2')
    def test_display_profile_rejects_ambiguous_scale(self):
        with self.assertRaises(ValueError):verify_display_profile({},'1728x1085')
    def test_camera_transition_delay_one_clock(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root);(p/'summary.json').write_text(json.dumps({'measured_sensor_hz':240,'video_metadata':{'nominal_playback_fps':30}}))
            (p/'sensor.jsonl').write_text('{}\n')
            rows=[{'playback_s':i/30,'counter_ids':[i//4,max(0,(i-6)//4)]} for i in range(300)]
            (p/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows))
            report=optical(p)
            self.assertAlmostEqual(report['screen_to_screen_transition_ms']['median'],25)
            self.assertAlmostEqual(report['source_visible_hz'],60)
    def test_reversed_reference_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root);(p/'summary.json').write_text(json.dumps({'measured_sensor_hz':240,'video_metadata':{'nominal_playback_fps':30}}))
            (p/'sensor.jsonl').write_text('{}\n');rows=[{'playback_s':i/30,'counter_ids':[max(0,(i-12)//4),i//4]} for i in range(300)]
            (p/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows))
            with self.assertRaises(ValueError):optical(p)

    def test_preparation_hold_excluded_but_active_stall_retained(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root)
            (p/'summary.json').write_text(json.dumps({'measured_sensor_hz':240,'video_metadata':{'nominal_playback_fps':30}}))
            (p/'sensor.jsonl').write_text('{}\n')
            rows=[]
            for i in range(600):
                source=0 if i<240 else 1+(i-240)//4
                receiver=0 if i<246 else 1+(i-246)//4
                if 400<=i<424:receiver=39
                rows.append({'playback_s':i/30,'counter_ids':[source,receiver]})
            (p/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows))
            result=optical(p)
            self.assertAlmostEqual(result['source_visible_hz'],60)
            self.assertGreaterEqual(result['preparation_to_first_update_ms']['receiver'],1000)
            self.assertLess(result['receiver_update_intervals_ms']['max'],200)
            self.assertGreater(result['receiver_update_intervals_ms']['max'],100)
            self.assertEqual(result['intervals_over_50ms'],1)

    def test_optical_readability_bounds_cover_hidden_transition_timing(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root);(p/'summary.json').write_text(json.dumps({'measured_sensor_hz':240,'video_metadata':{'nominal_playback_fps':30}}))
            (p/'sensor.jsonl').write_text('{}\n')
            rows=[{'playback_s':i/30,'counter_ids':[i//4,max(0,(i-6)//4)]} for i in range(300)]
            # Hide receiver readings around one real transition; its first
            # readable appearance is late, not proof of a late display update.
            for i in range(45,50):rows[i]['counter_ids'][1]=None
            (p/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows))
            result=optical(p)
            self.assertTrue(result['detected_update_rate_is_lower_bound'])
            self.assertGreaterEqual(result['unreadable_counter_intervals_ms']['receiver']['max'],25-1e-9)
            wide=[v for v in result['transition_delay_bounds_ms'] if v['upper_ms']-v['lower_ms']>20]
            self.assertTrue(wide)
            self.assertTrue(all(v['lower_ms']<=25+1e-9<=v['upper_ms'] for v in wide))
            self.assertIsNone(result['unreadable_counter_intervals_ms']['source'])
    def test_cpu_uses_intervals_and_process_scope(self):
        rows=[{'at_ns':0,'processes':[{'pid':1,'cpu_s':10,'rss_bytes':100}], 'collection_ms':2,'system_process_cpu_s':100}, {'at_ns':2_000_000_000,'processes':[{'pid':1,'cpu_s':11,'rss_bytes':200}], 'collection_ms':3,'system_process_cpu_s':102}]
        self.assertEqual(resources(rows)['cpu_percent_one_core']['median'],50)
    def test_new_process_cpu_history_not_counted(self):
        rows=[{'at_ns':0,'processes':[],'collection_ms':1,'system_process_cpu_s':0},{'at_ns':1_000_000_000,'processes':[{'pid':2,'cpu_s':100,'rss_bytes':3}],'collection_ms':1,'system_process_cpu_s':1}]
        self.assertEqual(resources(rows)['cpu_percent_one_core']['median'],0)
    def test_stage_join_never_uses_other_frame(self):
        rows=[{'stage':'submitted','frame_ns':1,'at_ns':100},{'stage':'encoded','frame_ns':2,'at_ns':200}]
        self.assertIsNone(summarize(rows,0,1000)['encode_ms'])
    def test_stage_latency(self):
        rows=[{'stage':'submitted','frame_ns':1,'at_ns':1_000_000},{'stage':'encoded','frame_ns':1,'at_ns':4_000_000,'bytes':100}]
        self.assertEqual(summarize(rows,0,10_000_000)['encode_ms']['median'],3)
    def test_no_unavailable_latency_zero(self):self.assertIsNone(summarize([],0,100)['encode_ms'])
    def test_helper_pipe_contention_is_separate_and_windowed(self):
        rows=[{'stage':'pipe_video_write_us','at_ns':100,'bytes':900000},
            {'stage':'pipe_cursor_wait_us','at_ns':1100,'bytes':4500},
            {'stage':'pipe_video_write_us','at_ns':1200,'bytes':7000},
            {'stage':'cursor_dispatch_wait_us','at_ns':1300,'bytes':12000}]
        result=summarize(rows,1000,2000)
        self.assertEqual(result['helper_pipe']['pipe_cursor_wait_us']['median'],4500)
        self.assertEqual(result['helper_pipe']['pipe_video_write_us']['median'],7000)
        self.assertIsNone(result['helper_pipe']['pipe_cursor_write_us'])
        self.assertIsNone(result['encode_ms'])
        self.assertNotIn('pipe_cursor_wait_us',result['unique_stage_hz'])
        self.assertEqual(result['cursor_dispatch_wait_us']['median'],12000)
        self.assertNotIn('cursor_dispatch_wait_us',result['cursor']['stages'])
    def test_cursor_query_cost_excludes_outside_window_and_is_not_cursor_rate(self):
        rows=[{'stage':'cursor_shape_query_us','at_ns':100,'bytes':9000},
              {'stage':'cursor_shape_query_us','at_ns':1100,'bytes':75},
              {'stage':'cursor_shape_query_us','at_ns':1200,'bytes':125}]
        result=summarize(rows,1000,2000)
        self.assertEqual(result['cursor_shape_query_us']['median'],100)
        self.assertEqual(result['cursor_shape_query_us']['count'],2)
        self.assertNotIn('cursor_shape_query_us',result['unique_stage_hz'])
        self.assertNotIn('cursor_shape_query_us',result['cursor']['stages'])
        self.assertIsNone(summarize([],1000,2000)['cursor_shape_query_us'])

    def test_cursor_source_joins_include_sample_cost_and_concurrent_pipe_reader(self):
        origin=1_000_000_000
        times={'cursor_position_sample_started':0,'cursor_position_sampled':1,
            'cursor_position_submitted':5,'cursor_position_pipe_queued':6,
            'cursor_position_pipe_acquired':10,'cursor_position_pipe_written':12,
            'cursor_position_helper_pipe_received':11,'cursor_position_runtime_taken':13,
            'cursor_position_transport_submitted':14}
        rows=[{'stage':stage,'at_ns':origin+millis*1_000_000,'frame_ns':7} for stage,millis in times.items()]
        # A coalesced sample without transport submission is not a delivered latency.
        rows.append({'stage':'cursor_position_sampled','at_ns':origin+2_000_000,'frame_ns':8})
        rows.append({'stage':'cursor_sample_interval_us','at_ns':origin+3_000_000,'bytes':8333})
        result=summarize(rows,origin,origin+20_000_000)
        cursor=result['cursor']
        self.assertEqual(cursor['source_sample_to_transport_ms']['median'],13)
        self.assertEqual(cursor['source_sample_to_transport_ms']['count'],1)
        self.assertEqual(cursor['source_sample_to_submit_ms']['median'],4)
        self.assertEqual(cursor['source_pipe_wait_ms']['median'],4)
        self.assertEqual(cursor['source_pipe_write_ms']['median'],2)
        self.assertEqual(cursor['source_pipe_to_helper_ms']['median'],5)
        self.assertEqual(cursor['source_helper_to_runtime_ms']['median'],2)
        self.assertEqual(cursor['source_runtime_to_transport_ms']['median'],1)
        self.assertEqual(result['cursor_sample_interval_us']['median'],8333)
        self.assertNotIn('cursor_sample_interval_us',cursor['stages'])

    def test_shape_visible_delay_joins_revision_only_within_host(self):
        rows=[{'stage':'cursor_shape_received','frame_ns':7,'at_ns':1_000_000},
              {'stage':'cursor_shape_layer_submit','frame_ns':7,'at_ns':2_000_000},
              {'stage':'cursor_shape_visible_layer_submit','frame_ns':7,'at_ns':12_000_000},
              {'stage':'cursor_shape_visible_layer_submit','frame_ns':8,'at_ns':99_000_000}]
        result=summarize(rows,0,100_000_000)['cursor']
        self.assertEqual(result['shape_native_to_layer_ms']['median'],1)
        self.assertEqual(result['shape_to_visible_layer_ms']['median'],11)
        self.assertEqual(result['shape_to_visible_layer_ms']['count'],1)

    def test_unknown_host_command_rejected(self):
        s=Session('/tmp/unused','unused',None)
        with self.assertRaises(ValueError):s.start({'id':'../../x','product':'extend','scene':'shell','seconds':30,'screen':'auto'})
    def test_short_phase_rejected(self):
        s=Session('/tmp/unused','unused',None)
        with self.assertRaises(ValueError):s.start({'id':'00000000-0000-0000-0000-000000000000','product':'extend','scene':'motion','seconds':False,'screen':'auto'})
    def test_phase_not_started_without_preflight(self):
        s=Session('/tmp/unused','unused',None)
        with self.assertRaises(ValueError):s.start({'id':'00000000-0000-0000-0000-000000000000','product':'extend','scene':'motion','seconds':30,'screen':'auto'})
    def test_route_dominant_flow_evidence(self):
        def flows(n,peer):return {'flows':[{'flow_sha256':'one','bytes_in':str(n),'bytes_out':'0','re-tx':'0','matches_peer':peer,'protocol':'udp','interface':'en0'}],'errors':[]}
        self.assertTrue(network_delta(flows(0,True),flows(2000,True))['dominant_flow_matches_peer'])
        self.assertIsNone(network_delta(flows(0,True),flows(0,True))['dominant_flow_matches_peer'])
    def test_route_classification_requires_both_hosts(self):
        def side(flag):return {'dominant_flow_matches_peer':flag,'sensor_errors':[]}
        self.assertEqual(classify_route({'source':side(True),'receiver':side(True)})['classification'],'direct_lan')
        self.assertEqual(classify_route({'source':side(False),'receiver':side(False)})['classification'],'non_lan')
        self.assertEqual(classify_route({'source':side(True),'receiver':side(False)})['classification'],'mixed')
        self.assertEqual(classify_route({'source':side(True)})['classification'],'unknown')
    def test_relay_confirmation_must_match_recorded_flows(self):
        side={'dominant_flow_matches_peer':False,'sensor_errors':[],'flow_deltas':[{'flow_sha256':'one','delta':{'bytes_in':2000,'bytes_out':0}}]}
        summary={'source':side,'receiver':side}
        evidence={'kind':'both_hosts_same_relay_endpoint','dominant_flow_sha256':{'source':'one','receiver':'one'}}
        self.assertEqual(classify_route(summary,evidence)['classification'],'relay')
        evidence['dominant_flow_sha256']['source']='other'
        self.assertEqual(classify_route(summary,evidence)['classification'],'non_lan')
    def test_no_authentication_config_import_in_public_core(self):
        from bench import product_adapter
        with patch.dict('os.environ',{},clear=True):self.assertIsNone(product_adapter())
    def test_quality_identical_and_distorted_target(self):
        try:
            import numpy as np
            from PIL import Image
        except ImportError:self.skipTest('Camera environment required for image tests')
        with tempfile.TemporaryDirectory() as root:
            image=np.zeros((220,900,3),dtype=np.uint8)
            patches=[[[10,10],[374,10],[10,150],[374,150]],[[460,10],[824,10],[460,150],[824,150]]]
            for left in [0,450]:
                for x in range(32,353):image[126:147,left+x]=255 if (x//2)%2 else 0
            path=Path(root)/'image.png';Image.fromarray(image).save(path)
            self.assertAlmostEqual(quality(path,patches)['global_optical_ssim'],1)
            image[126:147,450+32:450+353]=255-image[126:147,450+32:450+353]
            Image.fromarray(image).save(path)
            self.assertLess(quality(path,patches)['global_optical_ssim'],.5)
    def test_resource_counter_reset_not_negative(self):
        rows=[{'at_ns':0,'processes':[{'pid':1,'cpu_s':10,'rss_bytes':1}], 'collection_ms':2,'system_process_cpu_s':100},{'at_ns':1000000000,'processes':[{'pid':1,'cpu_s':0,'rss_bytes':1}], 'collection_ms':2,'system_process_cpu_s':0}]
        self.assertEqual(resources(rows)['cpu_percent_one_core']['median'],0)
    def test_comparison_rejects_hardware_change(self):
        def report(model):
            context={r:{'host':{'model':model,'chip':'M','memory_bytes':1,'os':'27','architecture':'arm64'}} for r in ['source','receiver']}
            context['source_display']={k:1 for k in ['width','height','scale','pixel_width','pixel_height','refresh_hz']}
            return {'product':'test','status':'complete_with_declared_limits','context':context,'settings':{k:1 for k in ['renderer','seconds','repetitions','mode','workload_sha256']},'phases':[]}
        self.assertTrue(comparable(report('M3'),report('M4')))
    def test_host_synthetic_phase_lifecycle(self):
        import os,uuid
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);tool=root/'tool';tool.write_text('#!/usr/bin/env python3\nimport sys,time,json\nstart=time.monotonic_ns(); print("ready",flush=True);time.sleep(3)\njson.dump({"valid_geometry":True,"duration_s":3,"start_ns":start,"end_ns":time.monotonic_ns(),"source_callback_hz":60,"placement_args":sys.argv[5:]},open(sys.argv[4],"w"))\n');tool.chmod(0o755)
            app={'executable_sha256':'test','pids':[os.getpid()],'main_pid':os.getpid(),'bundle_path':'/fake'}
            s=Session(root/'runs',tool,None,patch_sides='right,left');s.contexts['extend']={'application':app,'network':{'flows':[],'errors':[]},'displays':[]}
            row={'at_ns':1,'processes':[],'system_process_cpu_s':0,'collection_ms':1,'main_alive':True}
            with patch('host.sample',return_value=row),patch('host.network',return_value={'flows':[],'errors':[]}),patch('host.snapshot',return_value={'status':'unavailable'}),patch('host.application',return_value=app),patch.object(s,'displays',return_value=[]):
                ident=str(uuid.uuid4());s.start({'id':ident,'product':'extend','scene':'motion','seconds':3,'screen':'auto'})
                import time
                deadline=time.monotonic()+10
                while s.job['status']=='running' and time.monotonic()<deadline:time.sleep(.1)
                self.assertEqual(s.job['status'],'complete')
                self.assertEqual(s.job['fixture']['placement_args'],['--patch-sides','right,left'])
                self.assertTrue((root/'runs'/ident/'result.json').exists())

    def test_host_patch_sides_are_bounded_and_default_is_preserved(self):
        self.assertEqual(Session('/tmp/unused','unused',None).patch_sides,'left,right')
        for sides in ['left,left','right,right','arbitrary','right,left --unexpected']:
            with self.subTest(sides=sides),self.assertRaisesRegex(ValueError,'patch sides'):
                Session('/tmp/unused','unused',None,patch_sides=sides)

    def test_host_preserves_cursor_failure_without_geometry_mislabel(self):
        import os,uuid,time
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);tool=root/'tool'
            fixture={'valid_geometry':True,'valid_fixture':False,'failure_reason':'Source cursor raster did not remain applied','duration_s':.05,'cursor':{'errors':1}}
            tool.write_text('#!/usr/bin/env python3\nimport sys,json\nprint("ready",flush=True)\njson.dump('+repr(fixture)+',open(sys.argv[4],"w"))\n');tool.chmod(0o755)
            app={'executable_sha256':'test','pids':[os.getpid()],'main_pid':os.getpid(),'bundle_path':'/fake'}
            session=Session(root/'runs',tool,None);session.contexts['extend']={'application':app,'network':{'flows':[],'errors':[]},'displays':[]}
            row={'at_ns':1,'processes':[],'system_process_cpu_s':0,'collection_ms':1,'main_alive':True}
            with patch('host.sample',return_value=row):
                ident=str(uuid.uuid4());session.start({'id':ident,'product':'extend','scene':'cursor-optical','seconds':3,'screen':'auto'})
                deadline=time.monotonic()+5
                while session.job['status']=='running' and time.monotonic()<deadline:time.sleep(.05)
            self.assertEqual(session.job['status'],'failed')
            self.assertEqual(session.job['reason'],fixture['failure_reason'])
            self.assertEqual(session.job['fixture'],fixture)
            self.assertEqual(json.loads((root/'runs'/ident/'result.json').read_text())['fixture'],fixture)

class CampaignOrchestrationTests(unittest.TestCase):
    def test_complete_campaign_writes_all_scenes_and_hashed_report(self):
        self.campaign_case()
    def test_quick_campaign_preserves_scenes_durations_and_report(self):
        self.campaign_case(quick=True)
    def test_targeted_diagnostic_uses_real_orchestration_and_inherits_resume_selection(self):
        self.campaign_case(diagnostic=True)
    def campaign_case(self,quick=False,diagnostic=False):
        """Hardware seams are fake; real runner, scene order and report finalization execute."""
        import os,time,types
        import run as campaign
        with tempfile.TemporaryDirectory() as tmp:
            tmp=Path(tmp);output=tmp/'result';tools=tmp/'tools';tools.mkdir()
            fixture={'id':2,'builtin':False,'mirrored':False,'width':100,'height':100,'scale':1,'pixel_width':100,'pixel_height':100,'refresh_hz':60}
            app={'product':'extend','main_pid':os.getpid(),'executable_sha256':'test','version':'1','build':'1','display_helpers':[]}
            context={'application':app,'displays':[fixture],'host':{'model':'test'},'network':{'flows':[],'errors':[]}}
            calls=[];events=[];focuses=[];expected_captures=2 if diagnostic else 5 if quick else 15
            row={'at_ns':1,'processes':[{'pid':1,'cpu_s':1,'rss_bytes':1}], 'collection_ms':1,'system_process_cpu_s':1,'main_alive':True}
            class FakeSampler:
                def __init__(self,*_):self.rows=[{**row,'at_ns':i*1000000000} for i in range(1,4)]
                def __enter__(self):return self
                def __exit__(self,*_):pass
                def summary(self):return resources(self.rows)
            class FakeClient:
                def __init__(self,*_):self.config={'url':'https://10.1.1.1:8877'};self.job=None
                def request(self,path,body=None):
                    if path=='/health':return {'workload_sha256':campaign.digest(campaign.ROOT.parents[2]/'benchmarks/display/workloads/campaign-workload.swift')}
                    if path.startswith('/context'):return context
                    if path=='/start':self.job=body;calls.append(body['scene'])
                    return {}
                def wait(self,ident,seconds):
                    return {'context':context,'scene':self.job['scene'],'rows':FakeSampler().rows,'network_start':context['network'],'network_end':context['network'],'stages':{'status':'unavailable'},'fixture':{'source_callback_hz':60},'fault':{'scope_pids':[1]} if self.job['scene']=='recovery' else None}
            client=FakeClient()
            def command(argv,**kwargs):
                argv=[str(x) for x in argv]
                if '--context' in argv:return types.SimpleNamespace(stdout=json.dumps([fixture]))
                if '--windows' in argv:return types.SimpleNamespace(stdout='[]')
                if '--activate' in argv:focuses.append(argv);return types.SimpleNamespace(returncode=0)
                if 'getprop' in argv:return types.SimpleNamespace(stdout='synthetic phone')
                if 'df' in argv:return types.SimpleNamespace(stdout='Filesystem 1K-blocks Used Available Use% Mounted on\n/data 99999999 1 99999998 1% /data\n')
                folder=Path(argv[argv.index('--output')+1]);folder.mkdir(exist_ok=True);(folder/'raw').mkdir(exist_ok=True)
                if '--capture-only' in argv:
                    self.assertEqual(argv[argv.index('--exposure-compensation-steps')+1],'-6' if diagnostic else '0')
                    (folder/'capture.json').write_text('{"status":"complete"}');(folder/'phone-session.json').write_text('{"run_id":"test"}')
                    ack=Path(argv[argv.index('--ack')+1]);ack.write_text(json.dumps({'request_start_ns':0,'ack_ns':0}))
                    phase=json.loads(Path(argv[argv.index('--phase')+1]).read_text());client.request('/start',phase)
                    events.append('capture')
                else:
                    events.append('process')
                    self.assertEqual(len([e for e in events if e=='capture']),expected_captures)
                    (folder/'patches.json').write_text('[]');(folder/'raw/video.mp4').write_bytes(b'synthetic')
                    (folder/'summary.json').write_text(json.dumps({'video_sha256':campaign.digest(folder/'raw/video.mp4')}))
                    # The fake camera must provide the same provenance/selection
                    # contract as the real capture processor; never bypass gates.
                    synthetic=campaign.digest(folder/'raw/video.mp4')
                    (folder/'provenance.json').write_text(json.dumps({
                        'tool_source_sha256':{name:synthetic for name in ['camera_run.py','capture_phone.py','find_patches.py']},
                        'native_tools':{name:{'source_sha256':synthetic} for name in ['probe','extract','read-cells','optical-patches']}}))
                    (folder/'raw/registration.png.timing.json').write_text(json.dumps({
                        'selection':'zero-tolerance-v1','requested_s':0,'actual_s':0,'error_s':0,'max_error_s':1/30}))
                return types.SimpleNamespace(returncode=0)
            options=['--scene','motion','--scene','scroll','--seconds','6','--camera-exposure-compensation-steps','-6'] if diagnostic else ['--quick'] if quick else []
            with patch.object(campaign,'open_report') as opened,patch.object(campaign,'Client',return_value=client),patch.object(campaign,'build_swift',return_value=tools),patch.object(campaign,'application',return_value=app),patch.object(campaign,'load',return_value={}),patch.object(campaign,'save'),patch.object(campaign,'adb_at',return_value=Path('/fake/adb')),patch.object(campaign,'select_phone',return_value='test'),patch.object(campaign,'phone_identity',return_value='hash'),patch.object(campaign,'host',return_value={'model':'test'}),patch.object(campaign,'network',return_value=context['network']),patch.object(campaign,'Sampler',FakeSampler),patch.object(campaign.subprocess,'run',side_effect=command),patch.object(campaign,'source_patch',return_value=0),patch.object(campaign,'optical',return_value={'receiver_visible_hz':60,'terminal_receiver_stall_ms':0,'screen_to_screen_transition_ms':{'median':20}}),patch.object(campaign,'quality',return_value={'global_optical_ssim':1}),patch.object(campaign,'snapshot',return_value={'status':'unavailable'}),patch.object(sys,'argv',['campaign','--product','extend','--output',str(output)]+options):
                with patch("builtins.print") as messages:campaign.main()
                messages.assert_any_call("Development diagnostic completed successfully." if diagnostic else "Benchmark completed successfully.",flush=True)
                if diagnostic:
                    data=json.loads((output/'report.json').read_text())
                    self.assertEqual(calls,['baseline','warmup','warmup','motion','warmup','scroll'])
                    self.assertEqual(len(focuses),2)
                    self.assertEqual(data['settings']['seconds'],6)
                    self.assertEqual(data['settings']['repetitions'],1)
                    self.assertEqual(data['settings']['campaign_profile'],'diagnostic')
                    self.assertEqual(data['settings']['scene_selection'],['motion','scroll'])
                    self.assertEqual(data['settings']['measurement_scope'],'development-only')
                    self.assertEqual(data['settings']['baseline_seconds'],5)
                    self.assertEqual(data['settings']['warmup_seconds'],5)
                    self.assertEqual(data['settings']['camera_exposure_compensation_steps'],-6)
                    self.assertIn('not a full benchmark',(output/'report.html').read_text())
                    original=data['phases'][0];data['status']='failed';data['phases']=data['phases'][:1]
                    campaign.write(output/'report.json',data);campaign.write(output/'manifest.json',{'artifacts':campaign.artifact_inventory(output)})
                    calls.clear();events.clear();expected_captures=1
                    with patch.object(sys,'argv',['campaign','--product','extend','--resume',str(output)]),patch('builtins.print'):
                        campaign.main()
                    resumed=json.loads((output/'report.json').read_text())
                    self.assertEqual(calls,['baseline','warmup','warmup','scroll'])
                    self.assertEqual(len(focuses),3)
                    self.assertEqual(resumed['phases'][0],original)
                    self.assertEqual(resumed['settings']['scene_selection'],['motion','scroll'])
                    self.assertEqual(resumed['settings']['seconds'],6)
                    self.assertEqual(resumed['settings']['camera_exposure_compensation_steps'],-6)
                    self.assertEqual(resumed['status'],'complete_with_declared_limits')
                    self.assertEqual(len(resumed['phases']),2)
                    return
                if quick:
                    data=json.loads((output/'report.json').read_text())
                    self.assertEqual(calls,['baseline','warmup','static','scroll','panel','motion','recovery'])
                    self.assertEqual(data['settings']['seconds'],30)
                    self.assertEqual(data['settings']['repetitions'],1)
                    self.assertEqual(data['settings']['campaign_profile'],'quick')
                    self.assertEqual(len(data['phases']),5)
                    self.assertIn('Quick development run',(output/'report.html').read_text())
                    opened.assert_called_once()
                    return
                initial_calls=list(calls)
                interrupted=json.loads((output/'report.json').read_text())
                interrupted['status']='failed';interrupted['phases']=interrupted['phases'][:9]
                campaign.write(output/'report.json',interrupted)
                campaign.write(output/'manifest.json',{'artifacts':campaign.artifact_inventory(output)})
                changed=json.loads(json.dumps(interrupted));changed['context']['source_display']['width']+=1
                with self.assertRaisesRegex(RuntimeError,'geometry differs'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['context']['receiver']['application']['executable_sha256']='changed'
                with self.assertRaisesRegex(RuntimeError,'build differs'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['settings']['camera_focus_regions']=[[.1,.1,.3,.3]]
                with self.assertRaisesRegex(RuntimeError,'camera focus regions differ'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['settings']['camera_focus_distance_range']=[1.8,2]
                with self.assertRaisesRegex(RuntimeError,'camera focus distance range differs'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['settings']['camera_zoom']=1.4
                with self.assertRaisesRegex(RuntimeError,'camera zoom'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['settings']['camera_exposure_compensation_steps']=-6
                with self.assertRaisesRegex(RuntimeError,'camera exposure compensation'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['settings']['cursor_input_method']='mouse-events'
                with self.assertRaisesRegex(RuntimeError,'cursor input method differs'):campaign.validate_resume(interrupted,changed,output)
                video=output/'01-static/camera/raw/video.mp4';original=video.read_bytes();video.write_bytes(b'corrupt')
                with self.assertRaisesRegex(RuntimeError,'integrity check failed'):campaign.validate_resume(interrupted,interrupted,output)
                video.write_bytes(original)
                calls.clear();events.clear();expected_captures=6
                with patch.object(sys,'argv',['campaign','--product','extend','--resume',str(output)]),patch('builtins.print'):
                    campaign.main()
                self.assertEqual(calls,['baseline','warmup','static','static','scroll','panel','motion','recovery'])
                resumed=json.loads((output/'report.json').read_text())
                self.assertEqual(resumed['phases'][:9],interrupted['phases'])
                self.assertEqual(resumed['resumptions'][0]['preserved_scenes'],9)
                self.assertEqual(len(list((output/'attempts').iterdir())),6)
                # Processing retry needs neither the source service nor a live product.
                retry=json.loads((output/'report.json').read_text());retry['status']='failed';retry['phases'][-1]['status']='captured'
                campaign.write(output/'report.json',retry);campaign.write(output/'manifest.json',{'artifacts':campaign.artifact_inventory(output)})
                calls.clear();events.clear();expected_captures=0
                from contextlib import ExitStack
                with ExitStack() as stack:
                    stack.enter_context(patch.object(campaign,'Client',side_effect=AssertionError('Source accessed during processing')))
                    stack.enter_context(patch.object(campaign,'application',side_effect=AssertionError('Product inspected during processing')))
                    stack.enter_context(patch.object(campaign,'select_phone',side_effect=AssertionError('Phone accessed for local footage')))
                    stack.enter_context(patch('builtins.print'))
                    campaign.process_existing(output,no_open=True)
                self.assertEqual(calls,[])
                # A phone-buffered checkpoint also processes without re-running a scene.
                retry=json.loads((output/'report.json').read_text());retry['status']='failed';retry['phases'][-1]['status']='captured'
                (output/'03-recovery/camera/summary.json').unlink();(output/'03-recovery/camera/raw/video.mp4').unlink()
                campaign.write(output/'report.json',retry);campaign.write(output/'manifest.json',{'artifacts':campaign.artifact_inventory(output)})
                events.clear()
                with ExitStack() as stack:
                    stack.enter_context(patch.object(campaign,'Client',side_effect=AssertionError('Source accessed during processing')))
                    stack.enter_context(patch.object(campaign,'application',side_effect=AssertionError('Product inspected during processing')))
                    stack.enter_context(patch('builtins.print'))
                    campaign.process_existing(output,no_open=True)
                self.assertEqual(calls,[])
                self.assertTrue(all(row['status']=='measured' for row in json.loads((output/'report.json').read_text())['phases']))
                calls[:]=initial_calls
            self.assertEqual(opened.call_count,2)
            opened.assert_called_with(output.resolve()/'report.html')
            self.assertIn('Benchmark completed successfully',(output/'report.html').read_text())
            data=json.loads((output/'report.json').read_text());manifest=json.loads((output/'manifest.json').read_text())
            self.assertEqual(data['status'],'complete_with_declared_limits')
            self.assertEqual(len(data['phases']),15)
            self.assertEqual(calls[:2],['baseline','warmup'])
            self.assertEqual(calls[7:12],['recovery','motion','panel','scroll','static'])
            self.assertIn('report.json',manifest['artifacts'])
            self.assertIn('01-static/camera/raw/video.mp4',manifest['artifacts'])
            self.assertTrue((output/'report.html').exists())
            from routes import reclassify
            original_video=(output/'01-static/camera/raw/video.mp4').read_bytes()
            classified=reclassify(output)
            self.assertTrue(all('route' in phase for phase in classified['phases']))
            self.assertEqual((output/'01-static/camera/raw/video.mp4').read_bytes(),original_video)
            self.assertEqual(len(list((output/'route-history').iterdir())),1)
