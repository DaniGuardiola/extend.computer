"""Fixed synthetic-raster source operation. No caller-selected capture targets."""
import base64
import hashlib
import json
import struct
import subprocess
import time
import uuid
from pathlib import Path


def validate_image(encoded):
    if not isinstance(encoded, str) or len(encoded) > 4_000_000:
        raise ValueError('Synthetic PNG exceeds limit')
    raw = base64.b64decode(encoded, validate=True)
    if len(raw) < 33 or raw[:8] != b'\x89PNG\r\n\x1a\n' or raw[12:16] != b'IHDR':
        raise ValueError('Synthetic PNG required')
    width, height = struct.unpack('>II', raw[16:24])
    if (width, height) not in {(1280, 720), (2560, 1440), (3840, 2160)}:
        raise ValueError('Unsupported synthetic raster dimensions')
    return raw


class QualityController:
    def __init__(self, session):
        self.session = session
        self.process = None
        self.ident = None

    def active(self):
        return self.process is not None and self.process.poll() is None

    def start(self, body):
        if set(body) != {'id', 'product', 'screen', 'image_base64'}:
            raise ValueError('Unexpected quality fields')
        ident = str(uuid.UUID(body['id']))
        screen = body['screen']
        if not isinstance(screen, str) or not (screen == 'auto' or screen.isdigit()):
            raise ValueError('Invalid display')
        raw = validate_image(body['image_base64'])
        session = self.session
        with session.lock:
            if self.active() or getattr(session,'interaction',None) and session.interaction.active() or getattr(session,'cursor',None) and session.cursor.active() or session.job and session.job['status'] == 'running':
                raise ValueError('Another fixture running')
            if body['product'] not in session.contexts:
                raise ValueError('Preflight context required')
            destination = session.root / ident
            destination.mkdir(parents=True, exist_ok=False, mode=0o700)
            target = destination / 'target.png'
            target.write_bytes(raw)
            output = destination / 'reference'
            tool = Path(session.tool).parent / 'quality-fixture'
            with (destination / 'fixture.log').open('w') as log:
                self.process = subprocess.Popen([str(tool), str(target), str(output), screen],
                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            self.ident = ident
        try:
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                if output.with_suffix('.json').exists():
                    metadata = json.loads(output.with_suffix('.json').read_text())
                    if metadata.get('status') != 'ready':
                        raise RuntimeError(metadata.get('error', 'Quality fixture failed'))
                    screenshot = output.with_suffix('.png').read_bytes()
                    if len(screenshot) > 8_000_000:
                        raise RuntimeError('Synthetic screenshot exceeds limit')
                    return {'id': ident, 'fixture': metadata,
                        'target_sha256': hashlib.sha256(raw).hexdigest(),
                        'reference_sha256': hashlib.sha256(screenshot).hexdigest(),
                        'reference_base64': base64.b64encode(screenshot).decode()}
                if not self.active():
                    raise RuntimeError('Quality fixture exited before readiness')
                time.sleep(.05)
            raise TimeoutError('Quality fixture did not become ready')
        except BaseException:
            self.stop({'id': ident})
            raise

    def stop(self, body):
        if set(body) != {'id'} or body['id'] != self.ident:
            raise ValueError('Quality fixture ID mismatch')
        with self.session.lock:
            if self.active():
                self.process.terminate()
                try: self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill(); self.process.wait(timeout=3)
            return {'status': 'stopped', 'id': self.ident}
