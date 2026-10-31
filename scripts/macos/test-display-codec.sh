#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/display-codec-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/display/Metrics.swift native/macos/display/BenchmarkMarker.swift \
    native/macos/display/VideoCodec.swift scripts/macos/tests/display-codec.swift \
    -o target/display-codec-check/check
target/display-codec-check/check
