import Foundation
import CoreGraphics
import ApplicationServices

private func activeEdgeDisplays() -> [EdgeDisplay]? {
    var identifiers = [CGDirectDisplayID](repeating: 0, count: 32)
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(UInt32(identifiers.count), &identifiers, &count) == .success else { return nil }
    return identifiers.prefix(Int(count)).map { EdgeDisplay(id: $0, bounds: CGDisplayBounds($0)) }
}

private final class EdgeCaptureContext {
    var layout: EdgeLayout
    let input: InputCapture
    let persistent: Bool
    var bounds: CGRect
    var display: CGDirectDisplayID
    let displays: [EdgeDisplay]
    let runLoop: CFRunLoop
    var lastAck = ProcessInfo.processInfo.systemUptime
    let started = ProcessInfo.processInfo.systemUptime
    let canHideCursor = ExtendComputerEnableBackgroundCursorVisibility()
    var cursorHidden = false
    var stopped = false
    let diagnostics = ProcessInfo.processInfo.environment["EXTEND_COMPUTER_EDGE_DIAGNOSTICS"] == "1"
    var returnTime: TimeInterval?
    var returnGaps = [Double]()
    var returnAttemptsOutsideOverlap = 0
    var cursorProbeUntil: TimeInterval = 0
    var cursorProbeRows = [String]()

    init(layout: EdgeLayout, bounds: CGRect, display: CGDirectDisplayID, displays: [EdgeDisplay], fullInput: Bool, persistent: Bool) {
        self.persistent = persistent
        self.input = InputCapture(enabled: fullInput)
        self.layout = layout; self.bounds = bounds; self.display = display
        self.displays = displays
        self.runLoop = CFRunLoopGetCurrent()
    }
    func hideCursor() {
        guard canHideCursor, !cursorHidden else { return }
        if CGDisplayHideCursor(display) == .success { cursorHidden = true }
    }
    func restoreCursor() {
        guard cursorHidden else { return }
        if CGDisplayShowCursor(display) == .success { cursorHidden = false }
    }
    func warp(_ point: EdgePoint) {
        CGWarpMouseCursorPosition(CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y))
    }
    func stop(userRequested: Bool = false, reason: String = "input-closed") {
        guard !stopped else { return }
        stopped = true
        input.release()
        restoreCursor()
        if let point = layout.cancel() { warp(point) }
        output(userRequested ? "STOP user" : "STOP \(reason)")
        CFRunLoopStop(runLoop)
    }
    func exposed(at y: Double) -> Bool {
        edgeIsExposed(display: EdgeDisplay(id: display, bounds: bounds), side: layout.side, y: y, in: displays)
    }
    func selectDisplay(at point: CGPoint) -> Bool {
        guard let selected = edgeDisplay(at: point, in: displays) else { return false }
        if selected.id == display { return true }
        guard let next = try? EdgeLayout(localWidth: selected.bounds.width, localHeight: selected.bounds.height,
                                        remoteWidth: layout.remoteWidth, remoteHeight: layout.remoteHeight,
                                        side: layout.side, offsetY: layout.offsetY) else { return false }
        // Keep the source display fixed while remote, so return/cancel restores
        // the cursor to the display that actually handed control over.
        layout = next; bounds = selected.bounds; display = selected.id
        return true
    }
    func handle(_ type: CGEventType, _ event: CGEvent) -> Unmanaged<CGEvent>? {
        if stopped { return Unmanaged.passUnretained(event) }
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            stop(reason: "capture-disabled"); return Unmanaged.passUnretained(event)
        }
        if CGDisplayIsActive(display) == 0 || CGDisplayBounds(display) != bounds {
            stop(reason: "display-changed"); return Unmanaged.passUnretained(event)
        }
        if type == .keyDown, event.getIntegerValueField(.keyboardEventKeycode) == 53,
           event.flags.contains(.maskControl), event.flags.contains(.maskAlternate) {
            stop(userRequested: true); return nil
        }
        if event.getIntegerValueField(.eventSourceUserData) == 0x504f5254 {
            return Unmanaged.passUnretained(event)
        }
        if input.handle(type, event, remote: layout.remotePosition != nil) { return nil }
        if type == .mouseMoved || (input.enabled && [.leftMouseDragged, .rightMouseDragged, .otherMouseDragged].contains(type)) {
            if layout.remotePosition == nil && !selectDisplay(at: event.location) { return Unmanaged.passUnretained(event) }
            let now = ProcessInfo.processInfo.systemUptime
            if diagnostics && now < cursorProbeUntil && cursorProbeRows.count < 256 {
                let visible = CGEvent(source: nil)?.location ?? .zero
                cursorProbeRows.append("remaining_ms=\((cursorProbeUntil - now) * 1000) event_x=\(event.location.x) cursor_x=\(visible.x) dx=\(event.getIntegerValueField(.mouseEventDeltaX))")
            }
            if diagnostics, let returnedAt = returnTime {
                if returnGaps.count < 64 {
                    returnGaps.append((ProcessInfo.processInfo.systemUptime - returnedAt) * 1000)
                }
                returnTime = nil
            }
            let local = EdgePoint(x: event.location.x - bounds.minX, y: event.location.y - bounds.minY)
            let active = layout.remotePosition != nil
            if !active && !bounds.contains(event.location) { return Unmanaged.passUnretained(event) }
            if diagnostics, let remote = layout.remotePosition {
                let dx = Double(event.getIntegerValueField(.mouseEventDeltaX))
                let dy = Double(event.getIntegerValueField(.mouseEventDeltaY))
                let y = min(layout.remoteHeight - 1, max(0, remote.y + dy)) + layout.offsetY
                let crosses = layout.side == .left ? remote.x + dx >= layout.remoteWidth : remote.x + dx < 0
                if crosses && (y < 0 || y >= layout.localHeight) { returnAttemptsOutsideOverlap += 1 }
            }
            let outcome = layout.motion(local: local,
                dx: Double(event.getIntegerValueField(.mouseEventDeltaX)),
                dy: Double(event.getIntegerValueField(.mouseEventDeltaY)),
                exposed: active || ((layout.side == .left ? local.x <= 1 : local.x >= bounds.width - 2) && input.canEnter(event) && exposed(at: local.y)),
                allowReturn: input.remoteButtons.isEmpty)
            switch outcome {
            case .local: return Unmanaged.passUnretained(event)
            case .remote(let point):
                if !active { hideCursor(); output("REMOTE") }
                output("\(point.x) \(point.y)")
                // Suppress local motion while the peer owns cursor control.
                return nil
            case .returned(let point):
                input.release()
                restoreCursor()
                warp(point)
                returnTime = ProcessInfo.processInfo.systemUptime
                cursorProbeUntil = ProcessInfo.processInfo.systemUptime + 0.35
                output("LOCAL"); return nil
            }
        }
        if layout.remotePosition != nil && ![CGEventType.keyDown, .keyUp, .flagsChanged].contains(type) {
            // Cursor-only milestone: remote clicks/scrolling are not implemented.
            // Do not let those events accidentally act on the parked local pointer.
            return nil
        }
        return Unmanaged.passUnretained(event)
    }
}

func runEdgeCapture(side: EdgeSide, remoteWidth: Double, remoteHeight: Double, offsetY: Double, fullInput: Bool = false, persistent: Bool = false) throws {
    guard CGPreflightListenEventAccess(), CGPreflightPostEventAccess() else {
        fail("Edge handoff requires Input Monitoring and Accessibility for extend.computer Cursor on this Mac.")
    }
    // CGWarpMouseCursorPosition uses the legacy per-process suppression
    // interval. A private CGEventSource setting does not configure this warp.
    // Measured hardware movement continued while the cursor stayed frozen
    // for approximately 250 ms after returning from the peer.
    guard ExtendComputerDisableWarpSuppression() == .success else {
        fail("Cannot disable cursor-warp suppression for edge handoff.")
    }
    let display = CGMainDisplayID()
    let bounds = CGDisplayBounds(display)
    guard let displays = activeEdgeDisplays() else { fail("Cannot read local display layout.") }
    let layout = try EdgeLayout(localWidth: bounds.width, localHeight: bounds.height,
                                remoteWidth: remoteWidth, remoteHeight: remoteHeight,
                                side: side, offsetY: offsetY)
    let context = EdgeCaptureContext(layout: layout, bounds: bounds, display: display, displays: displays, fullInput: fullInput, persistent: persistent)
    let events: [CGEventType] = [.mouseMoved, .leftMouseDown, .leftMouseUp, .rightMouseDown,
        .rightMouseUp, .otherMouseDown, .otherMouseUp, .leftMouseDragged, .rightMouseDragged,
        .otherMouseDragged, .scrollWheel, .keyDown, .keyUp, .flagsChanged]
    let mask = events.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
    guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap,
        options: .defaultTap, eventsOfInterest: mask, callback: { _, type, event, info in
            guard let info else { return Unmanaged.passUnretained(event) }
            return Unmanaged<EdgeCaptureContext>.fromOpaque(info).takeUnretainedValue().handle(type, event)
        }, userInfo: Unmanaged.passUnretained(context).toOpaque()) else {
        fail("Cannot create edge-control event tap. Check Accessibility permission.")
    }
    guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else { fail("Cannot create edge run loop.") }
    CFRunLoopAddSource(context.runLoop, source, .commonModes)
    let timer = CFRunLoopTimerCreateWithHandler(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 0.1, 0.1, 0, 0) { _ in
        let now = ProcessInfo.processInfo.systemUptime
        if activeEdgeDisplays() != context.displays { context.stop(reason: "display-changed"); return }
        if now - context.lastAck > 2 { context.stop(reason: "heartbeat-timeout") }
        else if !context.persistent && now - context.started >= 30 { context.stop(reason: "duration-limit") }
    }!
    CFRunLoopAddTimer(context.runLoop, timer, .commonModes)
    DispatchQueue.global().async {
        while let line = readLine() {
            let side = line.hasPrefix("EDGE ") ? EdgeSide(rawValue: String(line.dropFirst(5))) : nil
            let valid = line == "ALIVE" || side != nil
            CFRunLoopPerformBlock(context.runLoop, CFRunLoopMode.commonModes.rawValue) {
                if let side { context.layout.setSide(side) }
                else if valid { context.lastAck = ProcessInfo.processInfo.systemUptime }
                else { context.stop(reason: "invalid-command") }
            }
            CFRunLoopWakeUp(context.runLoop)
            if !valid { return }
        }
        CFRunLoopPerformBlock(context.runLoop, CFRunLoopMode.commonModes.rawValue) { context.stop() }
        CFRunLoopWakeUp(context.runLoop)
    }
    CGEvent.tapEnable(tap: tap, enable: true)
    output("READY \(bounds.width) \(bounds.height)")
    withExtendedLifetime(context) { CFRunLoopRun() }
    context.input.release()
    context.restoreCursor()
    if let point = context.layout.cancel() { context.warp(point) }
    CFRunLoopTimerInvalidate(timer)
    CGEvent.tapEnable(tap: tap, enable: false)
    CFMachPortInvalidate(tap)
    let report = "Edge diagnostics: return-to-next-move-ms=\(context.returnGaps) blocked-gap-events=\(context.returnAttemptsOutsideOverlap)\n"
    if context.diagnostics {
        FileHandle.standardError.write(Data(report.utf8))
        FileHandle.standardError.write(Data((context.cursorProbeRows.joined(separator: "\n") + "\n").utf8))
    }
}
