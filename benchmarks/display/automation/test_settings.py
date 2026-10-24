"""Settings automation preserves phase isolation and native acknowledgment."""
import json,sys,tempfile,threading,time,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from host import Session

CONFIG={'mode':'extend','width':1728,'height':1085,'fps':60,'bitrate':20_000_000,'retina':True,'auto_size':False,'codec':'hevc','separate_cursor':True}

class SettingsTests(unittest.TestCase):
    def test_explicit_enable_and_bounds(self):
        with tempfile.TemporaryDirectory() as root:
            session=Session(root,'tool',None)
            with self.assertRaisesRegex(ValueError,'not started'):session.display_settings({'product':'extend','config':CONFIG})
            session.settings_file=Path(root)/'request.json'
            for invalid in [{**CONFIG,'width':3000},{**CONFIG,'fps':True},{**CONFIG,'retina':1},{**CONFIG,'unknown':True}]:
                with self.assertRaises(ValueError):session.display_settings({'product':'extend','config':invalid})
            self.assertFalse(session.settings_file.exists())

    def test_measured_scene_cannot_change_settings(self):
        with tempfile.TemporaryDirectory() as root:
            session=Session(root,'tool',None,Path(root)/'request.json');session.job={'status':'running'}
            with self.assertRaisesRegex(ValueError,'measured phase'):session.display_settings({'product':'extend','config':CONFIG})
            self.assertFalse(session.settings_file.exists())

    def test_native_acknowledgment_is_required(self):
        with tempfile.TemporaryDirectory() as root:
            path=Path(root)/'request.json';session=Session(root,'tool',None,path)
            def app():
                deadline=time.monotonic()+3
                while not path.exists() and time.monotonic()<deadline:time.sleep(.01)
                request=json.loads(path.read_text())
                path.with_suffix('.ack.json').write_text(json.dumps({'sequence':request['sequence'],'ok':True,'configuration':request['config']}))
            worker=threading.Thread(target=app);worker.start()
            with patch('host.application',return_value={'main_pid':42}):result=session.display_settings({'product':'extend','config':CONFIG})
            worker.join()
            self.assertTrue(result['ok']);self.assertEqual(result['configuration'],CONFIG)
            self.assertGreater(result['sequence'],0)

if __name__=='__main__':unittest.main()
