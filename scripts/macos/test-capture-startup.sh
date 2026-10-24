#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/capture-startup-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/Display/CaptureStartupDeadline.swift scripts/macos/tests/capture-startup.swift \
    -o target/capture-startup-check/check
target/capture-startup-check/check
