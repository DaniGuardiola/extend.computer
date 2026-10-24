"""Settings diagnostics never record an unacknowledged mode or skip restoration."""
import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from sweep import apply,execute,mode_plan
from settings import configuration

class Client:
    def __init__(self,mismatch=False):self.calls=[];self.mismatch=mismatch
    def request(self,path,body):
        self.calls.append(body['config'])
        config={**body['config']}
        if self.mismatch and len(self.calls)==1:config['fps']=30
        return {'ok':True,'configuration':config}

class SweepTests(unittest.TestCase):
    def test_modes_change_one_declared_axis(self):
        plan=mode_plan('1728x1085@2',['responsive','detail','30fps','120fps','native','compact'])
        self.assertEqual([r['configuration']['bitrate'] for r in plan[:2]],[8_000_000,40_000_000])
        self.assertEqual([r['configuration']['fps'] for r in plan[2:4]],[30,120])
        self.assertEqual(plan[4]['profile'],'1728x1085@1')
        self.assertEqual(plan[5]['profile'],'1280x800@2')
        for modes in [[],['balanced','balanced'],['unknown']]:
            with self.assertRaises(ValueError):mode_plan('1728x1085@2',modes)

    def test_ack_mismatch_prevents_recording_and_restores_baseline(self):
        with tempfile.TemporaryDirectory() as parent:
            client=Client(mismatch=True);calls=[];root=Path(parent)/'run';baseline=configuration('1728x1085@2')
            with self.assertRaisesRegex(RuntimeError,'acknowledge'):
                execute(client,root,mode_plan('1728x1085@2',['detail']),baseline,['motion'],10,Path('pairing'),runner=lambda *a,**k:calls.append(a))
            self.assertFalse(calls);self.assertEqual(client.calls[-1],baseline)
            report=json.loads((root/'sweep.json').read_text())
            self.assertEqual(report['status'],'failed');self.assertIn('acknowledged',report['restoration'])

    def test_campaign_failure_stops_other_modes_and_restores(self):
        with tempfile.TemporaryDirectory() as parent:
            client=Client();root=Path(parent)/'run';baseline=configuration('1728x1085@2')
            def fail(*args,**kwargs):raise RuntimeError('camera validation failed')
            with self.assertRaisesRegex(RuntimeError,'camera validation'):
                execute(client,root,mode_plan('1728x1085@2',['detail','responsive']),baseline,['motion'],10,Path('pairing'),runner=fail)
            self.assertEqual(len(client.calls),2);self.assertEqual(client.calls[-1],baseline)
            self.assertEqual(len(json.loads((root/'sweep.json').read_text())['runs']),1)

    def test_separate_output_geometry_and_settings_are_preserved(self):
        with tempfile.TemporaryDirectory() as parent:
            client=Client();root=Path(parent)/'run';baseline=configuration('1728x1085@2');calls=[]
            def record(argv,**kwargs):
                calls.append(argv);folder=Path(argv[argv.index('--output')+1]);folder.mkdir()
                (folder/'report.json').write_text(json.dumps({'phases':[{'scene':'scroll','status':'measured','display_configuration':client.calls[-1]}]}))
            result=execute(client,root,mode_plan('1728x1085@2',['balanced','native']),baseline,['scroll'],10,Path('pairing'),runner=record)
            self.assertEqual(result['status'],'complete');self.assertEqual(client.calls[-1],baseline)
            self.assertIn('1728x1085@1',calls[1]);self.assertNotEqual(calls[0][calls[0].index('--output')+1],calls[1][calls[1].index('--output')+1])
            self.assertTrue((root/'report.html').exists())

    def test_observed_wrong_mode_fails_even_after_acknowledgment(self):
        with tempfile.TemporaryDirectory() as parent:
            client=Client();root=Path(parent)/'run';baseline=configuration('1728x1085@2')
            def record(argv,**kwargs):
                folder=Path(argv[argv.index('--output')+1]);folder.mkdir()
                (folder/'report.json').write_text(json.dumps({'phases':[{'scene':'motion','status':'measured','display_configuration':baseline}]}))
            with self.assertRaisesRegex(RuntimeError,'configuration.*bitrate'):
                execute(client,root,mode_plan('1728x1085@2',['detail','responsive']),baseline,['motion'],10,Path('pairing'),runner=record)
            self.assertEqual(client.calls[-1],baseline)
            report=json.loads((root/'sweep.json').read_text())
            self.assertEqual(report['status'],'failed');self.assertEqual(len(report['runs']),1)
if __name__=='__main__':unittest.main()
