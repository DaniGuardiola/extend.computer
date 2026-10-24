import json
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'campaign'))
import launch


class LaunchDiagnosticTests(unittest.TestCase):
    def test_low_latency_environment_preserves_explicit_controls_and_auto(self):
        executable=Path('/fixture/extend.computer.app/Contents/MacOS/extend')
        for enabled,automatic,value in [(False,False,'0'),(True,False,'1'),(False,True,None)]:
            with self.subTest(enabled=enabled,automatic=automatic),patch.object(launch.subprocess,'run') as run, \
                 patch.object(launch,'processes',return_value=[{'path':str(executable),'pid':123}]):
                launch.launch_app(executable.parents[2],executable,[],Path('/fixture/trace.jsonl'),
                    Path('/fixture/launch.log'),low_latency_encoder=enabled,auto_low_latency=automatic)
                selected=[x for x in run.call_args.args[0] if x.startswith('EXTEND_DISPLAY_LOW_LATENCY_ENCODER=')]
                self.assertEqual(selected,[] if value is None else ['EXTEND_DISPLAY_LOW_LATENCY_ENCODER='+value])
        with patch.object(launch.subprocess,'run') as run:
            with self.assertRaises(ValueError):
                launch.launch_app(executable.parents[2],executable,[],Path('/fixture/trace.jsonl'),
                    Path('/fixture/launch.log'),low_latency_encoder=True,auto_low_latency=True)
            run.assert_not_called()

    def test_auto_low_latency_cli_records_requested_policy_without_actual_claim(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);app=root/'extend.computer.app';executable=app/'Contents/MacOS/extend'
            executable.parent.mkdir(parents=True);executable.touch()
            (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'extend'}))
            with patch.object(launch,'ROOT',root),patch.object(launch,'processes',return_value=[]), \
                 patch.object(launch,'launch_app',return_value=123) as start, \
                 patch.object(sys,'argv',['launch','--app',str(app),'--auto-low-latency']):
                launch.main()
            self.assertTrue(start.call_args.kwargs['auto_low_latency'])
            state=json.loads((root/'.cache/extend-session.json').read_text())
            self.assertEqual(state['diagnostics']['low_latency_encoder_policy'],'auto')
            self.assertIsNone(state['diagnostics']['low_latency_encoder'])
        with patch.object(sys,'argv',['launch','--auto-low-latency','--low-latency-encoder']), \
             patch.object(launch,'launch_app') as start,patch.object(sys,'stderr'):
            with self.assertRaises(SystemExit):launch.main()
            start.assert_not_called()

    def test_connection_codec_preserves_default_and_rejects_unknown_choice(self):
        legacy=['--display-benchmark','peer','1728','1085','60','retina','60','20000000']
        self.assertEqual(launch.connection_arguments('peer','1728x1085@2',60), legacy)
        self.assertEqual(launch.connection_arguments('peer','1728x1085@2',60,codec='hevc'), legacy)
        self.assertEqual(launch.connection_arguments('peer','1728x1085@2',60,codec='h264'), legacy+['h264'])
        with self.assertRaises(ValueError):
            launch.connection_arguments('peer','1728x1085@2',60,codec='unknown')

    def test_fixed_bitrate_environment_is_explicit_opt_in(self):
        for enabled in (False, True):
            with self.subTest(enabled=enabled):
                executable = Path('/fixture/extend.computer.app/Contents/MacOS/extend')
                with patch.object(launch.subprocess, 'run') as run, patch.object(
                    launch, 'processes', return_value=[{'path': str(executable), 'pid': 123}]
                ):
                    pid = launch.launch_app(executable.parents[2], executable, [],
                        Path('/fixture/trace.jsonl'), Path('/fixture/launch.log'), fixed_bitrate=enabled)
                command = run.call_args.args[0]
                self.assertEqual(pid, 123)
                self.assertEqual('EXTEND_DISPLAY_FIXED_BITRATE=1' in command, enabled)

    def test_fixed_bitrate_cli_records_launch_context(self):
        for enabled in (False, True):
            with self.subTest(enabled=enabled), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                app = root / 'extend.computer.app'
                executable = app / 'Contents/MacOS/extend'
                executable.parent.mkdir(parents=True)
                executable.touch()
                (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable': 'extend'}))
                argv = ['launch', '--app', str(app)] + (['--fixed-bitrate'] if enabled else [])
                with patch.object(launch, 'ROOT', root), patch.object(launch, 'processes', return_value=[]), \
                     patch.object(launch, 'launch_app', return_value=123) as start, patch.object(sys, 'argv', argv):
                    launch.main()
                self.assertIs(start.call_args.kwargs['fixed_bitrate'], enabled)
                state = json.loads((root / '.cache/extend-session.json').read_text())
                self.assertIs(state['diagnostics']['fixed_bitrate'], enabled)

    def test_kernel_arrival_environment_is_explicit_opt_in(self):
        executable = Path('/fixture/extend.computer.app/Contents/MacOS/extend')
        for enabled,automatic,value in [(False,False,'0'),(True,False,'1'),(False,True,None)]:
            with self.subTest(enabled=enabled,automatic=automatic), patch.object(launch.subprocess, 'run') as run, \
                 patch.object(launch, 'processes', return_value=[{'path': str(executable), 'pid': 123}]):
                launch.launch_app(executable.parents[2], executable, [], Path('/fixture/trace.jsonl'),
                    Path('/fixture/launch.log'), kernel_arrival=enabled,auto_kernel_arrival=automatic)
                selected=[x for x in run.call_args.args[0] if x.startswith('EXTEND_DISPLAY_KERNEL_ARRIVAL=')]
                self.assertEqual(selected,[] if value is None else ['EXTEND_DISPLAY_KERNEL_ARRIVAL='+value])
        with patch.object(launch.subprocess,'run') as run:
            with self.assertRaises(ValueError):
                launch.launch_app(executable.parents[2],executable,[],Path('/fixture/trace.jsonl'),
                    Path('/fixture/launch.log'),kernel_arrival=True,auto_kernel_arrival=True)
            run.assert_not_called()

    def test_auto_kernel_cli_records_requested_policy_without_actual_claim(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);app=root/'extend.computer.app';executable=app/'Contents/MacOS/extend'
            executable.parent.mkdir(parents=True);executable.touch()
            (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'extend'}))
            with patch.object(launch,'ROOT',root),patch.object(launch,'processes',return_value=[]), \
                 patch.object(launch,'launch_app',return_value=123) as start, \
                 patch.object(sys,'argv',['launch','--app',str(app),'--auto-kernel-arrival']):
                launch.main()
            self.assertTrue(start.call_args.kwargs['auto_kernel_arrival'])
            state=json.loads((root/'.cache/extend-session.json').read_text())
            self.assertEqual(state['diagnostics']['kernel_arrival_policy'],'auto')
            self.assertIsNone(state['diagnostics']['kernel_arrival'])
        with patch.object(sys,'argv',['launch','--auto-kernel-arrival','--kernel-arrival']), \
             patch.object(launch,'launch_app') as start,patch.object(sys,'stderr'):
            with self.assertRaises(SystemExit):launch.main()
            start.assert_not_called()

    def test_kernel_arrival_cli_records_independent_launch_context(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            app = root / 'extend.computer.app'
            executable = app / 'Contents/MacOS/extend'
            executable.parent.mkdir(parents=True)
            executable.touch()
            (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable': 'extend'}))
            with patch.object(launch, 'ROOT', root), patch.object(launch, 'processes', return_value=[]), \
                 patch.object(launch, 'launch_app', return_value=123) as start, \
                 patch.object(sys, 'argv', ['launch', '--app', str(app), '--kernel-arrival']):
                launch.main()
            self.assertTrue(start.call_args.kwargs['kernel_arrival'])
            self.assertFalse(start.call_args.kwargs['fixed_bitrate'])
            state = json.loads((root / '.cache/extend-session.json').read_text())
            self.assertTrue(state['diagnostics']['kernel_arrival'])
            self.assertFalse(state['diagnostics']['fixed_bitrate'])

    def test_connection_codec_cli_records_requested_choice(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);app=root/'extend.computer.app';executable=app/'Contents/MacOS/extend'
            executable.parent.mkdir(parents=True);executable.touch()
            (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'extend'}))
            argv=['launch','--app',str(app),'--connect','peer','--display-profile','1728x1085@2','--codec','h264']
            with patch.object(launch,'ROOT',root),patch.object(launch,'processes',return_value=[]), \
                 patch.object(launch,'launch_app',return_value=123) as start,patch.object(sys,'argv',argv):
                launch.main()
            self.assertEqual(start.call_args.args[2][-1],'h264')
            state=json.loads((root/'.cache/extend-session.json').read_text())
            self.assertEqual(state['requested_display']['codec'],'h264')


if __name__ == '__main__':
    unittest.main()
