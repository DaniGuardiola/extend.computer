#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p target/edge-tests
xcrun swiftc -module-cache-path "$PWD/target/edge-tests/cache" native/macos/EdgeLayout.swift native/macos/tests/LayoutTests.swift -o target/edge-tests/layout-tests
target/edge-tests/layout-tests
