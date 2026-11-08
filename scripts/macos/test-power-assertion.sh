#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/power-assertion-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/display/PowerAssertion.swift scripts/macos/tests/power-assertion.swift \
    -o target/power-assertion-check/check
target/power-assertion-check/check
