import json, subprocess, sys, tempfile, time, unittest
from pathlib import Path
from preparation import Preparation

class PreparationLifecycleTests(unittest.TestCase):
    def make(self, root, code):
        path=Path(root)/'command.json';path.write_text(json.dumps([sys.executable,'-u','-c',code]))
        return Preparation(path,root)

    def test_ready_and_stop_precede_measured_command(self):
        with tempfile.TemporaryDirectory() as root:
            prep=self.make(root,"import time; print('ready',flush=True); time.sleep(60)")
            try:
                prep.start(timeout=2);child=prep.process
                prep.stop('camera_ready_before_measured_scene',require_live=True)
                measured_ns=time.monotonic_ns()
                subprocess.run([sys.executable,'-c','pass'],check=True)
                receipt=json.loads((Path(root)/'preparation.json').read_text())
                self.assertLess(receipt['ready_ns'],receipt['stopped_ns'])
                self.assertLess(receipt['stopped_ns'],measured_ns)
                self.assertEqual(receipt['status'],'intentionally_terminated')
                self.assertIsNotNone(child.returncode)
            finally:prep.stop()

    def test_timeout_and_camera_error_cleanup_owned_child(self):
        for ready in (False,True):
            with self.subTest(ready=ready),tempfile.TemporaryDirectory() as root:
                prep=self.make(root,"import time; "+("print('ready',flush=True); " if ready else "")+"time.sleep(60)")
                child=None
                try:
                    with self.assertRaises(RuntimeError if ready else TimeoutError):
                        try:
                            child=None
                            prep.start(timeout=.2)
                            raise RuntimeError('Camera failed before readiness')
                        finally:
                            child=prep.process
                            prep.stop()
                    self.assertIsNotNone(child.returncode)
                    self.assertEqual(json.loads((Path(root)/'preparation.json').read_text())['stop_reason'],'cleanup')
                finally:prep.stop()

    def test_premature_exit_cannot_start_measured_scene(self):
        with tempfile.TemporaryDirectory() as root:
            prep=self.make(root,"import time; print('ready',flush=True); time.sleep(.1)")
            try:
                prep.start(timeout=1)
                prep.process.wait(timeout=1)
                with self.assertRaises(RuntimeError):prep.stop('camera_ready_before_measured_scene',require_live=True)
            finally:prep.stop()

    def test_invalid_command_never_launches(self):
        with tempfile.TemporaryDirectory() as root:
            path=Path(root)/'command.json';path.write_text('["sh", null]')
            with self.assertRaises(ValueError):Preparation(path,root)
