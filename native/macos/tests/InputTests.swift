import Foundation
import CoreGraphics

@main struct InputTests {
    static func require(_ value: Bool, line: Int = #line) { precondition(value, "failed at line \(line)") }
    static func main() throws {
        for value in ["Aaé", "e\u{301}", "🙂", "👩‍💻", "中文", String(repeating: "🙂", count: 64)] {
            let packet = InputPacket(kind: "text", value: value)
            var textState = InputState()
            require(try textState.apply(packet) == [.text(value)])
            for down in [true, false] {
                let event = CommittedTextEvent.make(source: nil, value: value, down: down)!
                var count = 0
                event.keyboardGetUnicodeString(maxStringLength: 0, actualStringLength: &count, unicodeString: nil)
                var units = [UniChar](repeating: 0, count: count)
                event.keyboardGetUnicodeString(maxStringLength: count, actualStringLength: &count, unicodeString: &units)
                require(String(decoding: units, as: UTF16.self) == value)
            }
        }
        for value in ["", "x\u{0}", String(repeating: "x", count: 65), String(repeating: "🙂", count: 65)] {
            do { try InputPacket(kind: "text", value: value).validate(); fatalError("invalid text accepted") } catch { }
        }
        var heldText = InputState()
        _ = try heldText.apply(InputPacket(kind: "modifiers", mask: 1))
        do { _ = try heldText.apply(InputPacket(kind: "text", value: "é")); fatalError("text with held modifier accepted") } catch { }
        _ = heldText.release()
        require(try heldText.apply(InputPacket(kind: "text", value: "é")) == [.text("é")])
        var state = InputState()
        let button = InputPacket(kind: "button", button: 0, down: true, clicks: 1)
        require(try state.apply(button) == [.button(0, true, 1, 0)])
        require(try state.apply(button).isEmpty)
        require(try state.apply(InputPacket(kind: "modifiers", mask: 9)) == [.key(56, true, false, 1), .key(55, true, false, 9)])
        require(try state.apply(InputPacket(kind: "mac_key", down: true, code: 0, repeat: false)) == [.key(0, true, false, 9)])
        require(try state.apply(InputPacket(kind: "mac_key", down: true, code: 0, repeat: true)) == [.key(0, true, true, 9)])
        let released = state.release()
        require(released == [.key(0, false, false, 9), .button(0, false, 1, 9), .key(56, false, false, 8), .key(55, false, false, 0)])
        require(state.keys.isEmpty && state.buttons.isEmpty && state.modifiers == 0)
        require(state.release().isEmpty)
        require(try state.apply(InputPacket(kind: "mac_key", down: false, code: 0, repeat: false)).isEmpty)
        require(try state.apply(InputPacket(kind: "mac_key", down: true, code: 0, repeat: true)).isEmpty)
        for invalid in [InputPacket(kind: "button", button: 3, down: true, clicks: 1), InputPacket(kind: "scroll", dx: 0, dy: 100000), InputPacket(kind: "mac_key", down: true, code: 65535, repeat: false), InputPacket(kind: "modifiers", mask: 256), InputPacket(kind: "modifiers", mask: 0x40)] {
            do { _ = try state.apply(invalid); fatalError("invalid input accepted") } catch { }
        }
        // Right-hand modifiers press their own keys and can switch sides.
        var sided = InputState()
        require(try sided.apply(InputPacket(kind: "modifiers", mask: 0x44)) == [.key(61, true, false, 4)])
        require(try sided.apply(InputPacket(kind: "modifiers", mask: 0x04)) == [.key(61, false, false, 0), .key(58, true, false, 4)])
        require(try sided.apply(InputPacket(kind: "modifiers", mask: 0x99)) == [.key(60, true, false, 5), .key(58, false, false, 1), .key(54, true, false, 9)])
        require(sided.release() == [.key(60, false, false, 8), .key(54, false, false, 0)])
        let rightOption = CGEventFlags(rawValue: CGEventFlags.maskAlternate.rawValue | 0x40)
        let bothOptions = CGEventFlags(rawValue: CGEventFlags.maskAlternate.rawValue | 0x60)
        require(InputCapture.modifiers(rightOption) == 0x44)
        require(InputCapture.modifiers(bothOptions) == 0x04)
        require(InputCapture.modifiers(CGEventFlags(rawValue: CGEventFlags.maskAlternate.rawValue | 0x20)) == 0x04)
        var layout = try EdgeLayout(localWidth: 1512, localHeight: 982, remoteWidth: 1728, remoteHeight: 1117, side: .left, offsetY: 0)
        _ = layout.motion(local: EdgePoint(x: 0,y: 400),dx: -1,dy: 0,exposed: true)
        require(layout.motion(local: EdgePoint(x: 0,y: 400),dx: 100,dy: 0,exposed: true,allowReturn: false) == .remote(EdgePoint(x: 1,y: 400.0/1116)))
        require(layout.motion(local: EdgePoint(x: 0,y: 400),dx: 1,dy: 0,exposed: true) == .returned(EdgePoint(x: 4,y: 400)))
        print("PASS input validation, owned releases, modifier cleanup, repeat ownership, idempotent teardown, drag confinement")
    }
}
