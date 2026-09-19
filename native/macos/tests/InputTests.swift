import Foundation

@main struct InputTests {
    static func require(_ value: Bool) { precondition(value) }
    static func main() throws {
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
        for invalid in [InputPacket(kind: "button", button: 3, down: true, clicks: 1), InputPacket(kind: "scroll", dx: 0, dy: 100000), InputPacket(kind: "mac_key", down: true, code: 65535, repeat: false), InputPacket(kind: "modifiers", mask: 255)] {
            do { _ = try state.apply(invalid); fatalError("invalid input accepted") } catch { }
        }
        var layout = try EdgeLayout(localWidth: 1512, localHeight: 982, remoteWidth: 1728, remoteHeight: 1117, side: .left, offsetY: 0)
        _ = layout.motion(local: EdgePoint(x: 0,y: 400),dx: -1,dy: 0,exposed: true)
        require(layout.motion(local: EdgePoint(x: 0,y: 400),dx: 100,dy: 0,exposed: true,allowReturn: false) == .remote(EdgePoint(x: 1,y: 400.0/1116)))
        require(layout.motion(local: EdgePoint(x: 0,y: 400),dx: 1,dy: 0,exposed: true) == .returned(EdgePoint(x: 4,y: 400)))
        print("PASS input validation, owned releases, modifier cleanup, repeat ownership, idempotent teardown, drag confinement")
    }
}
