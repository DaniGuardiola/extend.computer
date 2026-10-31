#!/bin/sh
set -eu
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir="$package_dir/.build/development"
mkdir -p "$build_dir"
if [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    target=arm64-apple-macos13.0
else
    target=x86_64-apple-macos13.0
fi
swiftc -target "$target" -module-cache-path "$build_dir/module-cache" \
    "$package_dir"/Sources/LowJitterLifecycle/*.swift \
    "$package_dir/Helper/main.swift" -o "$build_dir/ExtendComputerLowJitter"
codesign --force --sign - --options runtime \
    --identifier computer.extend.lowjitter-development "$build_dir/ExtendComputerLowJitter"
codesign --verify --strict "$build_dir/ExtendComputerLowJitter"
/usr/bin/python3 "$package_dir/package-development.py" "$build_dir"
printf '%s\n' "Development installer prepared: $build_dir/install.py"
