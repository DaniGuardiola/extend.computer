import Foundation
import CoreGraphics
import Darwin

private final class InputReceiver {
    let source: CGEventSource
    let persistent: Bool
    let bounds = CGDisplayBounds(CGMainDisplayID())
    let display = CGMainDisplayID()
    let loop = CFRunLoopGetCurrent()
    var point = CGEvent(source: nil)?.location ?? .zero
    var state = InputState()
    var lastInput = ProcessInfo.processInfo.systemUptime
    let started = ProcessInfo.processInfo.systemUptime
    let dryRun = ProcessInfo.processInfo.environment["EXTEND_COMPUTER_INPUT_DRY_RUN"] == "1"
    var stopped = false
    var eventNumber: Int64 = 1
    var buttonNumbers = [Int: Int64]()
    init(persistent: Bool) {
        self.persistent = persistent
        guard let source = CGEventSource(stateID: .privateState) else { fail("Cannot create input event source.") }
        self.source = source
        source.localEventsSuppressionInterval = 0
        source.userData = 0x504f5254
    }
    func flags(_ mask: Int) -> CGEventFlags {
        var result = CGEventFlags()
        for (bit, flag) in [CGEventFlags.maskShift, .maskControl, .maskAlternate, .maskCommand].enumerated() {
            if mask & (1 << bit) != 0 { result.insert(flag) }
        }
        return result
    }
    func post(_ event: CGEvent?, mask: Int) {
        guard let event else { stop(); return }
        if dryRun { return }
        event.flags = flags(mask)
        event.setIntegerValueField(.eventSourceUserData, value: 0x504f5254)
        event.post(tap: .cghidEventTap)
    }
    func perform(_ actions: [InputAction]) {
        for action in actions {
            switch action {
            case .key(let code, let down, let repeating, let mask):
                let event = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(code), keyDown: down)
                if InputState.modifierCodes.contains(code) { event?.type = .flagsChanged }
                event?.setIntegerValueField(.keyboardEventAutorepeat, value: repeating ? 1 : 0)
                post(event, mask: mask)
            case .button(let button, let down, let clicks, let mask):
                let types: [(CGEventType, CGEventType)] = [(.leftMouseDown,.leftMouseUp),(.rightMouseDown,.rightMouseUp),(.otherMouseDown,.otherMouseUp)]
                if down { buttonNumbers[button] = eventNumber; eventNumber += 1 }
                let event = CGEvent(mouseEventSource: source, mouseType: down ? types[button].0 : types[button].1,
                                    mouseCursorPosition: point, mouseButton: CGMouseButton(rawValue: UInt32(button))!)
                event?.setIntegerValueField(.mouseEventClickState, value: Int64(clicks))
                event?.setIntegerValueField(.mouseEventNumber, value: buttonNumbers[button] ?? eventNumber)
                post(event, mask: mask)
                if !down { buttonNumbers.removeValue(forKey: button) }
            case .scroll(let dx, let dy, let mask):
                let event = CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2, wheel1: Int32(dy), wheel2: Int32(dx), wheel3: 0)
                post(event, mask: mask)
            }
        }
    }
    func stop() {
        guard !stopped else { return }
        stopped = true
        perform(state.release())
        if dryRun { output("RELEASED keys=\(state.keys.count) buttons=\(state.buttons.count) modifiers=\(state.modifiers)") }
        output("STOP")
        CFRunLoopStop(loop)
    }
    func receive(_ line: String) {
        guard !stopped else { return }
        guard CGMainDisplayID() == display, CGDisplayBounds(display) == bounds else { stop(); return }
        do {
            if line == "ALIVE" { }
            else if line.hasPrefix("{") {
                let packet = try JSONDecoder().decode(InputPacket.self, from: Data(line.utf8))
                perform(try state.apply(packet))
            } else {
                let parts = line.split(separator: " ")
                guard parts.count == 2, let x = Double(parts[0]), let y = Double(parts[1]),
                      x.isFinite, y.isFinite, (0...1).contains(x), (0...1).contains(y) else { stop(); return }
                point = CGPoint(x: bounds.minX + x * (bounds.width - 1), y: bounds.minY + y * (bounds.height - 1))
                let button = state.buttons.sorted().first
                let type: CGEventType = button == 0 ? .leftMouseDragged : button == 1 ? .rightMouseDragged : button == 2 ? .otherMouseDragged : .mouseMoved
                let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point,
                                    mouseButton: CGMouseButton(rawValue: UInt32(button ?? 0))!)
                if let button { event?.setIntegerValueField(.mouseEventNumber, value: buttonNumbers[button] ?? 0) }
                post(event, mask: state.modifiers)
            }
            if !stopped { lastInput = ProcessInfo.processInfo.systemUptime; output("OK") }
        } catch { stop() }
    }
}

func runInputReceiver(persistent: Bool = false) {
    guard CGPreflightPostEventAccess() else { fail("Enable Accessibility for extend.computer Cursor, then restart.") }
    let receiver = InputReceiver(persistent: persistent)
    // Shutdown must run on the main loop before launchd terminates this helper.
    let signals = [SIGTERM, SIGINT].map { number -> DispatchSourceSignal in
        signal(number, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
        source.setEventHandler { receiver.stop() }; source.resume(); return source
    }
    let timer = CFRunLoopTimerCreateWithHandler(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 0.1, 0.1, 0, 0) { _ in
        let now = ProcessInfo.processInfo.systemUptime
        if now - receiver.lastInput > 2 || (!receiver.persistent && now - receiver.started >= 30) { receiver.stop() }
    }!
    CFRunLoopAddTimer(receiver.loop, timer, .commonModes)
    DispatchQueue.global().async {
        while let line = readLine() {
            let applied = DispatchSemaphore(value: 0)
            CFRunLoopPerformBlock(receiver.loop, CFRunLoopMode.commonModes.rawValue) {
                receiver.receive(line); applied.signal()
            }
            CFRunLoopWakeUp(receiver.loop)
            if applied.wait(timeout: .now() + 2) == .timedOut { return }
        }
        CFRunLoopPerformBlock(receiver.loop, CFRunLoopMode.commonModes.rawValue) { receiver.stop() }
        CFRunLoopWakeUp(receiver.loop)
    }
    output("READY \(receiver.bounds.width) \(receiver.bounds.height)")
    withExtendedLifetime(signals) { CFRunLoopRun() }
    receiver.stop()
    CFRunLoopTimerInvalidate(timer)
    for source in signals { source.cancel() }
}
