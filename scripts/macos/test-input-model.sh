#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/input-tests
# InputCapture writes lines through the helper's output function.
printf 'func output(_ text: String) { print(text) }\n' > target/input-tests/output.swift
xcrun swiftc -module-cache-path "$PWD/target/input-tests/cache" native/macos/Input/InputModel.swift native/macos/Input/InputCapture.swift target/input-tests/output.swift native/macos/tests/InputTests.swift -o target/input-tests/input-tests
target/input-tests/input-tests
