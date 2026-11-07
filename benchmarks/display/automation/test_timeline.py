import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
import timeline


class TimelineTests(unittest.TestCase):
    def scene(self,root):
        folder=root/'01-motion';camera=folder/'camera';camera.mkdir(parents=True)
        (camera/'summary.json').write_text(json.dumps({'video_metadata':{'nominal_playback_fps':30},'measured_sensor_hz':240}))
        rows=[]
        # 240 Hz camera, 60 Hz source counter; receiver shows each counter 25 ms later
        # and skips counter 40 entirely (one long stall).
        for sample in range(240*2):
            t=sample/240
            source=int(t*60)+1;receiver=int((t-.025)*60)+1 if t>=.025 else 0
            if receiver==40:receiver=39
            rows.append({'playback_s':t*8,'counter_ids':[source,receiver]})
        (camera/'optical-frames.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows))
        start=10**12
        stages={'receiver':{'window':{'start_ns':start},'numeric_events':[
                    {'stage':'decoder_submitted','frame_ns':1,'at_ns':start+int(.1e9)},{'stage':'decoded','frame_ns':1,'at_ns':start+int(.103e9)},
                    {'stage':'gpu_completed','frame_ns':1,'at_ns':start+int(.105e9)}]},
                'source':{'window':{'start_ns':start},'numeric_events':[
                    {'stage':'submitted','frame_ns':7,'at_ns':start+int(.3e9)},{'stage':'encoded','frame_ns':7,'at_ns':start+int(.315e9),'bytes':12500}]}}
        resources=[{'at_ns':start+int(i*.5e9),'processes':[{'pid':1,'cpu_s':i*.1,'rss_bytes':100*2**20}]} for i in range(5)]
        (folder/'receiver-resources.json').write_text(json.dumps(resources))
        (folder/'source.json').write_text(json.dumps({'stages':{'window':{'start_ns':start}},'rows':resources}))
        return folder,{'stages':stages}

    def test_bins_camera_stage_and_resource_series(self):
        with tempfile.TemporaryDirectory() as directory:
            folder,row=self.scene(Path(directory))
            built=timeline.build(folder,row,0,2)
            self.assertEqual(json.loads((folder/'timeline.json').read_text()),built)
            s=built['series'];self.assertEqual(built['bins'],8)
            self.assertAlmostEqual(s['source_updates_hz'][1],60,delta=4)
            self.assertTrue(all(abs(v-25)<5 for v in s['lag_ms_median'] if v is not None))
            self.assertGreater(max(v for v in s['receiver_longest_gap_ms'] if v),30)
            self.assertEqual(s['decode_ms'][0],3.0);self.assertEqual(s['render_ms'][0],2.0)
            self.assertEqual(s['encode_ms'][1],15.0);self.assertEqual(s['encoded_mbps'][1],0.4)
            self.assertEqual(built['resources']['receiver'][0],[0.5,20.0,100.0])

    def test_report_section_renders_charts_and_skips_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);folder,row=self.scene(root);timeline.build(folder,row,0,2)
            report={'settings':{'seconds':2},'phases':[{'repetition':1,'scene':'motion'},{'repetition':1,'scene':'scroll'}]}
            page=timeline.report_html(root,report)
            self.assertEqual(page.count('class="tl-scene"'),1)
            self.assertIn('Screen-to-screen lag',page);self.assertIn('01-motion/timeline.json',page)
            self.assertEqual(timeline.report_html(root,{'settings':{'seconds':2},'phases':[]}),'')
