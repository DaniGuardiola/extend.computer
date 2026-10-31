import Foundation
import CoreGraphics
import Darwin

private final class InputReceiver {
    let source: CGEventSource
    let bounds: CGRect
    let display: CGDirectDisplayID
    var binding: DisplayInputBinding?
    let parent = getppid()
    let loop = CFRunLoopGetCurrent()
    var point = CGEvent(source: nil)?.location ?? .zero
    var state = InputState()
    var lastInput = ProcessInfo.processInfo.systemUptime
    let dryRun = ProcessInfo.processInfo.environment["EXTEND_COMPUTER_INPUT_DRY_RUN"] == "1"
    var stopped = false
    var eventNumber: Int64 = 1
    var buttonNumbers = [Int: Int64]()
    init(target: DisplayInputTarget? = nil) {
        let selectedDisplay = target?.display_id ?? CGMainDisplayID()
        self.display = selectedDisplay
        self.bounds = target?.bounds.cgRect ?? CGDisplayBounds(selectedDisplay)
        self.binding = target.map { DisplayInputBinding(target: $0) }
        guard let source = CGEventSource(stateID: .privateState) else { fail("Cannot create input event source.") }
        self.source = source
        if target != nil { point = CGPoint(x: bounds.minX, y: bounds.minY) }
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
            case .text(let value):
                // Final text only; physical shortcuts continue through .key.
                for down in [true, false] {
                    let event = CommittedTextEvent.make(source: source, value: value, down: down)
                    post(event, mask: 0)
                }
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
                if binding != nil { event?.location = point }
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
    func targetValid() -> Bool {
        CGDisplayIsActive(display) != 0 && CGDisplayBounds(display) == bounds
            && (binding != nil || CGMainDisplayID() == display)
    }
    func move(to position: CGPoint) {
        point = position
        let button = state.buttons.sorted().first
        let type: CGEventType = button == 0 ? .leftMouseDragged : button == 1 ? .rightMouseDragged : button == 2 ? .otherMouseDragged : .mouseMoved
        let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point,
                            mouseButton: CGMouseButton(rawValue: UInt32(button ?? 0))!)
        if let button { event?.setIntegerValueField(.mouseEventNumber, value: buttonNumbers[button] ?? 0) }
        post(event, mask: state.modifiers)
    }
    func receive(_ line: String) {
        guard !stopped else { return }
        guard targetValid() else { stop(); return }
        do {
            if var scoped = binding {
                guard CGPreflightPostEventAccess() else { stop(); return }
                let command = try DisplayInputEnvelope.decode(Data(line.utf8))
                let position = try scoped.accept(command)
                binding = scoped
                // Button/scroll packets include their target for validation.
                // Repeating an unchanged target while a button is held would
                // manufacture a drag event for an ordinary tap.
                if let position, command.event.kind == "move" || position != point {
                    move(to: position)
                }
                guard !stopped else { return }
                if let packet = command.event.packet { perform(try state.apply(packet)) }
                if !stopped {
                    lastInput = ProcessInfo.processInfo.systemUptime
                    output("APPLIED {\"session\":\"\(scoped.target.session)\",\"epoch\":\(scoped.target.epoch),\"revision\":\(scoped.target.revision),\"sequence\":\(command.sequence)}")
                }
                return
            }
            if line == "ALIVE" { }
            else if line.hasPrefix("{") {
                let packet = try InputPacket.decode(Data(line.utf8))
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

func runInputReceiver(target: DisplayInputTarget? = nil) {
    guard CGPreflightPostEventAccess() else { fail("Enable Accessibility for extend.computer Cursor, then restart.") }
    if let target {
        guard CGDisplayIsActive(target.display_id) != 0, CGDisplayBounds(target.display_id) == target.bounds.cgRect else {
            fail("Captured display is inactive or its geometry changed.")
        }
    }
    let receiver = InputReceiver(target: target)
    // Shutdown must run on the main loop before launchd terminates this helper.
    let signals = [SIGTERM, SIGINT].map { number -> DispatchSourceSignal in
        signal(number, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
        source.setEventHandler { receiver.stop() }; source.resume(); return source
    }
    let timer = CFRunLoopTimerCreateWithHandler(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 0.1, 0.1, 0, 0) { _ in
        let now = ProcessInfo.processInfo.systemUptime
        if now - receiver.lastInput > 2
            || (target != nil && (!receiver.targetValid() || getppid() != receiver.parent)) { receiver.stop() }
    }!
    CFRunLoopAddTimer(receiver.loop, timer, .commonModes)
    DispatchQueue.global().async {
        var pending = Data()
        func nextLine() throws -> String? {
            guard target != nil else { return readLine() }
            while true {
                if let newline = pending.firstIndex(of: 10) {
                    let count = pending.distance(from: pending.startIndex, to: newline)
                    guard count <= 1024, let line = String(data: pending.prefix(count), encoding: .utf8) else {
                        throw NSError(domain: "DisplayInputLine", code: 1)
                    }
                    pending.removeFirst(count + 1)
                    return line
                }
                guard pending.count <= 1024 else { throw NSError(domain: "DisplayInputSize", code: 1) }
                // FileHandle.read(upToCount:) can wait to fill its requested
                // size on a pipe. A single command must be handled immediately,
                // without waiting for another command to satisfy that read.
                var bytes = [UInt8](repeating: 0, count: 256)
                let count = bytes.withUnsafeMutableBytes { buffer in
                    Darwin.read(STDIN_FILENO, buffer.baseAddress, buffer.count)
                }
                if count < 0 {
                    if errno == EINTR { continue }
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
                }
                guard count > 0 else {
                    guard pending.isEmpty else { throw NSError(domain: "DisplayInputTruncated", code: 1) }
                    return nil
                }
                pending.append(contentsOf: bytes.prefix(count))
            }
        }
        while let line = try? nextLine() {
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
    if let target {
        let encoded = try! JSONEncoder().encode(target)
        var value = try! JSONSerialization.jsonObject(with: encoded) as! [String: Any]
        value["text_supported"] = true
        value["shortcut_policy"] = "command"
        let ready = try! JSONSerialization.data(withJSONObject: value)
        output("READY " + String(data: ready, encoding: .utf8)!)
    } else { output("READY \(receiver.bounds.width) \(receiver.bounds.height) input-text-v1 input-shortcuts-command-v1") }
    withExtendedLifetime(signals) { CFRunLoopRun() }
    receiver.stop()
    CFRunLoopTimerInvalidate(timer)
    for source in signals { source.cancel() }
}
