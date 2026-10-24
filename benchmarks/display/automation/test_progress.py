import io,os,sys,unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from progress import Progress
from run import campaign_mode,recordings_finished

class ProgressTests(unittest.TestCase):
    def test_settings_sweep_does_not_release_devices_between_modes(self):
        stream=io.StringIO()
        with patch.dict(os.environ,{'EXTEND_DISPLAY_SETTINGS_SWEEP':'1'}):
            recordings_finished(Progress(stream))
        self.assertIn('Keep both Macs and phone idle',stream.getvalue())
        self.assertNotIn('free to use',stream.getvalue())

    def test_standalone_campaign_releases_devices_after_recording(self):
        stream=io.StringIO()
        with patch.dict(os.environ,{},clear=True):
            recordings_finished(Progress(stream))
        self.assertIn('free to use',stream.getvalue())
    def test_interactive_terminal_uses_color(self):
        stream=io.StringIO();stream.isatty=lambda:True
        with patch.dict(os.environ,{'TERM':'xterm-256color'},clear=True):
            ui=Progress(stream)
            with ui.step('recording'):pass
        self.assertIn('\033[36m',stream.getvalue());self.assertIn('\033[32m',stream.getvalue())
    def test_redirected_output_has_no_terminal_escapes(self):
        stream=io.StringIO();ui=Progress(stream)
        ui.heading('extend','quick');ui.scene(1,5,'static',1,1)
        with ui.step('recording'):pass
        output=stream.getvalue()
        self.assertNotIn('\033',output);self.assertNotIn('\r',output)
        self.assertIn('01/05',output);self.assertIn('✓  recording',output)
    def test_no_color_and_failure(self):
        stream=io.StringIO();stream.isatty=lambda:True
        with patch.dict(os.environ,{'NO_COLOR':'1'}):
            ui=Progress(stream)
            with self.assertRaises(RuntimeError),ui.step('transfer'):raise RuntimeError('failed')
        self.assertNotIn('\033',stream.getvalue());self.assertIn('×  transfer',stream.getvalue())
    def test_quick_resume_inherits_profile(self):
        args=SimpleNamespace(smoke=False,quick=False,repetitions=None)
        self.assertEqual(campaign_mode(args,{'settings':{'campaign_profile':'quick'}}),'quick')
        self.assertEqual(args.repetitions,1)
    def test_quick_cannot_change_full_resume(self):
        args=SimpleNamespace(smoke=False,quick=True,repetitions=None)
        with self.assertRaises(ValueError):campaign_mode(args,{'settings':{}})
    def test_quick_rejects_multiple_repetitions(self):
        with self.assertRaises(ValueError):campaign_mode(SimpleNamespace(smoke=False,quick=True,repetitions=3))
