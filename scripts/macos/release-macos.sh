#!/bin/bash
# Production signing material is confined to the runner's temporary directory.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_TEAM_ID APPLE_API_KEY_ID APPLE_API_ISSUER APPLE_API_KEY SPARKLE_PUBLIC_KEY RELEASE_BUILD; do
  [[ -n ${!name:-} ]] || { echo "Missing release credential: $name" >&2; exit 1; }
done
[[ -n ${UPDATE_FEED_BASE:-} ]] || unset UPDATE_FEED_BASE
temporary=$(mktemp -d)
keychain="$temporary/release.keychain-db"
cleanup() {
  if [[ -f "$temporary/keychain-search-list.json" ]]; then
    python3 - "$temporary/keychain-search-list.json" <<'PY'
import json, subprocess, sys
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', *json.load(open(sys.argv[1]))], check=False)
PY
  fi
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$temporary"
}
trap cleanup EXIT
umask 077
keychain_password=$(openssl rand -hex 32)
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$temporary/certificate.p12"
printf '%s' "$APPLE_API_KEY" > "$temporary/AuthKey.p8"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$temporary/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
python3 - "$temporary/keychain-search-list.json" "$keychain" <<'PY'
import json, shlex, subprocess, sys
previous = shlex.split(subprocess.check_output(['security', 'list-keychains', '-d', 'user'], text=True))
with open(sys.argv[1], 'w') as out: json.dump(previous, out)
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', sys.argv[2], *previous], check=True)
PY
# codesign's name matching can misread accented account names. Resolve the
# requested Developer ID identity in the isolated release keychain and use its
# fingerprint consistently for Tauri, native helpers, and final signing.
APPLE_SIGNING_IDENTITY_SHA1=$(python3 - "$keychain" <<'PY'
import os, re, subprocess, sys
identities = subprocess.check_output(['security', 'find-identity', '-v', '-p', 'codesigning', sys.argv[1]], text=True)
matches = re.findall(r'([0-9A-F]{40}) "([^"]+)"', identities)
matches = [digest for digest, name in matches
           if name == os.environ['APPLE_SIGNING_IDENTITY']
           and name.startswith('Developer ID Application:')
           and name.endswith('(' + os.environ['APPLE_TEAM_ID'] + ')')]
if len(matches) != 1:
    raise SystemExit('Expected exactly one valid Developer ID Application identity for the configured Apple team.')
print(matches[0])
PY
)
export APPLE_SIGNING_IDENTITY_SHA1
python3 scripts/release.py config --channel "$RELEASE_CHANNEL" --build "$RELEASE_BUILD"
export APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY_SHA1"
sh scripts/macos/build-cursor-helper.sh
# Our credential names intentionally differ from Tauri's notarization variables.
# We notarize the final, explicitly signed bundle ourselves below.
env -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD -u APPLE_API_KEY -u APPLE_API_ISSUER \
  npm run tauri --prefix desktop -- build --target universal-apple-darwin --config src-tauri/tauri.release.json
app="$root/desktop/src-tauri/target/universal-apple-darwin/release/bundle/macos/extend.computer.app"
python3 - "$app" "$APPLE_SIGNING_IDENTITY" "$APPLE_TEAM_ID" "$RELEASE_BUILD" <<'PY'
from pathlib import Path
import plistlib
import re
import subprocess
import sys
app = Path(sys.argv[1])
identity, team, build = sys.argv[2:]
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert info['CFBundleIdentifier'] == 'computer.extend.desktop'
assert info['CFBundleVersion'] == build, 'Build number was not merged into Info.plist'
assert info['SUEnableSystemProfiling'] is False
def sign(path):
    details = subprocess.run(['codesign', '-dv', str(path)], text=True, capture_output=True, check=True).stderr
    identifier = re.search(r'^Identifier=(.+)$', details, re.MULTILINE)
    assert identifier, f'Missing existing signing identifier: {path}'
    code_identifier = identifier.group(1)
    if path == app / 'Contents/Library/LaunchServices/ExtendComputerLowJitter':
        code_identifier = 'computer.extend.lowjitter.broker'
    subprocess.run(['codesign', '--force', '--sign', identity, '--identifier', code_identifier, '--timestamp', '--options', 'runtime', str(path)], check=True)
# Sign executable images first, then nested bundles from the inside out.
for path in app.rglob('*'):
    if path.is_file() and not path.is_symlink():
        kind = subprocess.check_output(['file', '-b', str(path)], text=True)
        if 'Mach-O' in kind: sign(path)
bundles = [p for p in app.rglob('*') if p.is_dir() and not p.is_symlink() and p.suffix in ('.app', '.xpc', '.framework')]
for path in sorted(bundles, key=lambda p: len(p.parts), reverse=True): sign(path)
sign(app)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
broker_requirement = f'=identifier "computer.extend.lowjitter.broker" and anchor apple generic and certificate leaf[subject.OU] = "{team}"'
subprocess.run(['codesign', '--verify', '--strict', '-R', broker_requirement, str(app / 'Contents/Library/LaunchServices/ExtendComputerLowJitter')], check=True)
details = subprocess.run(['codesign', '-dvv', str(app)], text=True, capture_output=True, check=True).stderr
assert f'TeamIdentifier={team}' in details, 'Apple signing team mismatch'
PY
mkdir -p release-output
notarize() {
  local archive=$1 receipt=$2
  # Keep non-secret submission receipts and rejection diagnostics so failed
  # releases can be investigated after the temporary credentials are removed.
  xcrun notarytool submit "$archive" --key "$temporary/AuthKey.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER" --output-format json > "$receipt"
  local submission
  submission=$(python3 - "$receipt" <<'PY'
import json, sys
print(json.load(open(sys.argv[1]))['id'])
PY
)
  echo "Waiting for Apple notarization: $submission"
  xcrun notarytool wait "$submission" --key "$temporary/AuthKey.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER" --output-format json > "$receipt.result"
  mv "$receipt.result" "$receipt"
  if ! python3 - "$receipt" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
if result['status'] != 'Accepted':
    raise SystemExit(f"Notarization failed: {result['status']} ({result['id']})")
PY
  then
    xcrun notarytool log "$submission" --key "$temporary/AuthKey.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER" "$receipt.log.json" || true
    return 1
  fi
}
ditto -c -k --keepParent "$app" "$temporary/notarize.zip"
notarize "$temporary/notarize.zip" "$root/release-output/app-notarization.json"
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose "$app"
version=$(node -p "require('./desktop/package.json').version")
dmg="$root/release-output/extend.computer-$version-universal.dmg"
mkdir "$temporary/image"
ditto "$app" "$temporary/image/extend.computer.app"
ln -s /Applications "$temporary/image/Applications"
hdiutil create -volname extend.computer -srcfolder "$temporary/image" -format UDZO -ov "$dmg"
codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$dmg"
notarize "$dmg" "$root/release-output/dmg-notarization.json"
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
(cd release-output && shasum -a 256 "extend.computer-$version-universal.dmg" > SHA256SUMS)
python3 - <<'PY'
import json, os
from pathlib import Path
version=json.loads(Path('desktop/package.json').read_text())['version']
Path('release-output/release-metadata.json').write_text(json.dumps({'version':version,'build':int(os.environ['RELEASE_BUILD']),'channel':os.environ['RELEASE_CHANNEL'],'commit':os.environ.get('GITHUB_SHA','')})+'\n')
PY
