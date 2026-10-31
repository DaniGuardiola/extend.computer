#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
app="$PWD/target/extend.computer Display.app"
sdk=$(xcrun --sdk macosx --show-sdk-path)
mkdir -p "$app/Contents/MacOS"
for arch in arm64 x86_64; do
    xcrun clang -fobjc-arc -isysroot "$sdk" -target "$arch-apple-macosx13.0" -c native/macos/display/VirtualDisplay.m -o "target/VirtualDisplay-$arch.o"
    xcrun swiftc -O -module-cache-path "$PWD/target/DisplayModuleCache" -parse-as-library -sdk "$sdk" -target "$arch-apple-macosx13.0" \
        -import-objc-header native/macos/display/VirtualDisplay.h "target/VirtualDisplay-$arch.o" \
        native/macos/display/Metrics.swift native/macos/display/BenchmarkMarker.swift native/macos/display/VideoCodec.swift native/macos/display/CaptureDiagnostics.swift native/macos/display/CaptureStartupDeadline.swift native/macos/display/CaptureLifecycle.swift native/macos/display/Capture.swift \
        native/macos/display/Cursor.swift native/macos/display/PresentationQueue.swift native/macos/display/RenderReadiness.swift native/macos/input/InputModel.swift \
        native/macos/display/ViewerInput.swift native/macos/display/Viewer.swift native/macos/display/Main.swift \
        -o "target/ExtendComputerDisplay-$arch"
done
xcrun lipo -create target/ExtendComputerDisplay-arm64 target/ExtendComputerDisplay-x86_64 -output "$app/Contents/MacOS/ExtendComputerDisplay"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>computer.extend.display</string>
<key>CFBundleName</key><string>extend.computer Display</string>
<key>CFBundleExecutable</key><string>ExtendComputerDisplay</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>Extend or mirror your display to a connected device.</string>
</dict></plist>
PLIST
codesign --force --sign - --identifier computer.extend.display "$app"
printf '%s\n' "$app"
