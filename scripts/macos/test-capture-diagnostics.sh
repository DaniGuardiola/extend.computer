#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/capture-diagnostics-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/Display/CaptureDiagnostics.swift scripts/macos/tests/capture-diagnostics.swift \
    -o target/capture-diagnostics-check/check
target/capture-diagnostics-check/check
