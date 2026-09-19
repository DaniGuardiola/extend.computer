#!/bin/bash
[ -n "${BASH_VERSION:-}" ] || exec /bin/bash "$0" "$@"
set -eu
[ "$(uname -s)" = Darwin ] || exit 0
if [ "$(/usr/sbin/sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
  exec /usr/bin/arch -arm64 /bin/bash "$0" "$@"
fi
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ARCH=${TAURI_ENV_ARCH:-${EXTEND_COMPUTER_BUILD_ARCH:-}}
case "$ARCH" in
  aarch64|arm64) ARCH=arm64 ;;
  x86_64|x64) ARCH=x86_64 ;;
  '') if [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then ARCH=arm64; else ARCH=x86_64; fi ;;
  *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
esac
PACKAGE="$ROOT/native/macos/Permissions"
BUILD="$PACKAGE/.build/$ARCH"
DEST="$ROOT/desktop/src-tauri/permission-resources"
mkdir -p "$BUILD" "$DEST"
SDK=${SDKROOT:-$(/usr/bin/xcrun --sdk macosx --show-sdk-path)}
build_module() {
  MODULE=$1
  shift
  /usr/bin/xcrun swiftc -swift-version 6 -O -sdk "$SDK" -target "$ARCH-apple-macosx13.0" \
    -module-cache-path "$BUILD/module-cache" -emit-library -emit-module -module-name "$MODULE" -I "$BUILD" -L "$BUILD" \
    -emit-module-path "$BUILD/$MODULE.swiftmodule" \
    -Xlinker -install_name -Xlinker "@rpath/lib$MODULE.dylib" \
    -Xlinker -rpath -Xlinker @loader_path \
    -o "$DEST/lib$MODULE.dylib" "$@"
  cp "$DEST/lib$MODULE.dylib" "$BUILD/"
}
# Preserve full paths, including spaces in the checkout directory.
build_sources() {
  local module=$1
  shift
  local source
  local -a sources=()
  while IFS= read -r -d '' source; do
    sources+=("$source")
  done < <(find "$PACKAGE/Vendor/PermissionFlow/Sources/$module" -name '*.swift' -print0)
  build_module "$module" "${sources[@]}" "$@"
}
build_sources SystemSettingsKit
build_sources PermissionFlow -lSystemSettingsKit
build_module ExtendComputerPermissions "$PACKAGE"/Sources/ExtendComputerPermissions/*.swift "$ROOT"/native/macos/LowJitter/Service/*.swift -lPermissionFlow -lSystemSettingsKit
/usr/bin/ditto "$PACKAGE/Vendor/PermissionFlow/Sources/PermissionFlow/Resources" "$DEST/PermissionFlow_PermissionFlow.bundle"
cp "$PACKAGE/Vendor/PermissionFlow/LICENSE" "$DEST/PermissionFlow-LICENSE"
for library in "$DEST"/*.dylib; do /usr/bin/codesign --force --sign - "$library"; done
