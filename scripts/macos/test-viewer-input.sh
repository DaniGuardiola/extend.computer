#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target
xcrun swiftc -O -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/Input/InputModel.swift native/macos/Display/ViewerInput.swift \
    scripts/macos/tests/viewer-input.swift -o target/viewer-input-tests
target/viewer-input-tests
