#!/bin/sh
set -eu
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir="$package_dir/.build/standalone"
mkdir -p "$build_dir"
# Do not trust uname under Rosetta: compile the native Apple Silicon target.
if [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    target=arm64-apple-macos13.0
else
    target=x86_64-apple-macos13.0
fi
swiftc -target "$target" -module-cache-path "$build_dir/module-cache" \
    "$package_dir"/Sources/LowJitterLifecycle/*.swift \
    "$package_dir/Tests/LowJitterLifecycleTests/LeaseControllerTests.swift" \
    -o "$build_dir/lifecycle-tests"
"$build_dir/lifecycle-tests"
