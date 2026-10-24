"""Sustained observations must retain changes and enforce bounded ownership."""
import sys, tempfile, unittest
from pathlib import Path
from unittest.mock import patch
sys.path[:0]=[str(Path(__file__).resolve().parents[1]/'campaign'),str(Path(__file__).resolve().parent)]
from soak import resource_windows
from host import Session

class SoakTests(unittest.TestCase):
    def test_time_windows_preserve_late_resource_changes(self):
        rows=[{'at_ns':i*10_000_000_000,'cpu':i} for i in range(6)]
        with patch('soak.resources',side_effect=lambda group:{'last_cpu':group[-1]['cpu']}):
            result=resource_windows(rows)
        self.assertEqual([x['start_offset_s'] for x in result],[0,30])
        self.assertEqual([x['resources']['last_cpu'] for x in result],[2,5])

    def test_insufficient_samples_are_not_a_success(self):
        with self.assertRaisesRegex(RuntimeError,'too few'):resource_windows([])

    def test_only_motion_accepts_longer_bounded_phase(self):
        with tempfile.TemporaryDirectory() as path:
            session=Session(Path(path),Path('/unused'),None)
            base={'id':'019ca200-0000-7000-8000-000000000001','product':'extend','screen':'auto'}
            # No context: a valid request reaches the readiness gate without
            # launching a fixture. Invalid durations fail before that gate.
            with self.assertRaisesRegex(ValueError,'Preflight context required'):
                session.start({**base,'scene':'motion','seconds':600})
            for scene,seconds in [('motion',601),('recovery',61),('motion',True)]:
                with self.subTest(scene=scene,seconds=seconds),self.assertRaisesRegex(ValueError,'Invalid phase'):
                    session.start({**base,'scene':scene,'seconds':seconds})

if __name__=='__main__':unittest.main()
