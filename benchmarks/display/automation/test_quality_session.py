import base64,json,struct,sys,tempfile,threading,unittest,uuid
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from quality_session import QualityController,validate_image


def png(width=2560,height=1440):
    return b'\x89PNG\r\n\x1a\n'+struct.pack('>I',13)+b'IHDR'+struct.pack('>II',width,height)+bytes(9)


class QualitySourceTests(unittest.TestCase):
    def test_upload_dimensions_and_encoding_are_bounded(self):
        self.assertEqual(validate_image(base64.b64encode(png()).decode()),png())
        for raw in [b'not png',png(16384,16384)]:
            with self.assertRaises(ValueError):validate_image(base64.b64encode(raw).decode())
        with self.assertRaises(ValueError):validate_image('!bad base64!')
        with self.assertRaises(ValueError):validate_image('x'*4_000_001)

    def test_source_only_uses_fixed_owned_capture_and_matching_stop_id(self):
        with tempfile.TemporaryDirectory() as directory:
            session=SimpleNamespace(root=Path(directory),tool=Path('/tools/campaign-workload'),lock=threading.Lock(),job=None,contexts={'extend':{}})
            controller=QualityController(session);ident=str(uuid.uuid4());calls=[]
            process=SimpleNamespace(poll=lambda:None,terminate=lambda:calls.append('terminate'),wait=lambda timeout:None)
            def spawn(argv,**kwargs):
                calls.append(argv)
                Path(argv[2]+'.json').write_text(json.dumps({'status':'ready'}))
                Path(argv[2]+'.png').write_bytes(png())
                return process
            body={'id':ident,'product':'extend','screen':'auto','image_base64':base64.b64encode(png()).decode()}
            with patch('quality_session.subprocess.Popen',side_effect=spawn):reply=controller.start(body)
            self.assertEqual(reply['id'],ident)
            self.assertEqual(calls[0][0],'/tools/quality-fixture')
            self.assertEqual(calls[0][-1],'auto')
            with self.assertRaises(ValueError):controller.stop({'id':str(uuid.uuid4())})
            self.assertNotIn('terminate',calls)
            controller.stop({'id':ident});self.assertIn('terminate',calls)

    def test_running_phase_blocks_quality_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            session=SimpleNamespace(root=Path(directory),tool=Path('/tools/workload'),lock=threading.Lock(),job={'status':'running'},contexts={'extend':{}})
            controller=QualityController(session)
            body={'id':str(uuid.uuid4()),'product':'extend','screen':'auto','image_base64':base64.b64encode(png()).decode()}
            with self.assertRaisesRegex(ValueError,'Another fixture'):controller.start(body)
