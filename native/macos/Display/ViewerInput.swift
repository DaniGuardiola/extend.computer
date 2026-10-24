import Foundation
import CoreGraphics

// The native viewer owns only local intent. The authenticated Rust receiver
// owns the lease, ordering and remote target. No OS input is posted here.
struct ViewerInputContent: Equatable {
    var x = 0.0, y = 0.0, width = 1.0, height = 1.0
    var valid: Bool {
        [x, y, width, height].allSatisfy { $0.isFinite }
            && x >= 0 && y >= 0 && width > 0 && height > 0
            && x + width <= 1 + 1e-12 && y + height <= 1 + 1e-12
    }
    func contains(_ point: CGPoint) -> Bool {
        valid && point.x.isFinite && point.y.isFinite
            && point.x >= x && point.x <= x + width
            && point.y >= y && point.y <= y + height
    }
}

struct ViewerInputCapabilities: Equatable {
    var revision: UInt64 = 0
    var remoteFocused = false
    var available = false
    var enabled = false
    var textSupported = false
    var shortcutPolicy: String?
    var content = ViewerInputContent()

    init() {}
    init(message: [String: Any]) throws {
        guard message["type"] as? String == "input_state",
              let revision = message["revision"] as? UInt64, revision > 0,
              let focused = message["focused"] as? Bool,
              let available = message["available"] as? Bool,
              let enabled = message["enabled"] as? Bool,
              let text = message["text_supported"] as? Bool,
              !enabled || available else {
            throw NSError(domain: "ExtendViewerInput", code: 1)
        }
        self.revision = revision
        remoteFocused = focused
        self.available = available; self.enabled = enabled; textSupported = available && text
        if let policy = message["shortcut_policy"] as? String {
            guard policy == "command" || policy == "control" else {
                throw NSError(domain: "ExtendViewerInput", code: 2)
            }
            shortcutPolicy = policy
        }
        if let content = message["content"] as? [String: Any] {
            guard let x = content["x"] as? Double, let y = content["y"] as? Double,
                  let width = content["width"] as? Double, let height = content["height"] as? Double else {
                throw NSError(domain: "ExtendViewerInput", code: 3)
            }
            self.content = ViewerInputContent(x: x, y: y, width: width, height: height)
        }
        guard self.content.valid else { throw NSError(domain: "ExtendViewerInput", code: 4) }
    }
}

enum ViewerInputGeometry {
    static func scrollDelta(_ value: Double, precise: Bool) -> Double {
        guard value.isFinite else { return value }
        // NSEvent.h: precise deltas are points; coarse deltas need a line
        // height. CGEvent.h: the source's pixel events default to 10 per line.
        // Clamp before multiplying to preserve finite protocol bounds.
        let scale = precise ? 1.0 : 10.0
        return min(4096 / scale, max(-4096 / scale, value)) * scale
    }
    // Cocoa screen coordinates are bottom-up. CoreGraphics display bounds are
    // top-down and may have a different origin/scale on a secondary display.
    static func screenRect(_ rect: CGRect, cocoaScreen: CGRect, display: CGRect) -> CGRect? {
        guard [rect.minX, rect.minY, rect.width, rect.height, cocoaScreen.minX,
               cocoaScreen.minY, cocoaScreen.width, cocoaScreen.height, display.minX,
               display.minY, display.width, display.height].allSatisfy({ $0.isFinite }),
              rect.width > 0, rect.height > 0, cocoaScreen.width > 0, cocoaScreen.height > 0,
              display.width > 0, display.height > 0 else { return nil }
        let x = display.width / cocoaScreen.width, y = display.height / cocoaScreen.height
        return CGRect(x: display.minX + (rect.minX - cocoaScreen.minX) * x,
            y: display.minY + (cocoaScreen.maxY - rect.maxY) * y,
            width: rect.width * x, height: rect.height * y)
    }
    // NSView coordinates start at the bottom left. Wire coordinates start at
    // the top left of the encoded canvas, including its source-side letterbox.
    static func point(_ location: CGPoint, viewport: CGRect, canvas: CGSize,
                      content: ViewerInputContent) -> CGPoint? {
        guard [location.x, location.y, viewport.minX, viewport.minY,
               viewport.width, viewport.height, canvas.width, canvas.height].allSatisfy({ $0.isFinite }),
              viewport.width > 0, viewport.height > 0, canvas.width > 0, canvas.height > 0 else { return nil }
        let scale = min(viewport.width / canvas.width, viewport.height / canvas.height)
        let width = canvas.width * scale, height = canvas.height * scale
        let left = viewport.minX + (viewport.width - width) / 2
        let bottom = viewport.minY + (viewport.height - height) / 2
        let point = CGPoint(x: (location.x - left) / width, y: 1 - (location.y - bottom) / height)
        guard (0...1).contains(point.x), (0...1).contains(point.y), content.contains(point) else { return nil }
        return point
    }
}

struct ViewerInputModel {
    private(set) var capabilities = ViewerInputCapabilities()
    private(set) var focused = false
    private(set) var state = InputState()
    private(set) var lastPoint: CGPoint?
    private var scrollX = 0.0, scrollY = 0.0
    var acceptsInput: Bool { focused && capabilities.available }

    // Return whether local composition/held state must also be discarded.
    mutating func configure(_ next: ViewerInputCapabilities) -> Bool {
        guard next.revision >= capabilities.revision else { return false }
        let invalidate = !next.available || (capabilities.enabled && !next.enabled)
            || (capabilities.enabled && next.content != capabilities.content)
            || next.revision != capabilities.revision
            || (capabilities.remoteFocused && !next.remoteFocused)
        capabilities = next
        if invalidate { reset(); focused = false }
        return invalidate
    }
    mutating func intent() -> [[String: Any]] {
        guard capabilities.available else { return [] }
        let request = !focused || !capabilities.enabled
        focused = true
        return request ? [["type": "focus", "active": true, "revision": capabilities.revision]] : []
    }
    mutating func blur() -> [[String: Any]] {
        let active = focused; focused = false; reset()
        return active ? [["type": "focus", "active": false, "revision": capabilities.revision]] : []
    }
    private mutating func reset() {
        _ = state.release(); lastPoint = nil; scrollX = 0; scrollY = 0
    }
    private func command(_ packet: InputPacket, point: CGPoint? = nil) -> [String: Any]? {
        guard let data = try? JSONEncoder().encode(packet),
              let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        var value: [String: Any] = ["kind": "input", "event": event]
        if let point { value["point"] = ["x": Double(point.x), "y": Double(point.y)] }
        return ["type": "input", "command": value, "revision": capabilities.revision]
    }
    private mutating func event(_ packet: InputPacket, point: CGPoint? = nil) -> [[String: Any]] {
        guard acceptsInput, let effects = try? state.apply(packet), !effects.isEmpty,
              let message = command(packet, point: point) else { return [] }
        return [message]
    }
    mutating func modifiers(_ mask: Int) -> [[String: Any]] {
        event(InputPacket(kind: "modifiers", mask: mask))
    }
    mutating func move(_ point: CGPoint) -> [[String: Any]] {
        guard acceptsInput, capabilities.content.contains(point) else { return [] }
        lastPoint = point
        return [["type": "input", "command": ["kind": "move", "point": ["x": Double(point.x), "y": Double(point.y)]], "revision": capabilities.revision]]
    }
    mutating func button(_ button: Int, down: Bool, clicks: Int, point: CGPoint?, modifiers: Int) -> [[String: Any]] {
        guard acceptsInput else { return [] }
        let point = point.flatMap { capabilities.content.contains($0) ? $0 : nil }
        // A drag may end beyond the viewport. Release at the last valid owned
        // position; never create a remote down outside the displayed content.
        guard let target = point ?? (!down ? lastPoint : nil) else { return down ? [] : blur() }
        lastPoint = target
        return self.modifiers(modifiers) + event(InputPacket(kind: "button", button: button,
            down: down, clicks: min(3, max(1, clicks))), point: target)
    }
    mutating func scroll(dx: Double, dy: Double, point: CGPoint, modifiers: Int) -> [[String: Any]] {
        guard acceptsInput, capabilities.content.contains(point), dx.isFinite, dy.isFinite else { return [] }
        lastPoint = point
        scrollX += min(4096, max(-4096, dx)); scrollY += min(4096, max(-4096, dy))
        let x = Int(min(4096, max(-4096, scrollX))), y = Int(min(4096, max(-4096, scrollY)))
        scrollX -= Double(x); scrollY -= Double(y)
        guard x != 0 || y != 0 else { return [] }
        return self.modifiers(modifiers) + event(InputPacket(kind: "scroll", dx: x, dy: y), point: point)
    }
    mutating func key(code: Int, down: Bool, repeat repeating: Bool, modifiers: Int) -> [[String: Any]] {
        var mask = modifiers
        // Translate conventional local Command editing chords to the source's
        // negotiated modifier. Clipboard operations execute on the source.
        if mask & 8 != 0 && [0, 6, 7, 8, 9, 16].contains(code), capabilities.shortcutPolicy == "control" {
            mask = mask & ~8 | 2
        }
        guard acceptsInput else { return [] }
        if !down && !state.keys.contains(code) { return [] }
        return self.modifiers(mask) + event(InputPacket(kind: "mac_key", down: down,
            code: code, repeat: repeating))
    }
    static func textChunks(_ value: String) -> [String]? {
        // A single local commit must fit the bounded sender queue. Inspect only
        // bounded prefixes before splitting so an input method cannot stall the
        // AppKit thread with an arbitrarily large committed string.
        guard !value.isEmpty, value.utf8.prefix(4097).count <= 4096,
              value.unicodeScalars.prefix(2049).count <= 2048,
              !value.unicodeScalars.contains(where: {
            $0.value < 32 || (127...159).contains($0.value)
        }) else { return nil }
        var chunks: [String] = [], current = "", count = 0
        for scalar in value.unicodeScalars {
            let text = String(scalar)
            if count == 64 || current.utf8.count + text.utf8.count > 256 {
                chunks.append(current); current = ""; count = 0
            }
            current += text; count += 1
        }
        if !current.isEmpty { chunks.append(current) }
        return chunks
    }
    mutating func text(_ value: String) -> [[String: Any]] {
        guard acceptsInput, capabilities.textSupported, state.keys.isEmpty,
              let chunks = Self.textChunks(value) else { return [] }
        let mask = state.modifiers
        var messages = modifiers(0)
        for chunk in chunks { messages += event(InputPacket(kind: "text", value: chunk)) }
        messages += modifiers(mask)
        return messages
    }
}

struct ViewerComposition {
    private(set) var marked: String?
    mutating func mark(_ value: String) -> Bool {
        guard value.utf8.count <= 4096 else { cancel(); return false }
        marked = value.isEmpty ? nil : value; return true
    }
    mutating func commit(_ value: String) -> String? {
        marked = nil; return value.isEmpty ? nil : value
    }
    mutating func finish() -> String? {
        let value = marked; marked = nil; return value
    }
    mutating func cancel() { marked = nil }
}

// Stdout may be backpressured by a stopped parent. Keep AppKit responsive and
// bound queued intent; overflow discards pending input and sends a focus loss.
final class ViewerInputOutput {
    private let lock = NSLock()
    private let worker = DispatchQueue(label: "computer.extend.display.viewer-input", qos: .userInteractive)
    private var pending: [(Data, Bool, UInt64)] = []
    private var scheduled = false
    private let output: (Data) -> Void
    var overflow: (() -> Void)?
    init(output: @escaping (Data) -> Void) { self.output = output }
    func enqueue(_ messages: [[String: Any]]) {
        guard !messages.isEmpty else { return }
        var overflowed = false
        lock.lock()
        for message in messages {
            guard let revision = message["revision"] as? UInt64,
                  let bytes = try? JSONSerialization.data(withJSONObject: message), bytes.count <= 1024 else { continue }
            let command = message["command"] as? [String: Any]
            let move = message["type"] as? String == "input" && command?["kind"] as? String == "move"
            if message["type"] as? String == "focus", message["active"] as? Bool == false {
                pending.removeAll(keepingCapacity: true)
            }
            if move, let last = pending.last, last.1 && last.2 == revision { pending.removeLast() }
            if pending.count >= 64 {
                pending.removeAll(keepingCapacity: true)
                let loss: [String: Any] = ["type": "focus", "active": false, "revision": revision]
                if let bytes = try? JSONSerialization.data(withJSONObject: loss) { pending.append((bytes, false, revision)) }
                overflowed = true; break
            }
            pending.append((bytes, move, revision))
        }
        let start = !scheduled && !pending.isEmpty
        if start { scheduled = true }
        lock.unlock()
        if overflowed { overflow?() }
        if start {
            worker.async { [self] in
                while true {
                    lock.lock()
                    guard !pending.isEmpty else { scheduled = false; lock.unlock(); return }
                    let next = pending.removeFirst().0
                    lock.unlock()
                    output(next)
                }
            }
        }
    }
}
