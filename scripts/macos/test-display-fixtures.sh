#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
mkdir -p target/display-fixture-tests
# Cursor.swift writes packets through Main.swift and fails with VideoCodec.swift's error type.
cat > target/display-fixture-tests/stubs.swift <<'EOF'
import Foundation
func packet(_ kind: UInt8, _ payload: Data, cursorIdentity: UInt64? = nil, cursorShape: Bool = false) {}
struct DisplayFailure: Error { init(_ description: String) {} }
EOF
xcrun swiftc -module-cache-path "$PWD/target/display-fixture-tests/cache" \
    native/macos/input/InputModel.swift native/macos/input/DisplayInputModel.swift \
    native/macos/display/Metrics.swift native/macos/display/Cursor.swift native/macos/display/ViewerInput.swift \
    target/display-fixture-tests/stubs.swift native/macos/tests/DisplayHelperFixtureTests.swift \
    -o target/display-fixture-tests/display-fixture-tests
target/display-fixture-tests/display-fixture-tests
