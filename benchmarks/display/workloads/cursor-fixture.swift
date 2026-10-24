import AppKit
import ImageIO
import Darwin

final class CursorWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}
final class CursorView: NSView {
    var selected = NSCursor.arrow
    override func resetCursorRects() { addCursorRect(bounds, cursor: selected) }
}
func saveJSON(_ value: [String: Any], _ path: String) throws {
    try JSONSerialization.data(withJSONObject: value, options: .sortedKeys).write(to: URL(fileURLWithPath: path), options: .atomic)
}
func shape(_ cross: Bool) -> NSCursor {
    let context = CGContext(data: nil, width: 128, height: 128, bitsPerComponent: 8,
        bytesPerRow: 512, space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.setFillColor(CGColor(red: 0, green: 0, blue: 0, alpha: 1))
    if cross {
        context.fill(CGRect(x: 48, y: 0, width: 32, height: 128)); context.fill(CGRect(x: 0, y: 48, width: 128, height: 32))
        context.setFillColor(CGColor(red: 0, green: 1, blue: 1, alpha: 1))
        context.fill(CGRect(x: 56, y: 8, width: 16, height: 112)); context.fill(CGRect(x: 8, y: 56, width: 112, height: 16))
    } else {
        context.fill(CGRect(x: 0, y: 0, width: 128, height: 128))
        context.setFillColor(CGColor(red: 1, green: 0, blue: 1, alpha: 1)); context.fill(CGRect(x: 8, y: 8, width: 112, height: 112))
        // An asymmetric corner distinguishes flipped or rotated cursor images.
        context.setFillColor(CGColor(red: 0, green: 1, blue: 1, alpha: 1)); context.fill(CGRect(x: 16, y: 88, width: 24, height: 24))
        context.setFillColor(CGColor(red: 0, green: 0, blue: 0, alpha: 1)); context.fill(CGRect(x: 24, y: 96, width: 8, height: 8))
    }
    return NSCursor(image: NSImage(cgImage: context.makeImage()!, size: NSSize(width: 64, height: 64)),
                    hotSpot: cross ? NSPoint(x: 32, y: 32) : NSPoint(x: 8, y: 8))
}
final class CursorFixture: NSObject, NSApplicationDelegate {
    var window: NSWindow?, view: CursorView?, timer: Timer?
    var signals: [DispatchSourceSignal] = []
    var original: CGPoint?, hidden = false, sequence = 0
    var output = "", screen: NSScreen?, identifier: UInt32 = 0
    var inputMode = "warp"
    let box = shape(false), cross = shape(true)
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments
        guard (3...5).contains(args.count) else { exit(2) }; output = args[1]
        if args.count >= 4 { inputMode = args[3] }
        let grayBackground = args.count == 5 && args[4] == "gray"
        guard (args.count < 5 || grayBackground), ["warp", "mouse-events"].contains(inputMode),
              inputMode != "mouse-events" || CGPreflightPostEventAccess() else { exit(3) }
        let candidates = NSScreen.screens.filter {
            let id = ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
            return CGDisplayIsBuiltin(id) == 0 && CGDisplayIsInMirrorSet(id) == 0
        }
        screen = args[2] == "auto" && candidates.count == 1 ? candidates.first : candidates.first {
            String(($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0) == args[2]
        }
        guard let screen, let point = CGEvent(source: nil)?.location else { exit(2) }
        original = point; identifier = (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! NSNumber).uint32Value
        let w = CursorWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false, screen: screen)
        let v = CursorView(frame: NSRect(origin: .zero, size: screen.frame.size)); w.contentView = v; w.backgroundColor = grayBackground ? NSColor(calibratedWhite: 0.2, alpha: 1) : .black
        w.setFrame(screen.frame, display: true); w.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 2)
        w.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true); window = w; view = v
        for value in [SIGTERM, SIGINT] {
            signal(value, SIG_IGN); let item = DispatchSource.makeSignalSource(signal: value, queue: .main)
            item.setEventHandler { NSApp.terminate(nil) }; item.resume(); signals.append(item)
        }
        try? saveJSON(["status": "ready", "display_id": identifier, "logical_size": [screen.frame.width, screen.frame.height],
                       "scale": screen.backingScaleFactor, "input_method": inputMode], output + ".ready.json")
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in self?.readControl() }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 45) { NSApp.terminate(nil) }
    }
    func readControl() {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: output + ".control.json")),
              let command = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let next = command["sequence"] as? Int, next > sequence,
              let state = command["state"] as? String,
              ["box-left", "cross-center", "box-right", "box-top-left", "cross-bottom-right", "hidden", "outside"].contains(state),
              let screen, let window, let view else { return }
        sequence = next
        if hidden { NSCursor.unhide(); hidden = false }
        let selected = state.hasPrefix("cross-") ? cross : box
        view.selected = selected; window.invalidateCursorRects(for: view); view.resetCursorRects(); selected.set()
        let bounds = CGDisplayBounds(identifier)
        var point = CGPoint(x: bounds.minX + bounds.width * (state == "box-left" ? 0.25 : state == "box-right" ? 0.75 : 0.5), y: bounds.midY)
        if state == "box-top-left" { point = CGPoint(x: bounds.minX + 1, y: bounds.minY + 1) }
        if state == "cross-bottom-right" { point = CGPoint(x: bounds.maxX - 1, y: bounds.maxY - 1) }
        if state == "outside" { let main = CGDisplayBounds(CGMainDisplayID()); point = CGPoint(x: main.midX, y: main.midY) }
        if inputMode == "mouse-events" {
            guard let event = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved,
                                      mouseCursorPosition: point, mouseButton: .left) else {
                fail("Mouse movement event unavailable"); return
            }
            if let previous = CGEvent(source: nil)?.location {
                event.setIntegerValueField(.mouseEventDeltaX, value: Int64((point.x-previous.x).rounded()))
                event.setIntegerValueField(.mouseEventDeltaY, value: Int64((point.y-previous.y).rounded()))
            }
            event.post(tap: .cghidEventTap)
        } else {
            guard CGWarpMouseCursorPosition(point) == .success else { fail("Pointer warp failed"); return }
        }
        selected.set()
        if state == "hidden" { NSCursor.hide(); hidden = true }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [self] in
            guard sequence == next else { return }
            guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGCursorIsVisible") else { fail("Cursor visibility query unavailable"); return }
            let visibility = unsafeBitCast(symbol, to: (@convention(c) () -> Int32).self)
            let systemVisible = visibility() != 0
            guard systemVisible == (state != "hidden") else { fail("Source cursor visibility did not apply"); return }
            let current = NSCursor.currentSystem ?? .arrow
            guard let observedPoint=CGEvent(source:nil)?.location,
                  abs(observedPoint.x-point.x)<=1,abs(observedPoint.y-point.y)<=1 else {
                fail("Source cursor moved away from requested fixture position");return
            }
            guard state == "outside" || current.hotSpot == selected.hotSpot,
                  let tiff = selected.image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
                  let png = bitmap.representation(using: .png, properties: [:]) else { fail("Selected cursor not observed: active=\(NSApp.isActive), actual hotspot=\(current.hotSpot), size=\(current.image.size), expected hotspot=\(selected.hotSpot)"); return }
            let factor = screen.backingScaleFactor
            try? saveJSON(["status": "applied", "sequence": next, "state": state,
                "display_id": identifier, "logical_size": [bounds.width, bounds.height], "scale": factor, "input_method": inputMode,
                "visible": state != "hidden" && state != "outside", "observed_system_visible": systemVisible, "cursor_size": [64, 64],
                "hotspot": [selected.hotSpot.x, selected.hotSpot.y],
                "requested_point": [point.x,point.y],"observed_point":[observedPoint.x,observedPoint.y],
                "cursor_top_left_px": [(observedPoint.x - bounds.minX - selected.hotSpot.x) * factor, (observedPoint.y - bounds.minY - selected.hotSpot.y) * factor],
                "cursor_png_base64": png.base64EncodedString(), "applied_ns": Int64(ProcessInfo.processInfo.systemUptime * 1e9)], output + ".state.json")
        }
    }
    func fail(_ error: String) { try? saveJSON(["status": "failed", "error": error, "sequence": sequence], output + ".state.json"); NSApp.terminate(nil) }
    func applicationWillTerminate(_ notification: Notification) {
        timer?.invalidate(); if hidden { NSCursor.unhide() }; NSCursor.arrow.set()
        let restored = original.map { CGWarpMouseCursorPosition($0) == .success } ?? false
        window?.orderOut(nil)
        try? saveJSON(["position_restored": restored, "visibility_restored": true], output + ".cleanup.json")
    }
}
let app = NSApplication.shared; let fixture = CursorFixture(); app.setActivationPolicy(.accessory); app.delegate = fixture; app.run()
