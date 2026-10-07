import importlib.util,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from metrics import optical,resources,quality,source_patch,network_delta,classify_route
from stages import summarize
from compare import comparable
from host import Session

class CampaignTests(unittest.TestCase):
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
            root=Path(root);tool=root/'tool';tool.write_text('#!/usr/bin/env python3\nimport sys,time,json\nstart=time.monotonic_ns(); print("ready",flush=True);time.sleep(3)\njson.dump({"valid_geometry":True,"duration_s":3,"start_ns":start,"end_ns":time.monotonic_ns(),"source_callback_hz":60},open(sys.argv[4],"w"))\n');tool.chmod(0o755)
            app={'executable_sha256':'test','pids':[os.getpid()],'main_pid':os.getpid(),'bundle_path':'/fake'}
            s=Session(root/'runs',tool,None);s.contexts['extend']={'application':app,'network':{'flows':[],'errors':[]},'displays':[]}
            row={'at_ns':1,'processes':[],'system_process_cpu_s':0,'collection_ms':1,'main_alive':True}
            with patch('host.sample',return_value=row),patch('host.network',return_value={'flows':[],'errors':[]}),patch('host.snapshot',return_value={'status':'unavailable'}),patch('host.application',return_value=app),patch.object(s,'displays',return_value=[]):
                ident=str(uuid.uuid4());s.start({'id':ident,'product':'extend','scene':'motion','seconds':3,'screen':'auto'})
                import time
                deadline=time.monotonic()+10
                while s.job['status']=='running' and time.monotonic()<deadline:time.sleep(.1)
                self.assertEqual(s.job['status'],'complete')
                self.assertTrue((root/'runs'/ident/'result.json').exists())

class CampaignOrchestrationTests(unittest.TestCase):
    def test_complete_campaign_writes_all_scenes_and_hashed_report(self):
        """Hardware seams are fake; real runner, scene order and report finalization execute."""
        import os,time,types
        import run as campaign
        with tempfile.TemporaryDirectory() as tmp:
            tmp=Path(tmp);output=tmp/'result';tools=tmp/'tools';tools.mkdir()
            fixture={'id':2,'builtin':False,'mirrored':False,'width':100,'height':100,'scale':1,'pixel_width':100,'pixel_height':100,'refresh_hz':60}
            app={'product':'extend','main_pid':os.getpid(),'executable_sha256':'test','version':'1','build':'1','display_helpers':[]}
            context={'application':app,'displays':[fixture],'host':{'model':'test'},'network':{'flows':[],'errors':[]}}
            calls=[]
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
                if '--activate' in argv:return types.SimpleNamespace(returncode=0)
                if 'getprop' in argv:return types.SimpleNamespace(stdout='synthetic phone')
                folder=Path(argv[argv.index('--output')+1]);folder.mkdir();(folder/'raw').mkdir()
                (folder/'patches.json').write_text('[]');(folder/'summary.json').write_text('{}');(folder/'raw/video.mp4').write_bytes(b'synthetic')
                ack=Path(argv[argv.index('--ack')+1]);ack.write_text(json.dumps({'request_start_ns':0,'ack_ns':0}))
                phase=json.loads(Path(argv[argv.index('--phase')+1]).read_text());client.request('/start',phase)
                return types.SimpleNamespace(returncode=0)
            with patch.object(campaign,'open_report') as opened,patch.object(campaign,'Client',return_value=client),patch.object(campaign,'build_swift',return_value=tools),patch.object(campaign,'application',return_value=app),patch.object(campaign,'load',return_value={}),patch.object(campaign,'save'),patch.object(campaign,'adb_at',return_value=Path('/fake/adb')),patch.object(campaign,'select_phone',return_value='test'),patch.object(campaign,'phone_identity',return_value='hash'),patch.object(campaign,'host',return_value={'model':'test'}),patch.object(campaign,'network',return_value=context['network']),patch.object(campaign,'Sampler',FakeSampler),patch.object(campaign.subprocess,'run',side_effect=command),patch.object(campaign,'source_patch',return_value=0),patch.object(campaign,'optical',return_value={'receiver_visible_hz':60,'terminal_receiver_stall_ms':0,'screen_to_screen_transition_ms':{'median':20}}),patch.object(campaign,'quality',return_value={'global_optical_ssim':1}),patch.object(campaign,'snapshot',return_value={'status':'unavailable'}),patch.object(sys,'argv',['campaign','--product','extend','--output',str(output)]):
                with patch("builtins.print") as messages:campaign.main()
                messages.assert_any_call("Benchmark completed successfully.",flush=True)
                initial_calls=list(calls)
                interrupted=json.loads((output/'report.json').read_text())
                interrupted['status']='failed';interrupted['phases']=interrupted['phases'][:9]
                campaign.write(output/'report.json',interrupted)
                campaign.write(output/'manifest.json',{'artifacts':campaign.artifact_inventory(output)})
                changed=json.loads(json.dumps(interrupted));changed['context']['source_display']['width']+=1
                with self.assertRaisesRegex(RuntimeError,'geometry differs'):campaign.validate_resume(interrupted,changed,output)
                changed=json.loads(json.dumps(interrupted));changed['context']['receiver']['application']['executable_sha256']='changed'
                with self.assertRaisesRegex(RuntimeError,'build differs'):campaign.validate_resume(interrupted,changed,output)
                video=output/'01-static/camera/raw/video.mp4';original=video.read_bytes();video.write_bytes(b'corrupt')
                with self.assertRaisesRegex(RuntimeError,'integrity check failed'):campaign.validate_resume(interrupted,interrupted,output)
                video.write_bytes(original)
                calls.clear()
                with patch.object(sys,'argv',['campaign','--product','extend','--resume',str(output)]),patch('builtins.print'):
                    campaign.main()
                self.assertEqual(calls,['baseline','warmup','static','static','scroll','panel','motion','recovery'])
                resumed=json.loads((output/'report.json').read_text())
                self.assertEqual(resumed['phases'][:9],interrupted['phases'])
                self.assertEqual(resumed['resumptions'][0]['preserved_scenes'],9)
                self.assertEqual(len(list((output/'attempts').iterdir())),6)
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
