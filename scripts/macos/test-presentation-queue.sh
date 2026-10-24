#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/presentation-queue-check
xcrun swiftc -O -parse-as-library -module-cache-path "$PWD/target/DisplayModuleCache" \
    native/macos/Display/PresentationQueue.swift scripts/macos/tests/presentation-queue.swift \
    -o target/presentation-queue-check/check
target/presentation-queue-check/check
