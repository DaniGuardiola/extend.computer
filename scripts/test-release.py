#!/usr/bin/env python3
"""Exercise real Sparkle signatures and feed evolution with disposable keys."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('release', REPO / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

class ReleaseTests(unittest.TestCase):
    def test_consumed_preview_changesets_allow_release_and_graduation(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = Path(directory)
            changesets = desktop / '.changeset'
            changesets.mkdir()
            (changesets / 'README.md').write_text('Instructions')
            (changesets / 'preview.md').write_text('Preview change')
            with patch.object(release, 'ROOT', desktop):
                self.assertTrue(release.pending_changesets())
                (changesets / 'pre.json').write_text(json.dumps({'mode':'pre','changesets':['preview']}))
                self.assertFalse(release.pending_changesets())
                (changesets / 'new.md').write_text('New change')
                self.assertTrue(release.pending_changesets())
                (changesets / 'new.md').unlink()
                (changesets / 'pre.json').write_text(json.dumps({'mode':'exit','changesets':['preview']}))
                self.assertTrue(release.pending_changesets())
                (changesets / 'preview.md').unlink()
                (changesets / 'pre').mkdir()
                (changesets / 'pre/preview.md').write_text('Archived preview change')
                self.assertTrue(release.pending_changesets())
                (changesets / 'pre.json').write_text(json.dumps({'mode':'pre','tag':'beta'}))
                self.assertFalse(release.pending_changesets())

    def test_real_alpha_beta_version_and_graduation(self):
        cli = REPO / 'node_modules/@changesets/cli/bin.js'
        if not cli.exists() or not shutil.which('node'):
            self.skipTest('Install workspace dependencies first')
        for channel in ('alpha', 'beta'):
            with self.subTest(channel=channel), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                desktop = root / 'desktop'
                desktop.mkdir()
                changesets = root / '.changeset'
                changesets.mkdir()
                (root / 'package.json').write_text(json.dumps({'name':'fixture-workspace','private':True,'workspaces':['desktop']}))
                (root / 'package-lock.json').write_text('{}')
                (desktop / 'package.json').write_text(json.dumps({'name':'extend-computer-desktop','private':True,'version':'1.0.0'}))
                config = json.loads((REPO / '.changeset/config.json').read_text())
                config['changelog'] = False
                (changesets / 'config.json').write_text(json.dumps(config))
                (changesets / 'preview.md').write_text('---\n"extend-computer-desktop": minor\n---\nPreview fixture.\n')
                def command(*args):
                    result = subprocess.run(['node', str(cli), *args], cwd=root, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                command('pre', 'enter', channel)
                command('version')
                preview = json.loads((desktop / 'package.json').read_text())['version']
                self.assertEqual(preview, f'1.1.0-{channel}.0')
                release.validate_channel(channel, preview)
                with patch.object(release, 'ROOT', root):
                    self.assertFalse(release.pending_changesets())
                    command('pre', 'exit')
                    self.assertTrue(release.pending_changesets())
                    command('version')
                    self.assertFalse(release.pending_changesets())
                self.assertEqual(json.loads((desktop / 'package.json').read_text())['version'], '1.1.0')

    def test_signing_config_uses_resolved_certificate_fingerprint(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = Path(directory)
            (desktop / 'src-tauri').mkdir()
            fingerprint = 'A' * 40
            env = {'APPLE_SIGNING_IDENTITY':'Developer ID Application: Name (TEAM)',
                   'APPLE_SIGNING_IDENTITY_SHA1':fingerprint,
                   'SPARKLE_PUBLIC_KEY':'fixture','UPDATE_FEED_BASE':''}
            with patch.object(release, 'DESKTOP', desktop), patch.object(release, 'sync'), patch.object(release, 'version', return_value='1.0.0'), patch.dict(os.environ, env):
                release.config('stable', 10)
                config = json.loads((desktop / 'src-tauri/tauri.release.json').read_text())
                self.assertEqual(config['bundle']['macOS']['signingIdentity'], fingerprint)
                with patch.dict(os.environ, {'APPLE_SIGNING_IDENTITY_SHA1':'invalid'}):
                    with self.assertRaises(ValueError): release.config('stable', 10)

    def test_channel_cannot_cross_publish(self):
        for channel, value in [('stable', '1.0.0'), ('alpha', '1.1.0-alpha.0'), ('beta', '1.1.0-beta.0'), ('canary', '1.1.0-canary.7')]:
            release.validate_channel(channel, value)
        for channel, value in [('stable', '1.1.0-beta.0'), ('beta', '1.0.0'), ('alpha', '1.1.0-beta.0')]:
            with self.assertRaises(ValueError): release.validate_channel(channel, value)

    def test_signed_feed_retries_retains_history_and_rejects_tampering(self):
        tools = REPO / 'native/macos/Updater/vendor/bin'
        if not (tools / 'sign_update').exists(): self.skipTest('Run build-updater.sh on macOS first')
        # OpenSSL PKCS#8 Ed25519 DER ends in the 32-byte seed. Do not touch Keychain.
        private = subprocess.check_output(['openssl', 'genpkey', '-algorithm', 'ed25519', '-outform', 'DER'])
        key = base64.b64encode(private[-32:]).decode()
        public_der = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'], input=private, capture_output=True, check=True).stdout
        public = base64.b64encode(public_der[-32:]).decode()
        with tempfile.TemporaryDirectory(prefix='extend-release-test-') as directory:
            root = Path(directory)
            desktop = root / 'desktop'
            desktop.mkdir()
            (desktop / 'scripts').symlink_to(REPO / 'desktop/scripts', target_is_directory=True)
            (desktop / 'node_modules').symlink_to(REPO / 'desktop/node_modules', target_is_directory=True)
            tool_directory = root / 'native/macos/Updater'
            tool_directory.mkdir(parents=True)
            (tool_directory / 'vendor').symlink_to(tools.parent, target_is_directory=True)
            (desktop / 'package.json').write_text(json.dumps({'version':'1.0.0'}))
            (desktop / 'CHANGELOG.md').write_text('# App\n\n## 1.0.0\n\n- Offline notes.\n- ![remote](https://example.invalid/pixel.png)\n')
            archive = root / 'app.dmg'
            archive.write_bytes(b'disposable signed archive fixture')
            feeds = root / 'feeds'
            with patch.object(release, 'ROOT', root), patch.object(release, 'DESKTOP', desktop), patch.dict(os.environ, {'SPARKLE_PRIVATE_KEY':key,'SPARKLE_PUBLIC_KEY':public}):
                release.appcast('stable', 10, archive, feeds)
                release.appcast('stable', 11, archive, feeds)
                release.appcast('stable', 11, archive, feeds)
            with self.assertRaises(ValueError): release.validate_signing_keys(key, base64.b64encode(b'0' * 32).decode())
            feed = feeds / 'stable.xml'
            tree = ET.parse(feed)
            entries = tree.findall('./channel/item')
            self.assertEqual(len(entries), 2)
            self.assertEqual({entry.findtext(f'{{{release.SPARKLE}}}version') for entry in entries}, {'10','11'})
            self.assertNotIn('<img', entries[0].findtext('description'))
            verify = [str(tools / 'sign_update'), '--verify', '--ed-key-file', '-', str(feed)]
            checked = subprocess.run(verify, input=key, text=True, capture_output=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            feed.write_text(feed.read_text().replace('Offline notes.', 'Tampered notes.'))
            tampered = subprocess.run(verify, input=key, text=True, capture_output=True)
            self.assertNotEqual(tampered.returncode, 0)

if __name__ == '__main__': unittest.main()
