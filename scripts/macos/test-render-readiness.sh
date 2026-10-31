#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/render-readiness-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/display/RenderReadiness.swift scripts/macos/tests/render-readiness.swift \
    -o target/render-readiness-check/check
target/render-readiness-check/check
