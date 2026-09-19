import Foundation
import CoreGraphics
import ApplicationServices

// Small native boundary: normalized main-display cursor position only.
func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}
func output(_ text: String) {
    FileHandle.standardOutput.write(Data((text + "\n").utf8))
}
let mode = CommandLine.arguments.dropFirst().first ?? "status"
if mode == "status" {
    output("listen=\(CGPreflightListenEventAccess()) post=\(CGPreflightPostEventAccess())")
    exit(0)
}
if mode == "request-listen" {
    output("listen=\(CGRequestListenEventAccess())")
    exit(0)
}
if mode == "request-post" {
    output("post=\(CGRequestPostEventAccess())")
    exit(0)
}
if ["capture-edge-left", "capture-edge-right", "capture-input-left", "capture-input-right", "capture-control-left", "capture-control-right"].contains(mode) {
    let args = CommandLine.arguments
    guard args.count == 5, let width = Double(args[2]), let height = Double(args[3]), let offset = Double(args[4]) else {
        fail("Edge capture needs remote width, height, and vertical offset in logical points.")
    }
    do { try runEdgeCapture(side: mode.hasSuffix("left") ? .left : .right,
                           remoteWidth: width, remoteHeight: height, offsetY: offset, fullInput: mode.hasPrefix("capture-input") || mode.hasPrefix("capture-control"), persistent: mode.hasPrefix("capture-control")) }
    catch { fail("Invalid display layout: \(error)") }
    exit(0)
}
if mode == "capture" {
    guard CGPreflightListenEventAccess() else { fail("Enable Input Monitoring for extend.computer Cursor, then restart.") }
    let mask = CGEventMask(1) << CGEventType.mouseMoved.rawValue
    guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap,
                                     options: .listenOnly, eventsOfInterest: mask,
                                     callback: { _, type, event, _ in
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput { exit(2) }
        // Never forward synthetic events from another extend.computer receiver.
        if type == .mouseMoved && event.getIntegerValueField(.eventSourceUserData) != 0x504f5254 {
            let bounds = CGDisplayBounds(CGMainDisplayID())
            let point = event.location
            if bounds.contains(point) {
                let x = (point.x - bounds.minX) / max(1, bounds.width - 1)
                let y = (point.y - bounds.minY) / max(1, bounds.height - 1)
                output("\(min(1, max(0, x))) \(min(1, max(0, y)))")
            }
        }
        return Unmanaged.passUnretained(event)
    }, userInfo: nil) else { fail("Cannot create listen-only cursor event tap.") }
    guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else { fail("Cannot create run loop source.") }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
    CGEvent.tapEnable(tap: tap, enable: true)
    let initialBounds = CGDisplayBounds(CGMainDisplayID())
    output("READY \(initialBounds.width) \(initialBounds.height)")
    CFRunLoopRunInMode(.defaultMode, 30, false)
    CFMachPortInvalidate(tap)
    exit(0)
}
if mode == "inject-input" || mode == "inject-control" { runInputReceiver(persistent: mode == "inject-control"); exit(0) }
if mode == "inject" {
    guard CGPreflightPostEventAccess() else { fail("Enable Accessibility for extend.computer Cursor, then restart.") }
    guard let source = CGEventSource(stateID: .privateState) else { fail("Cannot create event source.") }
    source.localEventsSuppressionInterval = 0
    source.userData = 0x504f5254
    let deadline = Date().addingTimeInterval(30)
    let initialBounds = CGDisplayBounds(CGMainDisplayID())
    output("READY \(initialBounds.width) \(initialBounds.height)")
    while let line = readLine() {
        guard Date() < deadline else { fail("Cursor grant expired.") }
        let parts = line.split(separator: " ")
        guard parts.count == 2, let x = Double(parts[0]), let y = Double(parts[1]),
              x.isFinite, y.isFinite, (0...1).contains(x), (0...1).contains(y) else { fail("Invalid cursor position.") }
        let bounds = CGDisplayBounds(CGMainDisplayID())
        guard bounds == initialBounds else { fail("Display layout changed; reconnect.") }
        let point = CGPoint(x: bounds.minX + x * max(1, bounds.width - 1), y: bounds.minY + y * max(1, bounds.height - 1))
        guard let event = CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left) else { fail("Cannot create cursor event.") }
        event.setIntegerValueField(.eventSourceUserData, value: 0x504f5254)
        event.post(tap: .cgSessionEventTap)
        output("OK")
    }
    exit(0)
}
fail("Usage: ExtendComputerCursor status|request-listen|request-post|capture|inject")
