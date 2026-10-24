"""Application launch identity and process discovery regression checks."""
import importlib.util,sys,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parent))
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
import launch

class LaunchTests(unittest.TestCase):
    def test_buffered_presentation_requires_pacing_and_is_opt_in(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run') as run:
            with self.assertRaisesRegex(ValueError,'requires paced'):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),buffered_render=True)
            run.assert_not_called()
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),paced_render=True,buffered_render=enabled)
            self.assertEqual('EXTEND_DISPLAY_BUFFERED_RENDER=1' in run.call_args.args[0],enabled)

    def test_context_only_retains_context_without_per_frame_instrumentation(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
            launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),context_only=True)
        argv=run.call_args.args[0]
        self.assertIn('EXTEND_DISPLAY_BENCHMARK_CONTEXT=1',argv)
        self.assertFalse(any(value.startswith('EXTEND_DISPLAY_METRICS=') for value in argv))
        self.assertFalse(any(value.startswith('EXTEND_DISPLAY_TRANSPORT_TIMING=') for value in argv))
        for options in ({'observe_marker':True},{'observe_transport':True}):
            with patch.object(launch.subprocess,'run') as run:
                with self.assertRaisesRegex(ValueError,'Context-only'):
                    launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),context_only=True,**options)
                run.assert_not_called()

    def test_context_only_reports_missing_stages_instead_of_zero_performance(self):
        import json,tempfile,bootstrap,stages
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'.cache').mkdir()
            (root/'.cache/extend-session.json').write_text(json.dumps({'pid':42,'diagnostics':{'stage_timing':False}}))
            with patch.object(bootstrap,'ROOT',root),patch.object(stages,'read_numeric') as read:
                result=stages.snapshot({'product':'extend','main_pid':42},None,0,100)
            read.assert_not_called()
            self.assertEqual(result['status'],'unavailable')
            self.assertNotIn('unique_stage_hz',result)

    def test_reordering_deadline_is_explicit_and_invalid_values_never_launch(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for deadline in (20,100,500,1000):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),reordering_ms=deadline)
            self.assertIn('EXTEND_DISPLAY_REORDERING_MS='+str(deadline),run.call_args.args[0])
        for deadline in (0,19,1001,-1,100.5,True,'fast'):
            with patch.object(launch.subprocess,'run') as run:
                with self.assertRaises(launch.argparse.ArgumentTypeError):
                    launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),reordering_ms=deadline)
                run.assert_not_called()

    def test_nv12_transfer_control_is_explicit(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),nv12_srgb_input=enabled)
            self.assertIn('EXTEND_DISPLAY_NV12_SRGB_INPUT='+('1' if enabled else '0'),run.call_args.args[0])

    def test_rgb_capture_is_opt_in(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),capture_bgra=enabled)
            self.assertEqual('EXTEND_DISPLAY_CAPTURE_BGRA=1' in run.call_args.args[0],enabled)

    def test_low_latency_encoder_is_opt_in(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),low_latency_encoder=enabled)
            self.assertEqual('EXTEND_DISPLAY_LOW_LATENCY_ENCODER=1' in run.call_args.args[0],enabled)

    def test_fit_display_uses_negotiation_without_claiming_fixed_geometry(self):
        self.assertEqual(launch.connection_arguments('account','1728x1085@2',60,fit_display=True)[-3:],
                         ['auto','60','20000000'])
        with self.assertRaisesRegex(ValueError, 'Retina'):
            launch.connection_arguments('account','1728x1085@1',60,fit_display=True)

    def test_settings_profiles_launch_real_configuration(self):
        for quality,bitrate in [('responsive',8_000_000),('balanced',20_000_000),('detail',40_000_000)]:
            for fps in (30,60,120):
                args=launch.connection_arguments('account','1728x1085@2',3600,fps,quality)
                self.assertEqual(args[-3:],['retina',str(fps),str(bitrate)])
        self.assertEqual(launch.connection_arguments('account','1920x1080@1',60)[-3:],['native','60','20000000'])
        for profile in ['3840x2160@2','100x100@1','1728x1085@3']:
            with self.assertRaises(ValueError):launch.connection_arguments('account',profile,60)

    def test_diagnostic_flags_are_explicit_launch_environment(self):
        app=Path('/tmp/Prepared App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
            launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),True,True)
        argv=run.call_args.args[0]
        self.assertIn('EXTEND_DISPLAY_BENCHMARK_MARKER=1',argv)
        self.assertIn('EXTEND_DISPLAY_TRANSPORT_TIMING=1',argv)

    def test_notification_reference_is_explicitly_requested(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
            launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),notification_render=True)
        self.assertIn('EXTEND_DISPLAY_EXPLICIT_RENDER=0',run.call_args.args[0])

    def test_video_priority_is_opt_in_and_recorded_in_launch_environment(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),video_qos=enabled)
            self.assertEqual('EXTEND_DISPLAY_DSCP=34' in run.call_args.args[0],enabled)

    def test_responsive_service_is_opt_in(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        for enabled in (False,True):
            with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':42}]):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'),responsive_service=enabled)
            self.assertEqual('EXTEND_DISPLAY_MACOS_RESPONSIVE=1' in run.call_args.args[0],enabled)

    def test_launch_services_tracks_app_not_launcher(self):
        app=Path('/tmp/Prepared App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run') as run,patch.object(launch,'processes',side_effect=[[],[{'path':str(exe),'pid':42}]]),patch.object(launch.time,'sleep'):
            pid=launch.launch_app(app,exe,['--display-benchmark','account','1728','1085','3600','retina'],Path('/tmp/metrics.jsonl'),Path('/tmp/session.log'))
        self.assertEqual(pid,42)
        argv=run.call_args.args[0]
        self.assertEqual(argv[:4],['/usr/bin/open','-n','-a',str(app)])
        self.assertIn('EXTEND_DISPLAY_METRICS=/tmp/metrics.jsonl',argv)
        self.assertEqual(argv[argv.index('--args')+1:],['--display-benchmark','account','1728','1085','3600','retina'])
    def test_ambiguous_app_launch_is_rejected(self):
        app=Path('/tmp/App.app');exe=app/'Contents/MacOS/app'
        with patch.object(launch.subprocess,'run'),patch.object(launch,'processes',return_value=[{'path':str(exe),'pid':1},{'path':str(exe),'pid':2}]):
            with self.assertRaisesRegex(RuntimeError,'Multiple prepared'):
                launch.launch_app(app,exe,[],Path('/tmp/metrics'),Path('/tmp/log'))

if __name__=='__main__':unittest.main()
