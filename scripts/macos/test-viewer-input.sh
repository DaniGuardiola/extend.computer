#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target
xcrun swiftc -O -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/input/InputModel.swift native/macos/display/ViewerInput.swift \
    scripts/macos/tests/viewer-input.swift -o target/viewer-input-tests
target/viewer-input-tests
