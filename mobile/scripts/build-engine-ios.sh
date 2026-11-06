#!/bin/sh
# Build the shared session engine for iOS devices and the Apple-silicon simulator,
# then package mobile/modules/extend-display/ios/ExtendEngine.xcframework
# (static library + C header + module map, so Swift can `import ExtendEngine`).
# Usage: mobile/scripts/build-engine-ios.sh [--debug]
set -eu
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo "iOS engine build requires macOS with Xcode." >&2; exit 1; }
task_profile=release
task_flag=--release
if [ "${1:-}" = "--debug" ]; then
  task_profile=debug
  task_flag=
fi
# iOS std components belong to the workspace-pinned rustup toolchain.
export RUSTC="$(rustup which rustc)"
task_cargo="$(rustup which cargo)"
# Match the deployment target of the Expo app; clang/cc honor this too.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.1}"
task_header_dir=native/ios/engine/include
if command -v cbindgen >/dev/null 2>&1; then
  native/ios/engine/generate-header.sh --check
else
  echo "cbindgen not installed; using checked-in $task_header_dir/ExtendEngine.h" >&2
fi
for task_target in aarch64-apple-ios aarch64-apple-ios-sim; do
  # shellcheck disable=SC2086 # empty task_flag must vanish
  "$task_cargo" build --locked $task_flag -p extend-ios-engine --target "$task_target"
done
task_output=mobile/modules/extend-display/ios/ExtendEngine.xcframework
task_staging=$(mktemp -d)
trap 'rm -rf "$task_staging"' EXIT
# Headers live in an ExtendEngine/ subdirectory so this module map cannot collide
# with other module.modulemap files merged into a shared Headers directory.
mkdir -p "$task_staging/include/ExtendEngine"
cp "$task_header_dir/ExtendEngine.h" "$task_header_dir/module.modulemap" "$task_staging/include/ExtendEngine/"
rm -rf "$task_output"
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/$task_profile/libextend_ios_engine.a" -headers "$task_staging/include" \
  -library "target/aarch64-apple-ios-sim/$task_profile/libextend_ios_engine.a" -headers "$task_staging/include" \
  -output "$task_output" >/dev/null
echo "Built $task_output ($task_profile)"
