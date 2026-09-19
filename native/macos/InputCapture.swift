import Foundation
import CoreGraphics

final class InputCapture {
    let enabled: Bool
    var localKeys = Set<Int>()
    var remoteKeys = Set<Int>()
    var remoteButtons = Set<Int>()
    var lastModifiers = 0
    init(enabled: Bool) { self.enabled = enabled }
    static func modifiers(_ flags: CGEventFlags) -> Int {
        var mask = 0
        for (bit, flag) in [CGEventFlags.maskShift, .maskControl, .maskAlternate, .maskCommand].enumerated() {
            if flags.contains(flag) { mask |= 1 << bit }
        }
        return mask
    }
    func emit(_ packet: InputPacket) {
        guard let data = try? JSONEncoder().encode(packet), let line = String(data: data, encoding: .utf8) else { return }
        output(line)
    }
    func release() {
        guard enabled else { return }
        emit(InputPacket(kind: "release")); remoteButtons.removeAll(); lastModifiers = 0
        // Keep ownership until physical key-up, so a remotely pressed key cannot
        // leak an unmatched release/repeat into a local app after returning.
    }
    func canEnter(_ event: CGEvent) -> Bool {
        guard enabled else { return true }
        return localKeys.isEmpty && !(0..<128).contains { CGEventSource.keyState(.combinedSessionState, key: CGKeyCode($0)) } && Self.modifiers(event.flags) == 0 &&
            ![CGMouseButton.left, .right, .center].contains { CGEventSource.buttonState(.combinedSessionState, button: $0) }
    }
    func handle(_ type: CGEventType, _ event: CGEvent, remote: Bool) -> Bool {
        guard enabled else { return false }
        let code = Int(event.getIntegerValueField(.keyboardEventKeycode))
        if !remote {
            if type == .keyUp && remoteKeys.remove(code) != nil { return true }
            if type == .keyDown && remoteKeys.contains(code) { return true }
            if type == .keyDown { localKeys.insert(code) }
            if type == .keyUp { localKeys.remove(code) }
            return false
        }
        let mask = Self.modifiers(event.flags)
        if mask != lastModifiers { emit(InputPacket(kind: "modifiers", mask: mask)); lastModifiers = mask }
        switch type {
        case .keyDown, .keyUp:
            guard (0..<128).contains(code), ![54,55,56,57,58,59,60,61,62,63].contains(code) else { return true }
            let down = type == .keyDown
            if down { remoteKeys.insert(code) } else { remoteKeys.remove(code) }
            emit(InputPacket(kind: "mac_key", down: down, code: code,
                repeat: down && event.getIntegerValueField(.keyboardEventAutorepeat) != 0))
            return true
        case .flagsChanged: return true
        case .leftMouseDown, .rightMouseDown, .otherMouseDown, .leftMouseUp, .rightMouseUp, .otherMouseUp:
            let button = Int(event.getIntegerValueField(.mouseEventButtonNumber))
            guard (0...2).contains(button) else { return true }
            let down = [.leftMouseDown, .rightMouseDown, .otherMouseDown].contains(type)
            if down { remoteButtons.insert(button) } else { remoteButtons.remove(button) }
            emit(InputPacket(kind: "button", button: button, down: down,
                clicks: min(3, max(1, Int(event.getIntegerValueField(.mouseEventClickState))))))
            return true
        case .scrollWheel:
            let continuous = event.getIntegerValueField(.scrollWheelEventIsContinuous) != 0
            let x = event.getDoubleValueField(continuous ? .scrollWheelEventPointDeltaAxis2 : .scrollWheelEventDeltaAxis2) * (continuous ? 1 : 10)
            let y = event.getDoubleValueField(continuous ? .scrollWheelEventPointDeltaAxis1 : .scrollWheelEventDeltaAxis1) * (continuous ? 1 : 10)
            emit(InputPacket(kind: "scroll", dx: Int(min(4096, max(-4096, x))), dy: Int(min(4096, max(-4096, y)))))
            return true
        default: return false
        }
    }
}
