import Foundation
import CoreGraphics

// Replays native/shared/fixtures/display-helper.json; engine/src/helper is the
// reference. The display helper reads stdin commands inline in Main.swift, so
// those checks cast the same fields it does.
@main struct DisplayHelperFixtureTests {
    static func object(_ json: String) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
    }
    static func main() throws {
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "native/shared/fixtures/display-helper.json"))) as! [String: Any]
        func vectors(_ key: String, _ channel: String) -> [(String, String)] {
            (fixture[key] as! [[String: Any]]).filter { $0["channel"] as? String == channel }
                .map { ($0["name"] as? String ?? $0["why"] as! String, $0["json"] as! String) }
        }

        for (name, json) in vectors("messages", "source_stdin") {
            let value = try object(json)
            switch value["type"] as? String {
            case "keyframe", "stop": break
            case "pause": precondition(value["value"] is Bool, name)
            case "bitrate": precondition(value["value"] as? Int != nil, name)
            case "configuration":
                precondition(["id", "width", "height", "fps", "bitrate"].allSatisfy { value[$0] as? Int != nil }
                    && value["retina"] is Bool, name)
            default: fatalError(name)
            }
        }
        for (name, json) in vectors("messages", "viewer_control") {
            let value = try object(json)
            switch value["type"] as? String {
            case "input_state": _ = try ViewerInputCapabilities(message: value)
            case "viewport": precondition(value["id"] as? Int != nil, name)
            case "configuration":
                precondition(["id", "width", "height", "fps"].allSatisfy { value[$0] as? Int != nil }, name)
            case "stop": break
            default: fatalError(name)
            }
        }

        let viewport = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        for (name, json) in vectors("messages", "cursor") {
            do { try CursorOverlay().apply(Data(json.utf8), viewport: viewport, source: viewport.size) }
            catch { fatalError("\(name): \(error)") }
        }
        for (why, json) in vectors("rejected", "cursor") {
            var rejected = false
            do { try CursorOverlay().apply(Data(json.utf8), viewport: viewport, source: viewport.size) }
            catch { rejected = true }
            precondition(rejected, why)
        }

        // The recorded viewer input is what ViewerInputModel sends for this sequence.
        let granted = vectors("messages", "viewer_control").first { $0.0 == "viewer input granted" }!.1
        var model = ViewerInputModel()
        _ = model.configure(try ViewerInputCapabilities(message: object(granted)))
        var sent = model.intent()
        sent += model.text("é🙂")
        sent += model.move(CGPoint(x: 0.25, y: 0.65625))
        sent += model.button(0, down: true, clicks: 1, point: CGPoint(x: 0.5, y: 0.5), modifiers: 0)
        sent += model.button(0, down: false, clicks: 1, point: CGPoint(x: 0.5, y: 0.5), modifiers: 0)
        sent += model.scroll(dx: 2.5, dy: -7, point: CGPoint(x: 0.5, y: 0.5), modifiers: 0)
        sent += model.key(code: 0, down: true, repeat: false, modifiers: 8)
        sent += model.blur()
        let recorded = vectors("messages", "viewer_input")
        precondition(sent.count == recorded.count, "Viewer input count changed")
        for (message, (name, json)) in zip(sent, recorded) {
            let bytes = try JSONSerialization.data(withJSONObject: message)
            let written = try JSONSerialization.jsonObject(with: bytes) as! NSDictionary
            let expected = try object(json) as NSDictionary
            precondition(written == expected, name)
        }

        let section = fixture["inject_display"] as! [String: Any]
        let descriptor = vectors("messages", "inject_display_target")[0].1
        var binding = DisplayInputBinding(target: try DisplayInputTarget.decode(Data(descriptor.utf8)))
        for envelope in section["envelopes"] as! [[String: Any]] {
            let line = envelope["line"] as! String
            do { _ = try binding.accept(DisplayInputEnvelope.decode(Data(line.utf8))) }
            catch { fatalError("\(line): \(error)") }
        }
        precondition(binding.sequence == UInt64((section["envelopes"] as! [Any]).count))
        print("PASS display helper fixtures: source and viewer stdin, cursor packets, recorded viewer input and inject-display lines")
    }
}
