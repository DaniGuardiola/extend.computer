import Foundation

// Modifier bits are portable: shift, control, alt, meta. Key codes are explicitly
// Mac virtual codes in this first adapter; no Unicode text or clipboard is sent.
struct InputPacket: Codable {
    var kind: String
    var button: Int? = nil
    var down: Bool? = nil
    var clicks: Int? = nil
    var dx: Int? = nil
    var dy: Int? = nil
    var code: Int? = nil
    var `repeat`: Bool? = nil
    var mask: Int? = nil

    func validate() throws {
        var valid = false
        switch kind {
        case "button": valid = (0...2).contains(button ?? -1) && down != nil && (1...3).contains(clicks ?? 0)
        case "scroll": valid = (-4096...4096).contains(dx ?? 99999) && (-4096...4096).contains(dy ?? 99999)
        case "mac_key": valid = (0..<128).contains(code ?? -1) && ![54,55,56,57,58,59,60,61,62,63].contains(code ?? -1) && down != nil && self.repeat != nil && (self.repeat != true || down == true)
        case "modifiers": valid = (0..<16).contains(mask ?? -1)
        case "release": valid = true
        default: break
        }
        if !valid { throw NSError(domain: "ExtendComputerInput", code: 1) }
    }
}

enum InputAction: Equatable {
    case key(Int, Bool, Bool, Int)
    case button(Int, Bool, Int, Int)
    case scroll(Int, Int, Int)
}

struct InputState {
    private(set) var keys = Set<Int>()
    private(set) var buttons = Set<Int>()
    private(set) var modifiers = 0
    // Use left-side modifiers in this adapter. Right-side distinctions are not
    // represented by the portable modifier mask.
    static let modifierCodes = [56, 59, 58, 55]

    mutating func apply(_ packet: InputPacket) throws -> [InputAction] {
        try packet.validate()
        switch packet.kind {
        case "mac_key":
            let code = packet.code!, down = packet.down!, repeating = packet.repeat!
            if down {
                if repeating { return keys.contains(code) ? [.key(code, true, true, modifiers)] : [] }
                guard keys.insert(code).inserted else { return [] }
            } else { guard keys.remove(code) != nil else { return [] } }
            return [.key(code, down, false, modifiers)]
        case "button":
            let button = packet.button!, down = packet.down!
            if down { guard buttons.insert(button).inserted else { return [] } }
            else { guard buttons.remove(button) != nil else { return [] } }
            return [.button(button, down, packet.clicks!, modifiers)]
        case "scroll": return [.scroll(packet.dx!, packet.dy!, modifiers)]
        case "modifiers": return updateModifiers(packet.mask!)
        case "release": return release()
        default: return []
        }
    }
    mutating func updateModifiers(_ target: Int) -> [InputAction] {
        var actions = [InputAction]()
        for (bit, code) in Self.modifierCodes.enumerated() {
            let flag = 1 << bit
            if modifiers & flag != target & flag {
                let down = target & flag != 0
                modifiers = down ? modifiers | flag : modifiers & ~flag
                actions.append(.key(code, down, false, modifiers))
            }
        }
        return actions
    }
    mutating func release() -> [InputAction] {
        var actions = keys.sorted().map { InputAction.key($0, false, false, modifiers) }
        actions += buttons.sorted().map { InputAction.button($0, false, 1, modifiers) }
        keys.removeAll(); buttons.removeAll()
        actions += updateModifiers(0)
        return actions
    }
}
