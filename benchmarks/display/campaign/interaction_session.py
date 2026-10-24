"""Bounded real OS events in the owned source fixture, not remote input latency."""
import base64
import hashlib
import json
import subprocess
import time
import threading
import uuid
from pathlib import Path

ACTIONS = ('click', 'field-focus', 'typing', 'select-all-replacement', 'drag', 'scroll')


class InteractionController:
    def __init__(self, session):
        self.session = session
        self.process = None
        self.ident = None
        self.output = None
        self.context = None
        self.fixture_hash = None
        self.deadline = None
        self.watchdog = None
        self.forced_error = None

    def active(self):
        return self.process is not None and self.process.poll() is None

    def validate_id(self, body):
        if set(body) != {'id'} or body['id'] != self.ident:
            raise ValueError('Interaction fixture ID mismatch')

    def start(self, body):
        if set(body) not in ({'id', 'product', 'screen'}, {'id', 'product', 'screen', 'observe'}) or type(body.get('observe', False)) is not bool:
            raise ValueError('Unexpected interaction fields')
        ident = str(uuid.UUID(body['id']))
        screen = body['screen']
        if not isinstance(screen, str) or not (screen == 'auto' or screen.isascii() and screen.isdigit()):
            raise ValueError('Invalid display')
        session = self.session
        with session.lock:
            if self.active() or session.quality.active() or session.cursor.active() or session.job and session.job['status'] == 'running':
                raise ValueError('Another fixture running')
            if body['product'] not in session.contexts:
                raise ValueError('Preflight context required')
            destination = session.root / ident
            destination.mkdir(parents=True, exist_ok=False, mode=0o700)
            tool = Path(session.tool).parent / 'interaction-fixture'
            self.fixture_hash = hashlib.sha256(tool.read_bytes()).hexdigest()
            self.ident = ident
            self.output = destination / 'interaction.json'
            self.context = session.contexts[body['product']]
            duration = 16 if body.get('observe') else 12
            self.deadline = time.monotonic() + duration
            self.forced_error = None
            with (destination / 'fixture.log').open('w') as log:
                self.process = subprocess.Popen([str(tool), str(self.output), screen] + (['--observe'] if body.get('observe') else []), stdout=log,
                    stderr=subprocess.STDOUT, start_new_session=True)
            self.watchdog = threading.Timer(duration, self.expire, args=(ident, self.process))
            self.watchdog.daemon = True
            self.watchdog.start()
        return {'id': ident, 'status': 'running'}

    def expire(self, ident, process):
        with self.session.lock:
            if self.ident == ident and self.process is process and self.active():
                self.forced_error = 'Interaction fixture deadline exceeded'
                self.terminate()
                self.failure(self.forced_error)

    def state(self, body):
        self.validate_id(body)
        with self.session.lock:
            if self.forced_error:
                return self.failure(self.forced_error)
            if self.active() and time.monotonic() >= self.deadline:
                self.terminate()
                return self.failure('Interaction fixture deadline exceeded')
            if self.active():
                ready = Path(str(self.output) + '.ready.json')
                if ready.exists():
                    metadata = json.loads(ready.read_text())
                    image = Path(str(self.output) + '.png').read_bytes()
                    if metadata.get('status') != 'ready' or len(image) > 8_000_000 or not image.startswith(b'\x89PNG\r\n\x1a\n'):
                        raise ValueError('Invalid owned interaction snapshot')
                    return {'id': self.ident, 'status': 'ready', 'fixture': metadata,
                        'reference_sha256': hashlib.sha256(image).hexdigest(),
                        'reference_base64': base64.b64encode(image).decode(), 'fixture_sha256': self.fixture_hash}
                return {'id': self.ident, 'status': 'running'}
            try:
                raw = self.output.read_bytes()
                if len(raw) > 65536:
                    raise ValueError('Interaction evidence exceeds limit')
                evidence = json.loads(raw)
            except (OSError, ValueError) as error:
                return self.failure('Missing or invalid interaction evidence: ' + str(error))
            valid = evidence.get('status') == 'passed' and self.process.returncode == 0
            actions = evidence.get('actions', [])
            valid = valid and isinstance(actions, list) and [a.get('action') for a in actions] == list(ACTIONS)
            valid = valid and all(a.get('accepted') is True for a in actions) and evidence.get('cursor_restored') is True
            result = {'id': self.ident, 'status': 'complete' if valid else 'failed',
                'fixture': evidence, 'exit_code': self.process.returncode,
                'fixture_sha256': self.fixture_hash, 'evidence_sha256': hashlib.sha256(raw).hexdigest(),
                'context': self.context,
                'measurement_scope': 'Source OS events and owned control outcomes only; receiver acceptance and physical input latency unmeasured'}
            if not valid:
                result['error'] = evidence.get('error') or 'Interaction outcome, cleanup or exit status failed validation'
            (self.output.parent / 'result.json').write_text(json.dumps(result, indent=2))
            return result

    def failure(self, message):
        result = {'id': self.ident, 'status': 'failed', 'error': message,
            'exit_code': self.process.returncode, 'fixture_sha256': self.fixture_hash}
        (self.output.parent / 'result.json').write_text(json.dumps(result, indent=2))
        return result

    def terminate(self):
        if self.watchdog:
            self.watchdog.cancel()
        if self.active():
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)

    def stop(self, body):
        self.validate_id(body)
        with self.session.lock:
            self.terminate()
        return self.state(body)
