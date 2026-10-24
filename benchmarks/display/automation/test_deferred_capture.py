import json,sys,tempfile,unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import capture_phone

class DeferredCaptureTests(unittest.TestCase):
    def test_capture_then_collect_without_another_recording(self):
        self.run_capture_and_collect()
    def test_failed_transfer_preserves_phone_recording(self):
        self.run_capture_and_collect(fail_transfer=True)
    def test_collection_preserves_nondefault_exposure_profile(self):
        self.run_capture_and_collect(compensation=-6)
    def run_capture_and_collect(self,fail_transfer=False,compensation=0):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp)/'camera';calls=[];run_id=None;receipt=None
            def command(argv,**kwargs):
                nonlocal run_id,receipt
                calls.append(argv)
                if 'start' in argv:
                    run_id=argv[argv.index('run_id')+1]
                    receipt={'run_id':run_id,'status':'complete','requested_capture_fps':120,'width':1920,'height':1080}
                    receipt['requested_orientation']=argv[argv.index('camera_orientation')+1]
                    receipt['requested_zoom']=capture_phone.camera_zoom(argv[argv.index('zoom')+1])
                    receipt['requested_exposure_compensation_steps']=compensation
                    self.assertEqual(int(argv[argv.index('exposure_compensation_steps')+1]),compensation)
                data=b''
                if 'cat' in argv:
                    name=argv[-1].rsplit('/',1)[-1]
                    if name=='ready.json':data=b'{"status":"recording"}'
                    elif name=='result.json':data=json.dumps(receipt).encode()
                    elif name=='sensor.jsonl':data=b'\n'.join(json.dumps({'sensor_ns':i*8333333}).encode() for i in range(120))
                    elif name=='video.mp4':
                        if fail_transfer:raise RuntimeError('Transfer interrupted')
                        data=b'synthetic footage'
                if 'getprop' in argv:data=b'test'
                return SimpleNamespace(stdout=data)
            argv=['capture','--serial','test','--output',str(folder),'--seconds','1','--fps','120','--quiet']
            with patch.object(capture_phone.subprocess,'run',side_effect=command),patch('builtins.print'):
                with patch.object(sys,'argv',argv+['--capture-only','--exposure-compensation-steps',str(compensation)]):capture_phone.main()
                self.assertTrue((folder/'phone-session.json').exists());self.assertTrue((folder/'capture.json').exists())
                self.assertFalse(any('cat' in c and c[-1].endswith('video.mp4') for c in calls))
                self.assertFalse(any('rm' in c for c in calls))
                calls.clear()
                with patch.object(sys,'argv',argv+['--collect-existing']):
                    if fail_transfer:
                        with self.assertRaisesRegex(RuntimeError,'Transfer interrupted'):capture_phone.main()
                    else:capture_phone.main()
                self.assertFalse(any('start' in c for c in calls))
                if fail_transfer:
                    self.assertFalse(any('rm' in c for c in calls))
                    self.assertFalse((folder/'summary.json').exists())
                else:
                    self.assertEqual((folder/'raw/video.mp4').read_bytes(),b'synthetic footage')
                    self.assertEqual(json.loads((folder/'summary.json').read_text())['capture_fps'],120)
                    self.assertEqual(json.loads((folder/'summary.json').read_text())['capture_profile']['requested_exposure_compensation_steps'],compensation)
                    self.assertTrue(any('rm' in c and c[-1]=='files/'+run_id for c in calls))
