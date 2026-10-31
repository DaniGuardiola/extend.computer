#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
app="$PWD/target/extend.computer Display.app"
sdk=$(xcrun --sdk macosx --show-sdk-path)
mkdir -p "$app/Contents/MacOS"
for arch in arm64 x86_64; do
    xcrun clang -fobjc-arc -isysroot "$sdk" -target "$arch-apple-macosx13.0" -c native/macos/Display/VirtualDisplay.m -o "target/VirtualDisplay-$arch.o"
    xcrun swiftc -O -module-cache-path "$PWD/target/DisplayModuleCache" -parse-as-library -sdk "$sdk" -target "$arch-apple-macosx13.0" \
        -import-objc-header native/macos/Display/VirtualDisplay.h "target/VirtualDisplay-$arch.o" \
        native/macos/Display/Metrics.swift native/macos/Display/BenchmarkMarker.swift native/macos/Display/VideoCodec.swift native/macos/Display/CaptureDiagnostics.swift native/macos/Display/CaptureStartupDeadline.swift native/macos/Display/CaptureLifecycle.swift native/macos/Display/Capture.swift \
        native/macos/Display/Cursor.swift native/macos/Display/PresentationQueue.swift native/macos/Display/RenderReadiness.swift native/macos/Input/InputModel.swift \
        native/macos/Display/ViewerInput.swift native/macos/Display/Viewer.swift native/macos/Display/Main.swift \
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
