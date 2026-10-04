#!/usr/bin/env python3
"""Build and run an explicitly debug-only, consistently signed extend.computer bundle."""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('dev_signing', REPO/'scripts/dev-signing.py')
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)
DEST = REPO/'target/development/extend.computer.app'


def build():
    signing.setup()
    subprocess.run(['sh', 'scripts/build-cursor-helper.sh'], cwd=REPO, check=True)
    # No caller-supplied flags: this path cannot select a release profile.
    subprocess.run(['npm', 'run', 'tauri', '--', 'build', '--debug', '--features', 'dev-identity', '--target', 'aarch64-apple-darwin', '--config', json.dumps({'identifier': 'computer.extend.desktop.development'})], cwd=REPO/'desktop', check=True)
    source = REPO/'desktop/src-tauri/target/aarch64-apple-darwin/debug/bundle/macos/extend.computer.app'
    with tempfile.TemporaryDirectory(prefix='extend-computer-dev-build-') as temporary:
        app = Path(temporary)/'extend.computer.app'
        subprocess.run(['/usr/bin/ditto', str(source), str(app)], check=True)
        info = plistlib.loads((app/'Contents/Info.plist').read_bytes())
        if info['CFBundleIdentifier'] != 'computer.extend.desktop.development':
            raise RuntimeError('Refusing to dev-sign a production app')
        helper = app/'Contents/Resources/helpers/extend.computer Cursor.app'
        helper_info = helper/'Contents/Info.plist'
        data = plistlib.loads(helper_info.read_bytes())
        data['CFBundleIdentifier'] = 'computer.extend.prototype.cursor.development'
        helper_info.write_bytes(plistlib.dumps(data))
        # Unique build provenance also makes retention tests exercise new code hashes.
        stamp = datetime.now(timezone.utc).isoformat() + '\n'
        for bundle in (app, helper):
            resources = bundle/'Contents/Resources'
            resources.mkdir(parents=True, exist_ok=True)
            (resources/'ExtendComputerDevelopmentBuild.txt').write_text(stamp)
        daemon_plist = app/'Contents/Library/LaunchDaemons/computer.extend.lowjitter.plist'
        daemon = plistlib.loads(daemon_plist.read_bytes())
        daemon['Label'] = 'computer.extend.lowjitter.development'
        daemon['MachServices'] = {'computer.extend.lowjitter.development': True}
        daemon['AssociatedBundleIdentifiers'] = ['computer.extend.desktop.development']
        daemon_plist.write_bytes(plistlib.dumps(daemon))
        with signing.signing_keychain():
            signing.sign(app/'Contents/Library/LaunchServices/ExtendComputerLowJitter', 'computer.extend.lowjitter.broker.development', hardened_runtime=True)
            for library in sorted(app.rglob('*.dylib')):
                signing.sign(library, 'computer.extend.library.' + library.stem + '.development')
            signing.sign(helper, 'computer.extend.prototype.cursor.development')
            signing.sign(app, 'computer.extend.desktop.development')
        signing.run('/usr/bin/codesign', '--verify', '--deep', '--strict', app)
        # The stable install path is only replaced while this dev app is stopped.
        if subprocess.run(['pgrep', '-x', 'ExtendComputerCursor'], capture_output=True).returncode == 0:
            raise RuntimeError('Stop input sharing before installing the development build')
        if subprocess.run(['pgrep', '-x', 'extend-computer-desktop'], capture_output=True).returncode == 0:
            raise RuntimeError('Quit extend.computer before installing the development build')
        DEST.parent.mkdir(parents=True, exist_ok=True)
        previous = DEST.with_name('extend.computer.previous.app')
        if previous.exists(): shutil.rmtree(previous)
        if DEST.exists(): DEST.rename(previous)
        shutil.copytree(app, DEST, symlinks=True)
    print(DEST)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['setup', 'build', 'run'])
    args = parser.parse_args()
    try:
        if args.action == 'setup': signing.setup()
        else:
            build()
            if args.action == 'run': subprocess.run(['open', str(DEST)], check=True)
    except (RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'{error}\n')
