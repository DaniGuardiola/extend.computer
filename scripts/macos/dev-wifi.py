#!/usr/bin/env python3
"""Prepare an explicit admin installer for the signed development Wi-Fi broker.

For machines whose SMAppService launch constraints reject the local development
certificate. This installs the same authenticated broker, never the legacy
ad-hoc helper. Production bundles are refused.
"""
import hashlib
import importlib.util
from pathlib import Path
import plistlib
import shutil
import subprocess

REPO = Path(__file__).resolve().parents[2]
APP = REPO / 'target/development/extend.computer.app'
OUTPUT = REPO / 'target/development/wifi-admin'
spec = importlib.util.spec_from_file_location('dev_signing', REPO / 'scripts/macos/dev-signing.py')
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


def prepare():
    info = plistlib.loads((APP / 'Contents/Info.plist').read_bytes())
    if info['CFBundleIdentifier'] != 'computer.extend.desktop.development':
        raise RuntimeError('Refusing a production app')
    source = APP / 'Contents/Library/LaunchServices/ExtendComputerLowJitter'
    pin = signing.fingerprint()
    requirement = f'identifier "computer.extend.lowjitter.broker.development" and certificate leaf = H"{pin}"'
    signing.run('/usr/bin/codesign', '--verify', '--strict', '-R', '=' + requirement, source)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, OUTPUT / 'ExtendComputerLowJitter')
    payload = (OUTPUT / 'ExtendComputerLowJitter').read_bytes()
    template = (REPO / 'native/macos/LowJitter/install-bundled-development.py.in').read_text()
    template = template.replace('BUILD_SHA256', hashlib.sha256(payload).hexdigest())
    template = template.replace('BUILD_REQUIREMENT', repr(requirement))
    (OUTPUT / 'install.py').write_text(template)
    subprocess.run(['/usr/bin/python3', '-m', 'py_compile', str(OUTPUT / 'install.py')], check=True)
    print(f'sudo /usr/bin/python3 "{OUTPUT / "install.py"}"')
    print('Remove with the same command followed by uninstall. Quit input sharing first.')


if __name__ == '__main__':
    prepare()
