#!/usr/bin/env python3
"""Opt-in signing boundary test; uses the already configured dev signing identity."""
import importlib.util
import re
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('dev_signing', REPO/'scripts/macos/dev-signing.py')
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)
with tempfile.TemporaryDirectory(prefix='extend-wifi-signing-') as temporary:
    root = Path(temporary)
    probe = root/'probe'
    source = root/'Probe.swift'
    shutil.copyfile(REPO/'native/macos/LowJitter/Service/IdentityProbe.swift.test', source)
    subprocess.run(['/usr/bin/xcrun', 'swiftc', '-module-cache-path', str(root/'cache'),
                    str(REPO/'native/macos/LowJitter/Service/SigningIdentity.swift'), str(source), '-o', str(probe)], check=True)
    helper = root/'helper'
    shutil.copyfile(REPO/'native/macos/LowJitter/.build/bundled/ExtendComputerLowJitter', helper)
    with signing.signing_keychain():
        signing.sign(probe, 'computer.extend.desktop.development')
        signing.sign(helper, 'computer.extend.lowjitter.broker.development', hardened_runtime=True)
        details = subprocess.run(['/usr/bin/codesign', '-dvv', str(helper)], text=True, capture_output=True, check=True)
        flags = re.search(r'flags=0x([0-9a-f]+)', details.stderr)
        assert flags and int(flags.group(1), 16) & 0x10000, 'Development signing must retain helper hardened runtime'
        assert subprocess.run([str(probe), str(helper)], capture_output=True).returncode == 0
        signing.sign(helper, 'computer.extend.unrelated.development')
        assert subprocess.run([str(probe), str(helper)], capture_output=True).returncode != 0
    subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', '--identifier', 'computer.extend.lowjitter.broker.development', str(helper)], check=True, capture_output=True)
    assert subprocess.run([str(probe), str(helper)], capture_output=True).returncode != 0
    subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', '--identifier', 'computer.extend.desktop.development', str(probe)], check=True, capture_output=True)
    assert subprocess.run([str(probe), str(helper)], capture_output=True).returncode != 0
print('PASS: matching signer accepted; wrong identifier, wrong signer, and ad-hoc app rejected.')
