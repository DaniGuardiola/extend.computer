#!/usr/bin/env python3
"""Opt-in signing test; requires scripts/dev-app.py setup on macOS."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

spec = importlib.util.spec_from_file_location('dev_signing', Path(__file__).with_name('dev-signing.py'))
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)
with tempfile.TemporaryDirectory(prefix='extend-computer-signing-test-') as temporary:
    root = Path(temporary)
    requirement = None
    with signing.signing_keychain():
        for version in (1, 2):
            source = root/'main.c'
            source.write_text(f'int main(void) {{ return {version}; }}\n')
            binary = root/f'probe-{version}'
            subprocess.run(['xcrun', 'clang', str(source), '-o', str(binary)], check=True)
            signing.sign(binary, 'computer.extend.signing-proof.development')
            result = subprocess.run(['codesign', '-d', '-r-', str(binary)], capture_output=True, text=True, check=True)
            current = next(line.split('designated => ', 1)[1] for line in (result.stdout + result.stderr).splitlines() if line.startswith('designated => '))
            if requirement is not None:
                assert current == requirement, 'Requirement changed between builds'
            requirement = current
        subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', 'computer.extend.signing-proof.development', str(binary)], capture_output=True, check=True)
        assert subprocess.run(['codesign', '--verify', '-R', '=' + requirement, str(binary)], capture_output=True).returncode != 0, 'Impostor signature accepted'
    try:
        signing.sign(binary, 'computer.extend.desktop')
    except RuntimeError:
        pass
    else:
        raise AssertionError('Production bundle identifier accepted')
print('PASS stable requirements, impostor rejection, production identifier rejection')
