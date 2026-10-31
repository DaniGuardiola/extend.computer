import Foundation
import CoreGraphics

// Modifier bits are portable: shift, control, alt, meta. Key codes are explicitly
// Physical Mac virtual codes and separately negotiated committed Unicode text. No clipboard is used.
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
    var value: String? = nil

    private static let fields: [String: Set<String>] = ["button": ["button", "down", "clicks"], "scroll": ["dx", "dy"],
        "mac_key": ["code", "down", "repeat"], "modifiers": ["mask"], "text": ["value"], "release": []]

    // Like the engine's InputEvent, a line carries exactly its kind's fields.
    static func decode(_ line: Data) throws -> Self {
        guard let object = try JSONSerialization.jsonObject(with: line) as? [String: Any],
              let kind = object["kind"] as? String, let keys = fields[kind],
              Set(object.keys) == keys.union(["kind"]),
              // JSONDecoder reads 1.0 or 1e2 into an Int; the engine rejects them.
              !object.values.contains(where: { ($0 as? NSNumber).map { CFNumberIsFloatType($0) } ?? false })
        else { throw NSError(domain: "ExtendComputerInput", code: 1) }
        return try JSONDecoder().decode(Self.self, from: line)
    }

    func validate() throws {
        var valid = false
        switch kind {
        case "button": valid = (0...2).contains(button ?? -1) && down != nil && (1...3).contains(clicks ?? 0)
        case "scroll": valid = (-4096...4096).contains(dx ?? 99999) && (-4096...4096).contains(dy ?? 99999)
        case "mac_key": valid = (0..<128).contains(code ?? -1) && ![54,55,56,57,58,59,60,61,62,63].contains(code ?? -1) && down != nil && self.repeat != nil && (self.repeat != true || down == true)
        // The high nibble marks held modifiers as their right-hand keys.
        case "modifiers": valid = mask.map { (0..<256).contains($0) && ($0 >> 4) & ~($0 & 0xf) == 0 } ?? false
        case "text": valid = value.map { !$0.isEmpty && $0.utf8.count <= 256 && $0.unicodeScalars.count <= 64 && !$0.unicodeScalars.contains(where: { $0.value < 32 || (127...159).contains($0.value) }) } ?? false
        case "release": valid = true
        default: break
        }
        if !valid { throw NSError(domain: "ExtendComputerInput", code: 1) }
    }
}

enum InputAction: Equatable {
    case text(String)
    case key(Int, Bool, Bool, Int)
    case button(Int, Bool, Int, Int)
    case scroll(Int, Int, Int)
}

struct InputState {
    private(set) var keys = Set<Int>()
    private(set) var buttons = Set<Int>()
    private(set) var modifiers = 0
    private var modifierKeys = [Int?](repeating: nil, count: 4)
    // Shift, control, option, command: left keys unless the mask marks the
    // right-hand key.
    static let modifierCodes = [56, 59, 58, 55]
    static let rightModifierCodes = [60, 62, 61, 54]

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
        case "text":
            guard keys.isEmpty && modifiers == 0 else { throw NSError(domain: "ExtendComputerInput", code: 2) }
            return [.text(packet.value!)]
        case "release": return release()
        default: return []
        }
    }
    mutating func updateModifiers(_ target: Int) -> [InputAction] {
        var actions = [InputAction]()
        for bit in 0..<4 {
            let flag = 1 << bit
            let wanted = target & flag == 0 ? nil
                : target & (flag << 4) != 0 ? Self.rightModifierCodes[bit] : Self.modifierCodes[bit]
            guard modifierKeys[bit] != wanted else { continue }
            // Switching sides releases the other key first.
            if let held = modifierKeys[bit] {
                modifiers &= ~flag
                actions.append(.key(held, false, false, modifiers))
            }
            modifierKeys[bit] = wanted
            if let wanted {
                modifiers |= flag
                actions.append(.key(wanted, true, false, modifiers))
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

// Factory is testable without posting input or acquiring permissions.
enum CommittedTextEvent {
    static func make(source: CGEventSource?, value: String, down: Bool) -> CGEvent? {
        let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)
        let units = Array(value.utf16)
        units.withUnsafeBufferPointer { event?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: $0.baseAddress) }
        return event
    }
}
