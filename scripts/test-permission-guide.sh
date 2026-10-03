#!/bin/bash
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PACKAGE="$ROOT/native/macos/Permissions"
ARCH=$(uname -m)
BUILD="$PACKAGE/.build/$ARCH"
if [ ! -f "$BUILD/SystemSettingsKit.swiftmodule" ]; then bash "$ROOT/scripts/build-permission-flow.sh"; fi
TMP=$(mktemp -d /tmp/extend-permission-guide.XXXXXX)
trap 'rm -rf "$TMP"' EXIT
sources=()
while IFS= read -r -d '' source; do sources+=("$source"); done < <(find "$PACKAGE/Vendor/PermissionFlow/Sources/PermissionFlow" -name '*.swift' -print0)
xcrun swiftc -swift-version 6 -module-cache-path "$BUILD/module-cache" -I "$BUILD" -L "$BUILD" \
  -Xlinker -rpath -Xlinker "$BUILD" -lSystemSettingsKit \
  "${sources[@]}" "$PACKAGE/Tests/GuideTests.swift" -o "$TMP/guide-tests"
"$TMP/guide-tests" "$@"
xcrun swiftc -module-cache-path "$BUILD/module-cache" \
  "$PACKAGE/Sources/ExtendComputerPermissions/SettingsReturnGuard.swift" \
  "$PACKAGE/Tests/SettingsReturnGuardTests.swift" -o "$TMP/return-tests"
"$TMP/return-tests"
