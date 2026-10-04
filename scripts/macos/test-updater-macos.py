#!/usr/bin/env python3
"""Exercise the shipping Sparkle bridge in an isolated, signed Cocoa app.

Requires the release signing identity in the login keychain. Never launches the
desktop app or touches its device database. Artifacts remain in the printed
temporary directory for inspection.
"""
import argparse
import functools
import http.server
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]


def run(*args, **kwargs):
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--credential-directory', type=Path, default=Path.home() / 'Library/Application Support/extend.computer/signing')
    parser.add_argument('--tamper', action='store_true', help='Verify that a modified signed feed is rejected')
    args = parser.parse_args()
    metadata = json.loads((args.credential_directory / 'metadata.json').read_text())
    key = (args.credential_directory / 'sparkle-private-key.txt').read_text().strip()
    public = (args.credential_directory / 'sparkle-public-key.txt').read_text().strip()
    identity = metadata['certificate_sha1']
    directory = Path(tempfile.mkdtemp(prefix='extend-updater-verification-'))
    print(f'Verification artifacts: {directory}', flush=True)
    served = directory / 'server'
    served.mkdir()
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(served))
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    identifier = 'computer.extend.updater-verification.' + uuid.uuid4().hex
    markers = directory / 'markers'
    markers.mkdir()
    app = directory / 'installed/Updater Verification.app'
    contents = app / 'Contents'
    for name in ('MacOS', 'Resources', 'Frameworks'):
        (contents / name).mkdir(parents=True)
    vendor = ROOT / 'native/macos/Updater/vendor'
    run('ditto', vendor / 'Sparkle.framework', contents / 'Frameworks/Sparkle.framework')
    shutil.copy2(ROOT / 'desktop/src-tauri/updater-resources/libExtendUpdater.dylib', contents / 'Resources')
    run('xcrun', 'clang', '-arch', 'arm64', '-mmacosx-version-min=13.0', '-fobjc-arc', '-fblocks',
        ROOT / 'native/macos/Updater/Verification.m', '-framework', 'Cocoa',
        '-L', contents / 'Resources', '-lExtendUpdater', '-Wl,-rpath,@executable_path/../Resources',
        '-o', contents / 'MacOS/Verification')
    info = {'CFBundleIdentifier': identifier, 'CFBundleName': 'Updater Verification',
            'CFBundleExecutable': 'Verification', 'CFBundlePackageType': 'APPL',
            'CFBundleVersion': '1', 'CFBundleShortVersionString': '0.1.0',
            'LSMinimumSystemVersion': '13.0', 'SUPublicEDKey': public,
            'SUFeedURL': base + '/stable.xml', 'ExtendUpdateFeedBase': base,
            'ExtendUpdateDefaultChannel': 'stable', 'SUEnableSystemProfiling': False,
            'SUEnableJavaScript': False, 'SUVerifyUpdateBeforeExtraction': True,
            'SURequireSignedFeed': True, 'SUSignedFeedFailureExpirationInterval': 0,
            'ExtendVerificationDirectory': str(markers),
            'NSAppTransportSecurity': {'NSAllowsLocalNetworking': True}}

    def sign_bundle(bundle, version):
        info['CFBundleVersion'] = version
        (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        for path in bundle.rglob('*'):
            if path.is_file() and not path.is_symlink():
                kind = run('file', '-b', path, capture_output=True, text=True).stdout
                if 'Mach-O' in kind:
                    run('codesign', '--force', '--sign', identity, '--timestamp', '--options', 'runtime', path)
        nested = [p for p in bundle.rglob('*') if p.is_dir() and not p.is_symlink() and p.suffix in ('.app', '.xpc', '.framework')]
        for path in sorted(nested, key=lambda p: len(p.parts), reverse=True):
            run('codesign', '--force', '--sign', identity, '--timestamp', '--options', 'runtime', path)
        run('codesign', '--force', '--sign', identity, '--timestamp', '--options', 'runtime', bundle)
        run('codesign', '--verify', '--deep', '--strict', bundle)

    candidate = directory / 'candidate/Updater Verification.app'
    candidate.parent.mkdir()
    run('ditto', app, candidate)
    sign_bundle(candidate, '2')
    sign_bundle(app, '1')
    archive = served / 'update.zip'
    run('ditto', '-c', '-k', '--keepParent', candidate, archive)
    signer = vendor / 'bin/sign_update'
    signature = run(signer, '--ed-key-file', '-', '-p', archive, input=key, capture_output=True, text=True).stdout.strip()
    feed = served / 'stable.xml'
    feed.write_text(f'''<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel>
<title>Isolated updater verification</title><item><title>Verification version 2</title>
<sparkle:version>2</sparkle:version><sparkle:shortVersionString>0.2.0</sparkle:shortVersionString>
<description>Native update verification.</description><enclosure url="{base}/update.zip" length="{archive.stat().st_size}" type="application/octet-stream" sparkle:edSignature="{signature}"/>
</item></channel></rss>''')
    run(signer, '--ed-key-file', '-', feed, input=key, capture_output=True, text=True)
    if args.tamper:
        feed.write_text(feed.read_text().replace('Native update verification.', 'Tampered update verification.'))
    environment = os.environ.copy()
    environment['EXTEND_UPDATE_TEST_DIRECTORY'] = str(markers)
    if args.tamper:
        environment['EXTEND_UPDATE_TEST_EXPECT_ERROR'] = '1'
    with (directory / 'host.log').open('w') as log:
        process = subprocess.Popen([str(contents / 'MacOS/Verification')], env=environment, stdout=log, stderr=log)
        expected = ['menu.marker', 'error-dialog.marker', 'aborted.marker'] if args.tamper else [
            'menu.marker', 'deferred.marker', 'sharing-ended.marker', 'reserved.marker', 'cleanup.marker', 'relaunch.marker']
        deadline = time.monotonic() + 100
        while time.monotonic() < deadline:
            if all((markers / name).exists() for name in expected):
                break
            if (markers / 'unsafe-install.marker').exists() or (markers / 'timeout.marker').exists():
                break
            if args.tamper and process.poll() is not None:
                break
            time.sleep(0.25)
        if process.poll() is None:
            process.terminate()
        process.wait(timeout=10)
    server.shutdown()
    missing = [name for name in expected if not (markers / name).exists()]
    if missing or (markers / 'unsafe-install.marker').exists():
        raise SystemExit(f'Verification failed; missing markers: {missing}. Inspect {directory}/host.log')
    if args.tamper and (markers / 'cleanup.marker').exists():
        raise SystemExit('Tampered feed reached installation')
    print('Verified tampered-feed rejection.' if args.tamper else 'Verified native dialog, deferred installation, cleanup, and updated relaunch.')


if __name__ == '__main__':
    main()
