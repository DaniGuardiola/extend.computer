#!/usr/bin/env python3
"""Check installer refusal paths without modifying any system files."""
import ast
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]


class InstallerSafety(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        source = (REPO / 'native/macos/LowJitter/install-bundled-development.py.in').read_text()
        source = source.replace('BUILD_REQUIREMENT', repr('test-only-requirement'))
        tree = ast.parse(source)
        # Load only definitions; never execute the root entry point.
        tree.body = [node for node in tree.body if isinstance(node, (ast.Import, ast.ImportFrom, ast.Assign, ast.FunctionDef))]
        self.code = {'__file__': str(self.root / 'install.py')}
        exec(compile(tree, '<installer-test>', 'exec'), self.code)
        self.code.update(DEST=self.root / 'installed', BINARY=self.root / 'installed/broker', PLIST=self.root / 'daemon.plist')
        self.payload = b'test broker bytes'
        (self.root / 'ExtendComputerLowJitter').write_bytes(self.payload)
        self.code['EXPECTED'] = hashlib.sha256(self.payload).hexdigest()

    def test_changed_payload_never_creates_installation(self):
        (self.root / 'ExtendComputerLowJitter').write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'Build changed'):
            self.code['install']()
        self.assertFalse(self.code['DEST'].exists())

    def test_existing_symlink_is_preserved(self):
        self.code['DEST'].symlink_to(self.root / 'missing')
        with self.assertRaisesRegex(RuntimeError, 'Already installed'):
            self.code['install']()
        self.assertTrue(self.code['DEST'].is_symlink())

    def test_running_broker_never_stops_or_replaces_service(self):
        result = subprocess.CompletedProcess([], 0, stdout='state = running', stderr='')
        with patch.object(subprocess, 'run', return_value=result):
            with self.assertRaisesRegex(RuntimeError, 'broker is running'):
                self.code['install']()
        self.assertFalse(self.code['DEST'].exists())

    def test_failed_signature_never_stops_service_or_writes_plist(self):
        result = subprocess.CompletedProcess([], 1, stdout='', stderr='')
        calls = []
        self.code['verify'] = lambda: (_ for _ in ()).throw(RuntimeError('Invalid signature'))
        self.code['stop'] = lambda label: calls.append(label)
        with patch.object(subprocess, 'run', return_value=result):
            with self.assertRaisesRegex(RuntimeError, 'Invalid signature'):
                self.code['install']()
        self.assertEqual(calls, [])
        self.assertFalse(self.code['PLIST'].exists())


if __name__ == '__main__':
    unittest.main()
