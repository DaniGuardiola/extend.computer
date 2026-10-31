#!/bin/sh
set -eu
[ "$(uname -s)" = Darwin ] || exit 0
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir="$package_dir/.build/bundled"
mkdir -p "$build_dir"
architecture=${TAURI_ENV_ARCH:-${EXTEND_COMPUTER_BUILD_ARCH:-}}
if [ "$architecture" = universal ]; then
  TAURI_ENV_ARCH=arm64 sh "$0"
  cp "$build_dir/ExtendComputerLowJitter" "$build_dir/ExtendComputerLowJitter-arm64"
  TAURI_ENV_ARCH=x86_64 sh "$0"
  /usr/bin/xcrun lipo -create "$build_dir/ExtendComputerLowJitter-arm64" "$build_dir/ExtendComputerLowJitter" -output "$build_dir/ExtendComputerLowJitter-universal"
  mv "$build_dir/ExtendComputerLowJitter-universal" "$build_dir/ExtendComputerLowJitter"
  /usr/bin/codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" --options runtime --identifier computer.extend.lowjitter.broker "$build_dir/ExtendComputerLowJitter"
  exit 0
fi
case "$architecture" in
  aarch64|arm64) architecture=arm64 ;;
  x86_64|x64) architecture=x86_64 ;;
  '') if [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then architecture=arm64; else architecture=x86_64; fi ;;
  *) echo "Unsupported architecture: $architecture" >&2; exit 1 ;;
esac
/usr/bin/xcrun swiftc -D APP_SERVICE -target "$architecture-apple-macos13.0" \
    -module-cache-path "$build_dir/module-cache" \
    "$package_dir"/Sources/LowJitterLifecycle/*.swift \
    "$package_dir/Service/SigningIdentity.swift" "$package_dir/Helper/main.swift" \
    -o "$build_dir/ExtendComputerLowJitter"
/usr/bin/codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" --options runtime \
    --identifier computer.extend.lowjitter.broker "$build_dir/ExtendComputerLowJitter"
