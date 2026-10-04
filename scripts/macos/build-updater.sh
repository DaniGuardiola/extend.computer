#!/bin/bash
set -euo pipefail
[[ $(uname -s) == Darwin ]] || exit 0
root=$(cd "$(dirname "$0")/../.." && pwd)
vendor="$root/native/macos/Updater/vendor"
version=2.10.0
digest=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
if [[ ! -f "$vendor/.verified-$version" ]]; then
  archive=$(mktemp -t extend-sparkle)
  trap 'rm -f "$archive"' EXIT
  curl --fail --location --retry 3 "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" -o "$archive"
  echo "$digest  $archive" | shasum -a 256 -c -
  mkdir -p "$vendor"
  tar -xJf "$archive" -C "$vendor"
  touch "$vendor/.verified-$version"
fi
arch=${TAURI_ENV_ARCH:-${EXTEND_COMPUTER_BUILD_ARCH:-$(uname -m)}}
if [[ $arch == universal ]]; then
  TAURI_ENV_ARCH=arm64 bash "$0"
  cp "$root/desktop/src-tauri/updater-resources/libExtendUpdater.dylib" "$vendor/libExtendUpdater-arm64.dylib"
  TAURI_ENV_ARCH=x86_64 bash "$0"
  xcrun lipo -create "$vendor/libExtendUpdater-arm64.dylib" "$root/desktop/src-tauri/updater-resources/libExtendUpdater.dylib" -output "$vendor/libExtendUpdater-universal.dylib"
  cp "$vendor/libExtendUpdater-universal.dylib" "$root/desktop/src-tauri/updater-resources/libExtendUpdater.dylib"
  codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" --options runtime "$root/desktop/src-tauri/updater-resources/libExtendUpdater.dylib"
  exit 0
fi
[[ $arch != aarch64 ]] || arch=arm64
dest="$root/desktop/src-tauri/updater-resources"
mkdir -p "$dest"
xcrun clang -arch "$arch" -mmacosx-version-min=13.0 -fobjc-arc -fblocks -dynamiclib \
  -F "$vendor" -framework Cocoa -framework Sparkle \
  -Wl,-install_name,@rpath/libExtendUpdater.dylib -Wl,-rpath,@loader_path/../Frameworks \
  "$root/native/macos/Updater/Updater.m" -o "$dest/libExtendUpdater.dylib"
codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" --options runtime "$dest/libExtendUpdater.dylib"
cp "$vendor/LICENSE" "$dest/Sparkle-LICENSE"
