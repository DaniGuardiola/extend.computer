#!/bin/bash
# Build the accessory carrier and its redistributable dependency for macOS 13.
[ -n "${BASH_VERSION:-}" ] || exec /bin/bash "$0" "$@"
set -euo pipefail
[ "$(uname -s)" = Darwin ] || exit 0
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
arch=${TAURI_ENV_ARCH:-${EXTEND_COMPUTER_BUILD_ARCH:-universal}}
case "$arch" in
  aarch64|arm64) arches=(arm64) ;;
  x86_64|x64) arches=(x86_64) ;;
  universal) arches=(arm64 x86_64) ;;
  *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac
version=1.0.30
sha256=fea36f34f9156400209595e300840767ab1a385ede1dc7ee893015aea9c6dbaf
url="https://github.com/libusb/libusb/releases/download/v$version/libusb-$version.tar.bz2"
work="$root/target/usb-helper"
destination="$root/target/extend-usb-helper"
mkdir -p "$work/source"
archive=${EXTEND_LIBUSB_ARCHIVE:-$work/source/libusb-$version.tar.bz2}
temporary=$(mktemp -d "$work/build.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
if [ ! -f "$archive" ]; then
  if [ -n "${EXTEND_LIBUSB_ARCHIVE:-}" ]; then
    echo "Configured libusb source archive does not exist: $archive" >&2; exit 1
  fi
  curl --fail --location --retry 3 "$url" -o "$temporary/source.tar.bz2"
  echo "$sha256  $temporary/source.tar.bz2" | shasum -a 256 -c - >/dev/null
  mv "$temporary/source.tar.bz2" "$archive"
fi
echo "$sha256  $archive" | shasum -a 256 -c - >/dev/null
tar -xjf "$archive" -C "$temporary"
source="$temporary/libusb-$version"
sdk=${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}
stage="$temporary/bundle"
mkdir -p "$stage"
for current in "${arches[@]}"; do
  build="$temporary/$current"
  prefix="$build/install"
  mkdir -p "$build"
  (
    cd "$build"
    CC="$(xcrun --find clang)" CFLAGS="-O2 -arch $current -isysroot $sdk -mmacosx-version-min=13.0" \
      LDFLAGS="-arch $current -isysroot $sdk -mmacosx-version-min=13.0" \
      MACOSX_DEPLOYMENT_TARGET=13.0 \
      "$source/configure" --host="$current-apple-darwin" --prefix="$prefix" --disable-static --enable-shared --disable-dependency-tracking
    make -j "${EXTEND_BUILD_JOBS:-4}"
    make install
  )
  xcrun clang -O2 -Wall -Wextra -Werror -isysroot "$sdk" -target "$current-apple-macosx13.0" \
    "$root/scripts/android/aoa-host.c" -I "$prefix/include/libusb-1.0" \
    "$prefix/lib/libusb-1.0.0.dylib" -pthread -o "$build/ExtendComputerUsb"
  cp "$prefix/lib/libusb-1.0.0.dylib" "$build/libusb-1.0.0.dylib"
  dependency=$(python3 - "$build/ExtendComputerUsb" <<'PY'
import subprocess
import sys
lines = subprocess.check_output(['otool', '-L', sys.argv[1]], text=True).splitlines()
matches = [line.strip().split(' (', 1)[0] for line in lines
           if line.startswith('\t') and 'libusb-1.0' in line]
if len(matches) != 1:
    raise SystemExit('Expected exactly one libusb dependency')
print(matches[0])
PY
  )
  [ -n "$dependency" ] || { echo 'USB helper did not link libusb' >&2; exit 1; }
  install_name_tool -change "$dependency" '@loader_path/libusb-1.0.0.dylib' "$build/ExtendComputerUsb"
  install_name_tool -id '@loader_path/libusb-1.0.0.dylib' "$build/libusb-1.0.0.dylib"
done
for image in ExtendComputerUsb libusb-1.0.0.dylib; do
  inputs=()
  for current in "${arches[@]}"; do inputs+=("$temporary/$current/$image"); done
  if [ "${#arches[@]}" = 1 ]; then cp "${inputs[0]}" "$stage/$image"
  else xcrun lipo -create "${inputs[@]}" -output "$stage/$image"; fi
done
# Reject newer deployment targets and dependencies outside the copied bundle.
python3 - "$stage" <<'PY'
from pathlib import Path
import re
import subprocess
import sys
folder = Path(sys.argv[1])
for filename in ('ExtendComputerUsb', 'libusb-1.0.0.dylib'):
    path = folder / filename
    commands = subprocess.check_output(['otool', '-l', str(path)], text=True)
    # Only deployment commands: other load commands may also contain a version.
    deployment = re.findall(r'cmd LC_(?:BUILD_VERSION|VERSION_MIN_MACOSX)\b(.*?)(?=\nLoad command|\Z)', commands, re.S)
    if not deployment:
        raise SystemExit(f'Missing deployment version: {path}')
    for command in deployment:
        match = re.search(r'\b(?:minos|version)\s+(\d+)\.(\d+)', command)
        if not match or tuple(map(int, match.groups())) > (13, 0):
            raise SystemExit(f'USB dependency requires macOS newer than 13: {path}')
    linked = subprocess.check_output(['otool', '-L', str(path)], text=True)
    for line in linked.splitlines():
        if not line.startswith('\t'):
            continue
        dependency = line.strip().split(' (', 1)[0]
        if dependency == '@loader_path/libusb-1.0.0.dylib':
            continue
        if not dependency.startswith(('/usr/lib/', '/System/Library/')):
            raise SystemExit(f'Unbundled USB dependency: {dependency}')
PY
cp "$source/COPYING" "$stage/libusb-COPYING"
python3 - "$stage/libusb-source.json" "$version" "$url" "$sha256" "${arches[*]}" <<'PY'
import json
import sys
from pathlib import Path
path, version, url, digest, arches = sys.argv[1:]
Path(path).write_text(json.dumps({'name': 'libusb', 'version': version, 'source': url,
    'sha256': digest, 'license': 'LGPL-2.1-or-later', 'architectures': arches.split(),
    'minimum_macos': '13.0', 'linkage': 'dynamic'}, indent=2) + '\n')
PY
# Ad-hoc and local self-signed identities have no Apple TeamIdentifier. Their
# helper cannot use hardened library validation with a separate dylib. Actual
# Developer ID builds retain hardened runtime and sign both images as one team.
identity=${APPLE_SIGNING_IDENTITY:--}
codesign --force --sign "$identity" --identifier computer.extend.usb.libusb-1.0.0 --options 0 "$stage/libusb-1.0.0.dylib"
team=$(codesign -dvv "$stage/libusb-1.0.0.dylib" 2>&1 | sed -n 's/^TeamIdentifier=//p')
runtime=(--options 0)
if [ -n "$team" ] && [ "$team" != 'not set' ]; then runtime=(--options runtime); fi
for image in libusb-1.0.0.dylib ExtendComputerUsb; do
  codesign --force --sign "$identity" --identifier "computer.extend.usb.${image%.dylib}" "${runtime[@]}" "$stage/$image"
done
mkdir -p "$destination"
for filename in ExtendComputerUsb libusb-1.0.0.dylib libusb-COPYING libusb-source.json; do
  cp "$stage/$filename" "$destination/$filename"
done
printf '%s\n' "$destination"
