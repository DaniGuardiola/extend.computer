#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
app="$PWD/target/extend.computer Cursor.app"
sdk=${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}
mkdir -p "$app/Contents/MacOS"
cp native/macos/CursorHelper.swift target/main.swift
# Build both architectures explicitly: uname reports x86_64 under Rosetta.
for arch in arm64 x86_64; do
    xcrun clang -isysroot "$sdk" -target "$arch-apple-macosx13.0" -c native/macos/CursorCompatibility.c -o "target/CursorCompatibility-$arch.o"
    xcrun swiftc -sdk "$sdk" -import-objc-header native/macos/CursorCompatibility.h "target/CursorCompatibility-$arch.o" -target "$arch-apple-macosx13.0" native/macos/InputModel.swift native/macos/InputCapture.swift native/macos/InputReceiver.swift native/macos/EdgeLayout.swift native/macos/EdgeCapture.swift target/main.swift -o "target/ExtendComputerCursor-$arch"
done
xcrun lipo -create target/ExtendComputerCursor-arm64 target/ExtendComputerCursor-x86_64 -output "$app/Contents/MacOS/ExtendComputerCursor"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>computer.extend.prototype.cursor</string>
<key>CFBundleName</key><string>extend.computer Cursor</string>
<key>CFBundleExecutable</key><string>ExtendComputerCursor</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/>
<key>NSInputMonitoringUsageDescription</key><string>Move the cursor across an arranged screen edge to your paired Mac during an approved session.</string>
</dict></plist>
PLIST
codesign --force --sign - --identifier computer.extend.prototype.cursor "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
printf '%s\n' "$app"
