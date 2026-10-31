import Foundation
import CoreGraphics

private func displayInputError(_ detail: String) -> NSError {
    NSError(domain: "ExtendComputerDisplayInput", code: 1, userInfo: [NSLocalizedDescriptionKey: detail])
}

private func exactKeys(_ value: Any?, _ keys: Set<String>) throws -> [String: Any] {
    guard let object = value as? [String: Any], Set(object.keys) == keys else {
        throw displayInputError("Unexpected or missing display input fields")
    }
    return object
}

struct DisplayInputRect: Codable, Equatable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    var valid: Bool {
        x.isFinite && y.isFinite && width.isFinite && height.isFinite && width >= 1 && height >= 1
            && (x + width).isFinite && (y + height).isFinite
    }
    var cgRect: CGRect { CGRect(x: x, y: y, width: width, height: height) }
}

struct DisplayInputSize: Codable {
    let width: Double
    let height: Double
}

// Target comes exclusively from the local capture helper. It is immutable for
// the lifetime of this injector: a geometry change requires a fresh process.
struct DisplayInputTarget: Codable {
    let session: String
    let epoch: UInt64
    let revision: UInt64
    let display_id: UInt32
    let bounds: DisplayInputRect
    let canvas: DisplayInputSize
    let content: DisplayInputRect

    static func decode(_ data: Data) throws -> Self {
        guard data.count <= 4096 else { throw displayInputError("Target descriptor too large") }
        let object = try exactKeys(JSONSerialization.jsonObject(with: data), ["session", "epoch", "revision", "display_id", "bounds", "canvas", "content"])
        _ = try exactKeys(object["bounds"], ["x", "y", "width", "height"])
        _ = try exactKeys(object["content"], ["x", "y", "width", "height"])
        _ = try exactKeys(object["canvas"], ["width", "height"])
        let target = try JSONDecoder().decode(Self.self, from: data)
        guard !target.session.isEmpty, target.session.utf8.count <= 128,
              target.session.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) || "-_".unicodeScalars.contains($0) }),
              target.display_id != 0, target.bounds.valid, target.content.valid,
              target.canvas.width.isFinite, target.canvas.height.isFinite,
              (1...32768).contains(target.canvas.width), (1...32768).contains(target.canvas.height),
              target.content.x >= 0, target.content.y >= 0,
              target.content.x + target.content.width <= target.canvas.width,
              target.content.y + target.content.height <= target.canvas.height else {
            throw displayInputError("Invalid display target geometry or binding")
        }
        return target
    }

    func point(x: Double, y: Double) throws -> CGPoint {
        guard x.isFinite, y.isFinite, (0...1).contains(x), (0...1).contains(y) else {
            throw displayInputError("Invalid normalized display point")
        }
        let px = x * canvas.width, py = y * canvas.height
        guard px >= content.x, py >= content.y,
              px <= content.x + content.width, py <= content.y + content.height else {
            throw displayInputError("Display point is outside captured content")
        }
        // Preserve capture's logical fit; only constrain the final pixel strip.
        // Bars are rejected above, never clamped into an edge click.
        return CGPoint(x: bounds.x + min(bounds.width - 1, (px - content.x) / content.width * bounds.width),
                       y: bounds.y + min(bounds.height - 1, (py - content.y) / content.height * bounds.height))
    }
}

struct DisplayInputEvent: Codable {
    let kind: String
    let x: Double?
    let y: Double?
    let button: Int?
    let down: Bool?
    let clicks: Int?
    let dx: Int?
    let dy: Int?
    let code: Int?
    let `repeat`: Bool?
    let mask: Int?
    let value: String?

    var packet: InputPacket? {
        switch kind {
        case "button": return InputPacket(kind: kind, button: button, down: down, clicks: clicks)
        case "scroll": return InputPacket(kind: kind, dx: dx, dy: dy)
        case "mac_key": return InputPacket(kind: kind, down: down, code: code, repeat: self.repeat)
        case "modifiers": return InputPacket(kind: kind, mask: mask)
        case "text": return InputPacket(kind: kind, value: value)
        case "release": return InputPacket(kind: kind)
        default: return nil
        }
    }
}

struct DisplayInputEnvelope: Codable {
    let session: String
    let epoch: UInt64
    let revision: UInt64
    let sequence: UInt64
    let event: DisplayInputEvent

    static func decode(_ data: Data) throws -> Self {
        guard data.count <= 1024 else { throw displayInputError("Display input command too large") }
        let object = try exactKeys(JSONSerialization.jsonObject(with: data), ["session", "epoch", "revision", "sequence", "event"])
        guard let event = object["event"] as? [String: Any], let kind = event["kind"] as? String else {
            throw displayInputError("Missing display input event")
        }
        let keys: Set<String>
        switch kind {
        case "move": keys = ["kind", "x", "y"]
        case "button": keys = ["kind", "x", "y", "button", "down", "clicks"]
        case "scroll": keys = ["kind", "x", "y", "dx", "dy"]
        case "mac_key": keys = ["kind", "code", "down", "repeat"]
        case "modifiers": keys = ["kind", "mask"]
        case "text": keys = ["kind", "value"]
        case "release", "alive": keys = ["kind"]
        default: throw displayInputError("Unsupported display input event")
        }
        _ = try exactKeys(event, keys)
        let value = try JSONDecoder().decode(Self.self, from: data)
        try value.event.packet?.validate()
        return value
    }
}

struct DisplayInputBinding {
    let target: DisplayInputTarget
    private(set) var sequence: UInt64 = 0

    mutating func accept(_ command: DisplayInputEnvelope) throws -> CGPoint? {
        guard command.session == target.session, command.epoch == target.epoch, command.revision == target.revision,
              command.sequence > sequence else {
            throw displayInputError("Stale display input session, epoch or sequence")
        }
        let point: CGPoint?
        switch command.event.kind {
        case "move", "button", "scroll":
            guard let x = command.event.x, let y = command.event.y else { throw displayInputError("Missing display input position") }
            point = try target.point(x: x, y: y)
        default: point = nil
        }
        sequence = command.sequence
        return point
    }
}
