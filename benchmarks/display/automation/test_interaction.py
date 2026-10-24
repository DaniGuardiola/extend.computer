import json
import sys
import tempfile
import threading
import unittest
import uuid
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'campaign'))
from interaction_session import ACTIONS, InteractionController
from interaction import execute


class InteractionTests(unittest.TestCase):
    def make(self, root, busy=False):
        idle = SimpleNamespace(active=lambda: False)
        session = SimpleNamespace(root=Path(root), tool=Path(root) / 'campaign-workload',
            lock=threading.Lock(), quality=idle, cursor=idle,
            job={'status': 'running'} if busy else None, contexts={'extend': {'application': 'owned'}})
        (Path(root) / 'interaction-fixture').write_bytes(b'fixture')
        return InteractionController(session)

    def test_request_schema_preflight_and_concurrency(self):
        with tempfile.TemporaryDirectory() as root:
            controller = self.make(root, busy=True)
            body = {'id': str(uuid.uuid4()), 'product': 'extend', 'screen': 'auto'}
            with self.assertRaisesRegex(ValueError, 'Another fixture'):
                controller.start(body)
            with self.assertRaisesRegex(ValueError, 'Unexpected'):
                controller.start({**body, 'command': 'ignored'})
            controller.session.job = None
            with self.assertRaisesRegex(ValueError, 'Preflight'):
                controller.start({**body, 'product': 'missing'})

    def test_failed_exit_cannot_be_green_and_all_outcomes_required(self):
        with tempfile.TemporaryDirectory() as root:
            controller = self.make(root)
            process = SimpleNamespace(poll=lambda: 1, returncode=1)
            body = {'id': str(uuid.uuid4()), 'product': 'extend', 'screen': 'auto'}
            with patch('interaction_session.subprocess.Popen', return_value=process):
                controller.start(body)
            evidence = {'status': 'passed', 'cursor_restored': True,
                'actions': [{'action': action, 'accepted': True} for action in ACTIONS]}
            controller.output.write_text(json.dumps(evidence))
            self.assertEqual(controller.state({'id': body['id']})['status'], 'failed')
            process.returncode = 0
            self.assertEqual(controller.state({'id': body['id']})['status'], 'complete')
            evidence['actions'].pop()
            controller.output.write_text(json.dumps(evidence))
            self.assertEqual(controller.state({'id': body['id']})['status'], 'failed')
            with self.assertRaisesRegex(ValueError, 'ID mismatch'):
                controller.stop({'id': str(uuid.uuid4())})

    def test_watchdog_owns_only_current_process_and_stops_without_client(self):
        with tempfile.TemporaryDirectory() as root:
            controller = self.make(root)
            calls = []
            process = SimpleNamespace(returncode=None)
            process.poll = lambda: process.returncode
            process.terminate = lambda: calls.append('terminate')
            def wait(timeout): process.returncode = 1
            process.wait = wait
            body = {'id': str(uuid.uuid4()), 'product': 'extend', 'screen': 'auto'}
            with patch('interaction_session.subprocess.Popen', return_value=process):
                controller.start(body)
            controller.expire(str(uuid.uuid4()), process)
            self.assertEqual(calls, [])
            controller.expire(body['id'], process)
            self.assertEqual(calls, ['terminate'])
            result = controller.state({'id': body['id']})
            self.assertEqual(result['status'], 'failed')
            self.assertIn('deadline', result['error'])

    def test_client_failure_preserves_report_and_requests_cleanup(self):
        with tempfile.TemporaryDirectory() as root:
            calls = []
            def request(path, body=None):
                calls.append(path)
                if path == '/interaction/start': return {'id': body['id'], 'status': 'running'}
                if path == '/interaction/state': return {'id': body['id'], 'status': 'failed', 'error': 'Focus changed'}
                return {'status': 'failed'}
            output = Path(root) / 'run'
            with self.assertRaisesRegex(RuntimeError, 'Focus changed'):
                execute(SimpleNamespace(request=request), 'extend', output)
            self.assertEqual(calls[-1], '/interaction/stop')
            self.assertEqual(json.loads((output / 'report.json').read_text())['status'], 'failed')
            self.assertTrue((output / 'report.html').exists())


if __name__ == '__main__': unittest.main()
