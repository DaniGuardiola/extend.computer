#!/usr/bin/env python3
"""Create a separately installed, signed display test bundle from a known-good app."""
from pathlib import Path
import shutil
import subprocess
import argparse
import plistlib
import re

root = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--base', type=Path, required=True)
parser.add_argument('--destination', type=Path, required=True)
parser.add_argument('--identity', required=True)
parser.add_argument('--binary', type=Path, help='Built app executable; defaults to the arm64 debug build.')
args = parser.parse_args()
source = args.base.resolve()
destination = args.destination.resolve()
binary = args.binary.resolve() if args.binary else root / 'desktop/src-tauri/target/aarch64-apple-darwin/debug/extend-computer-desktop'
identity = args.identity
usb_source = root / 'target/extend-usb-helper'
cursor_source = root / 'target/extend.computer Cursor.app'
if not cursor_source.is_dir():
    raise SystemExit('Build scripts/macos/build-cursor-helper.sh before packaging the test app.')
if not usb_source.is_dir():
    raise SystemExit('Build scripts/macos/build-usb-helper.sh before packaging the test app.')
if plistlib.loads((source / 'Contents/Info.plist').read_bytes())['CFBundleIdentifier'] != 'computer.extend.desktop':
    raise SystemExit('Base must be the production-identity extend.computer app.')
if destination.exists():
    raise SystemExit('Test bundle already exists; preserve it or choose a new destination.')
shutil.copytree(source, destination, symlinks=True)
# Copied bases may predate required macOS local-network privacy declarations.
info_path = destination / 'Contents/Info.plist'
info = plistlib.loads(info_path.read_bytes())
info.update(plistlib.loads((root / 'desktop/src-tauri/Info.plist').read_bytes()))
info_path.write_bytes(plistlib.dumps(info))
shutil.copy2(binary, destination / 'Contents/MacOS/extend-computer-desktop')
subprocess.run(['strip', '-S', str(destination / 'Contents/MacOS/extend-computer-desktop')], check=True)
helper = destination / 'Contents/Resources/helpers/extend.computer Display.app'
if helper.exists():
    shutil.rmtree(helper)
shutil.copytree(root / 'target/extend.computer Display.app', helper)
cursor = destination / 'Contents/Resources/helpers/extend.computer Cursor.app'
if cursor.exists():
    shutil.rmtree(cursor)
shutil.copytree(cursor_source, cursor)
usb_helper = destination / 'Contents/Resources/helpers/ExtendComputerUsb'
if usb_helper.exists():
    shutil.rmtree(usb_helper)
shutil.copytree(usb_source, usb_helper)
library = usb_helper / 'libusb-1.0.0.dylib'
subprocess.run(['codesign', '--force', '--sign', identity, '--timestamp=none', '--options', '0', str(library)], check=True)
details = subprocess.run(['codesign', '-dvv', str(library)], text=True, capture_output=True, check=True).stderr
team = re.search(r'^TeamIdentifier=(.+)$', details, re.MULTILINE)
# Local certificates and ad-hoc signatures lack a team, so only those helper
# builds omit hardened runtime. Developer ID helper/library pairs retain it.
runtime = ['--options', 'runtime' if team and team.group(1) != 'not set' else '0']
for image in ('libusb-1.0.0.dylib', 'ExtendComputerUsb'):
    subprocess.run(['codesign', '--force', '--sign', identity, '--timestamp=none', *runtime, str(usb_helper / image)], check=True)
for bundle in (cursor, helper, destination):
    subprocess.run(['codesign', '--force', '--sign', identity, '--timestamp=none', '--options', 'runtime', str(bundle)], check=True)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(destination)], check=True)
print(destination)
