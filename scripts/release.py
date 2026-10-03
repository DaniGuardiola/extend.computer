#!/usr/bin/env python3
"""Version synchronization, production configuration, and signed release feeds."""
import argparse
import base64
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tomllib
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
DESKTOP = ROOT / 'desktop'
CHANNELS = ('stable', 'beta', 'alpha', 'canary')
SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'
ET.register_namespace('sparkle', SPARKLE)

def version():
    return json.loads((DESKTOP / 'package.json').read_text())['version']

def pending_changesets():
    directory = ROOT / '.changeset'
    state_path = directory / 'pre.json'
    state = json.loads(state_path.read_text()) if state_path.exists() else {}
    consumed = set(state.get('changesets', [])) if state.get('mode') == 'pre' else set()
    # Changesets 3 archives consumed preview notes under .changeset/pre. Exiting
    # preview mode must still create a graduation PR even with no new notes.
    if state.get('mode') == 'exit' and any((directory / 'pre').glob('*.md')):
        return True
    return any(path.name != 'README.md' and path.stem not in consumed
               for path in directory.glob('*.md'))

def sync(check=False):
    expected = version()
    for path in (ROOT / 'Cargo.toml', DESKTOP / 'src-tauri/Cargo.toml'):
        original = path.read_text()
        updated, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{expected}"', original, count=1)
        assert count == 1
        if check and original != updated:
            raise ValueError(f'Version mismatch: {path}')
        if not check:
            path.write_text(updated)
    if check:
        for lock_path in (ROOT / 'Cargo.lock', DESKTOP / 'src-tauri/Cargo.lock'):
            packages = tomllib.loads(lock_path.read_text())['package']
            for package in packages:
                if package['name'] in ('extend-computer-agent', 'extend-computer-desktop'):
                    assert package['version'] == expected, f'Rust lockfile version mismatch: {lock_path}'
    path = DESKTOP / 'src-tauri/tauri.conf.json'
    data = json.loads(path.read_text())
    if check:
        assert data['version'] == expected, 'Tauri version mismatch'
        assert f'## {expected}\n' in (DESKTOP / 'CHANGELOG.md').read_text(), 'Release changelog missing'
        lock = json.loads((ROOT / 'package-lock.json').read_text())
        assert lock['packages']['desktop']['version'] == expected, 'Workspace lockfile version mismatch'
    else:
        data['version'] = expected
        path.write_text(json.dumps(data, indent=2) + '\n')
        # Refresh only lockfile metadata; no dependency upgrades.
        subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=ROOT, stdout=subprocess.DEVNULL, check=True)
        subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=DESKTOP / 'src-tauri', stdout=subprocess.DEVNULL, check=True)
        subprocess.run(['npm', 'install', '--package-lock-only', '--ignore-scripts', '--offline'], cwd=ROOT, check=True)

def notes():
    text = (DESKTOP / 'CHANGELOG.md').read_text()
    match = re.search(r'^## ' + re.escape(version()) + r'\s*\n(.*?)(?=^## |\Z)', text, re.M | re.S)
    if not match:
        raise ValueError('No changelog for this version')
    return match.group(1).strip()

def validate_channel(channel, value):
    if channel not in CHANNELS:
        raise ValueError('Unknown channel')
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', value):
        raise ValueError('Invalid release version')
    if channel == 'stable' and '-' in value:
        raise ValueError('Stable releases cannot be prereleases')
    if channel != 'stable' and not value.split('-', 1)[-1].startswith(channel + '.'):
        raise ValueError(f'{channel} releases require a {channel} prerelease version')

def config(channel, build):
    sync(check=True)
    validate_channel(channel, version())
    identity = os.environ['APPLE_SIGNING_IDENTITY']
    key = os.environ['SPARKLE_PUBLIC_KEY']
    if not identity.startswith('Developer ID Application:') or not key:
        raise ValueError('Production signing identity and Sparkle public key required')
    signing_identity = os.environ.get('APPLE_SIGNING_IDENTITY_SHA1') or identity
    if signing_identity != identity and not re.fullmatch(r'[0-9A-Fa-f]{40}', signing_identity):
        raise ValueError('Invalid signing certificate fingerprint')
    repository = os.environ.get('RELEASE_REPOSITORY', 'DaniGuardiola/extend.computer')
    owner, name = repository.split('/')
    feed_base = os.environ.get('UPDATE_FEED_BASE') or f'https://{owner}.github.io/{name}/updates/macos'
    if not feed_base.startswith('https://'):
        raise ValueError('Update feed must use HTTPS')
    info = {
        'CFBundleVersion': str(build),
        'CFBundleShortVersionString': version(),
        'SUPublicEDKey': key,
        'SUFeedURL': f'{feed_base}/{channel}.xml',
        'ExtendUpdateFeedBase': feed_base,
        'ExtendUpdateDefaultChannel': channel,
        'SUEnableSystemProfiling': False,
        'SUEnableJavaScript': False,
        'SUVerifyUpdateBeforeExtraction': True,
        'SURequireSignedFeed': True,
        'SUSignedFeedFailureExpirationInterval': 0,
    }
    path = DESKTOP / 'src-tauri/release-info.plist'
    path.write_bytes(plistlib.dumps(info))
    (DESKTOP / 'src-tauri/tauri.release.json').write_text(json.dumps({
        'bundle': {'macOS': {'signingIdentity': signing_identity, 'hardenedRuntime': True, 'infoPlist': 'release-info.plist'}}
    }, indent=2) + '\n')

def validate_signing_keys(private_key, public_key):
    seed = base64.b64decode(private_key.strip(), validate=True)
    if len(seed) != 32:
        raise ValueError('Use a modern 32-byte Sparkle key generated by the pinned tools')
    der = bytes.fromhex('302e020100300506032b657004220420') + seed
    public_der = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'], input=der, capture_output=True, check=True).stdout
    if base64.b64encode(public_der[-32:]).decode() != public_key.strip():
        raise ValueError('Sparkle public and private keys do not match')

def appcast(channel, build, archive, feed_directory):
    validate_channel(channel, version())
    feed_directory.mkdir(parents=True, exist_ok=True)
    feed = feed_directory / f'{channel}.xml'
    # Retain existing releases so Macs on older supported OS versions can find
    # their newest compatible build. Reject corrupt feeds rather than overwrite.
    tree = ET.parse(feed) if feed.exists() else ET.ElementTree(ET.Element('rss', {'version': '2.0'}))
    root = tree.getroot()
    stream = root.find('channel')
    if stream is None:
        stream = ET.SubElement(root, 'channel')
        ET.SubElement(stream, 'title').text = 'extend.computer updates'
    for previous in list(stream.findall('item')):
        if previous.findtext(f'{{{SPARKLE}}}version') == str(build):
            stream.remove(previous)
    item = ET.Element('item')
    ET.SubElement(item, 'title').text = f'extend.computer {version()}'
    ET.SubElement(item, f'{{{SPARKLE}}}version').text = str(build)
    ET.SubElement(item, f'{{{SPARKLE}}}shortVersionString').text = version()
    ET.SubElement(item, f'{{{SPARKLE}}}minimumSystemVersion').text = '13.0'
    # Embedded notes avoid third-party resources and separate note signatures.
    rendered = subprocess.run(['node', 'scripts/render-notes.mjs'], cwd=DESKTOP, input=notes(), text=True, capture_output=True, check=True).stdout
    ET.SubElement(item, 'description').text = rendered
    repo = os.environ.get('RELEASE_REPOSITORY', 'DaniGuardiola/extend.computer')
    tools = ROOT / 'native/macos/Updater/vendor/bin'
    metadata_path = archive.parent / 'release-metadata.json'
    if metadata_path.exists():
        metadata = json.loads(metadata_path.read_text())
        if metadata.get('version') != version() or metadata.get('build') != build or metadata.get('channel') != channel:
            raise ValueError('Release metadata does not match the selected feed')
        subprocess.run(['shasum', '-a', '256', '-c', 'SHA256SUMS'], cwd=archive.parent, stdout=subprocess.DEVNULL, check=True)
    # sign_update reads the private key from stdin, never a command argument.
    key = os.environ['SPARKLE_PRIVATE_KEY']
    validate_signing_keys(key, os.environ['SPARKLE_PUBLIC_KEY'])
    result = subprocess.run([str(tools / 'sign_update'), '--ed-key-file', '-', str(archive)], input=key, text=True, capture_output=True, check=True)
    signature_match = re.search(r'sparkle:edSignature="([^"]+)"', result.stdout)
    if not signature_match: raise ValueError('Sparkle returned no archive signature')
    signature = signature_match[1]
    ET.SubElement(item, 'enclosure', {
        'url': f'https://github.com/{repo}/releases/download/v{version()}/{archive.name}',
        'length': str(archive.stat().st_size), 'type': 'application/octet-stream',
        f'{{{SPARKLE}}}edSignature': signature,
    })
    stream.insert(1, item)
    tree.write(feed, encoding='utf-8', xml_declaration=True)
    # Sparkle embeds the feed signature inside XML; never edit after signing.
    subprocess.run([str(tools / 'sign_update'), '--ed-key-file', '-', str(feed)], input=key, text=True, capture_output=True, check=True)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('sync', 'check', 'config', 'notes', 'appcast', 'channel', 'feed-contains', 'pending'))
    parser.add_argument('--channel', choices=(*CHANNELS, 'auto'), default='stable')
    parser.add_argument('--build', type=int)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--feed-directory', type=Path, default=ROOT / 'release-output/updates/macos')
    args = parser.parse_args()
    if args.action == 'channel':
        value = version()
        inferred = value.split('-', 1)[1].split('.', 1)[0] if '-' in value else 'stable'
        chosen = inferred if args.channel == 'auto' else args.channel
        validate_channel(chosen, value)
        print(chosen)
    elif args.action == 'feed-contains':
        tree = ET.parse(args.feed_directory / 'extend-current-feed.xml')
        exists = any(item.findtext(f'{{{SPARKLE}}}version') == str(args.build) for item in tree.findall('./channel/item'))
        raise SystemExit(0 if exists else 1)
    elif args.action == 'pending': print('true' if pending_changesets() else 'false')
    elif args.action in ('sync', 'check'): sync(args.action == 'check')
    elif args.action == 'notes': print(notes())
    elif args.action == 'config':
        if not args.build or args.build < 1: parser.error('--build must be positive')
        config(args.channel, args.build)
    elif args.action == 'appcast':
        if not args.build or not args.archive: parser.error('--build and --archive required')
        appcast(args.channel, args.build, args.archive.resolve(), args.feed_directory)

if __name__ == '__main__': main()
