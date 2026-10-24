import Foundation
import CoreGraphics

func require(_ condition: @autoclosure () -> Bool, _ detail: String) {
    if !condition() { fatalError(detail) }
}
func packet(_ message: [String: Any]) -> [String: Any] {
    ((message["command"] as? [String: Any])?["event"] as? [String: Any]) ?? [:]
}
func capabilities(_ revision: UInt64 = 1, enabled: Bool = false, focused: Bool = false,
                  text: Bool = true, policy: String? = "command") -> ViewerInputCapabilities {
    var value: [String: Any] = ["type": "input_state", "revision": revision,
        "available": true, "enabled": enabled, "focused": focused, "text_supported": text,
        "content": ["x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0]]
    if let policy { value["shortcut_policy"] = policy }
    return try! ViewerInputCapabilities(message: value)
}

@main enum ViewerInputTests {
    static func main() {
        let viewport = CGRect(x: 10, y: 20, width: 1000, height: 1000)
        let canvas = CGSize(width: 1000, height: 500), content = ViewerInputContent()
        require(ViewerInputGeometry.point(CGPoint(x: 510, y: 520), viewport: viewport,
            canvas: canvas, content: content) == CGPoint(x: 0.5, y: 0.5), "Letterbox center")
        require(ViewerInputGeometry.point(CGPoint(x: 10, y: 270), viewport: viewport,
            canvas: canvas, content: content) == CGPoint(x: 0, y: 1), "Bottom-left conversion")
        for point in [CGPoint(x: 510, y: 100), CGPoint(x: CGFloat.nan, y: 400), CGPoint(x: 1011, y: 400)] {
            require(ViewerInputGeometry.point(point, viewport: viewport, canvas: canvas, content: content) == nil,
                "Invalid/outside viewport point")
        }
        let inset = ViewerInputContent(x: 0.1, y: 0.2, width: 0.8, height: 0.6)
        require(ViewerInputGeometry.point(CGPoint(x: 10, y: 520), viewport: viewport,
            canvas: canvas, content: inset) == nil, "Source content letterbox")
        require(ViewerInputGeometry.point(.zero, viewport: .zero, canvas: canvas, content: content) == nil,
            "Zero geometry")
        require(ViewerInputGeometry.screenRect(CGRect(x: -1400, y: 120, width: 1200, height: 700),
            cocoaScreen: CGRect(x: -1600, y: -100, width: 1600, height: 1000),
            display: CGRect(x: -1600, y: 50, width: 1600, height: 1000))
            == CGRect(x: -1400, y: 130, width: 1200, height: 700), "Secondary screen CG origin and flipped Y")
        require(ViewerInputGeometry.scrollDelta(20.5, precise: true) == 20.5
            && ViewerInputGeometry.scrollDelta(0.4, precise: true) == 0.4,
            "Precise trackpad deltas retain point magnitude and fractional motion")
        require(ViewerInputGeometry.scrollDelta(3, precise: false) == 30
            && ViewerInputGeometry.scrollDelta(-2, precise: false) == -20,
            "Coarse wheel lines use the source pixel event's documented default scale")
        require(ViewerInputGeometry.scrollDelta(Double.greatestFiniteMagnitude, precise: false) == 4096
            && ViewerInputGeometry.scrollDelta(.nan, precise: true).isNaN,
            "Scroll conversion bounds finite values without accepting invalid input")

        var model = ViewerInputModel()
        require(model.intent().isEmpty && model.move(CGPoint(x: 0.5, y: 0.5)).isEmpty,
            "No input before negotiated availability")
        _ = model.configure(capabilities())
        let capBytes = try! JSONSerialization.data(withJSONObject: ["type": "input_state", "revision": 1,
            "focused": false, "available": true, "enabled": false, "text_supported": true,
            "shortcut_policy": "command", "content": ["x": 0, "y": 0, "width": 1, "height": 1]])
        let capJSON = try! JSONSerialization.jsonObject(with: capBytes) as! [String: Any]
        require((try! ViewerInputCapabilities(message: capJSON)) == capabilities(), "Real framed JSON capabilities")
        require(!model.focused && model.move(CGPoint(x: 0.5, y: 0.5)).isEmpty,
            "Availability must not request a lease")
        let focus = model.intent()
        require(focus.count == 1 && focus[0]["active"] as? Bool == true && focus[0]["revision"] as? UInt64 == 1,
            "Explicit intent carries revision")
        let down = model.button(0, down: true, clicks: 2, point: CGPoint(x: 0.4, y: 0.5), modifiers: 1)
        require(down.count == 2 && packet(down[0])["kind"] as? String == "modifiers"
            && packet(down[1])["down"] as? Bool == true && model.state.buttons == [0], "First click before grant")
        _ = model.configure(capabilities(enabled: true, focused: true))
        let up = model.button(0, down: false, clicks: 2, point: nil, modifiers: 0)
        require(packet(up.last!)["down"] as? Bool == false && model.state.buttons.isEmpty,
            "Drag ending outside view releases at last owned point")
        require(model.button(0, down: true, clicks: 1, point: CGPoint(x: -1, y: 0), modifiers: 0).isEmpty,
            "Outside content cannot start a button")
        require(model.scroll(dx: 0.4, dy: 0.4, point: CGPoint(x: 0.5, y: 0.5), modifiers: 0).isEmpty,
            "Retain fractional scrolling")
        let scroll = model.scroll(dx: 0.7, dy: 0.7, point: CGPoint(x: 0.5, y: 0.5), modifiers: 0)
        require(packet(scroll.last!)["dx"] as? Int == 1 && packet(scroll.last!)["dy"] as? Int == 1,
            "Fractional scrolling accumulates")
        require(model.scroll(dx: .infinity, dy: 1, point: .zero, modifiers: 0).isEmpty, "Nonfinite scroll")
        let bounded = model.scroll(dx: 1e300, dy: -1e300, point: .zero, modifiers: 0)
        require(packet(bounded.last!)["dx"] as? Int == 4096, "Scroll remains protocol bounded")

        _ = model.key(code: 123, down: true, repeat: false, modifiers: 0)
        require(model.text("blocked").isEmpty, "Text cannot overlap held physical keys")
        require(!model.key(code: 123, down: true, repeat: true, modifiers: 0).isEmpty, "Held key repeat")
        require(!model.key(code: 123, down: false, repeat: false, modifiers: 0).isEmpty, "Balanced physical key")
        require(model.key(code: 123, down: false, repeat: false, modifiers: 0).isEmpty, "Suppress duplicate key up")
        _ = model.modifiers(1)
        let unicode = String(repeating: "é e\u{301} 中 👩‍💻 ", count: 30)
        let text = model.text(unicode)
        let commits = text.filter { packet($0)["kind"] as? String == "text" }
        require(commits.map { packet($0)["value"] as! String }.joined() == unicode, "Unicode chunks preserve exact scalars")
        for commit in commits {
            let value = packet(commit)["value"] as! String
            require(value.utf8.count <= 256 && value.unicodeScalars.count <= 64, "Protocol text bounds")
            require((try! JSONSerialization.data(withJSONObject: commit)).count <= 1024, "Framed intent byte limit")
        }
        require(packet(text.first!)["mask"] as? Int == 0 && packet(text.last!)["mask"] as? Int == 1,
            "Committed Unicode neutralizes then restores modifiers")
        _ = model.modifiers(4)
        let optionText = model.text("é")
        require(packet(optionText.first!)["mask"] as? Int == 0 && packet(optionText.last!)["mask"] as? Int == 4,
            "Option composition neutralizes then restores source modifiers")
        var sourceState = InputState(); _ = try! sourceState.apply(InputPacket(kind: "modifiers", mask: 4))
        for message in optionText {
            let bytes = try! JSONSerialization.data(withJSONObject: packet(message))
            let event = try! JSONDecoder().decode(InputPacket.self, from: bytes)
            require(!(try! sourceState.apply(event)).isEmpty, "Source accepts committed text with neutral modifiers")
        }
        require(model.text("bad\ntext").isEmpty, "Control text rejected")
        require(model.text(String(repeating: "x", count: 2049)).isEmpty
            && model.text(String(repeating: "👩", count: 1025)).isEmpty, "A local commit stays within the bounded sender queue")
        var control = capabilities(enabled: true, focused: true, policy: "control")
        _ = model.configure(control)
        let shortcut = model.key(code: 8, down: true, repeat: false, modifiers: 8)
        require(packet(shortcut.first!)["mask"] as? Int == 2 && model.state.modifiers == 2,
            "Command+C follows negotiated source Control policy without clipboard access")
        let blur = model.blur()
        require(blur.count == 1 && blur[0]["active"] as? Bool == false && model.state.keys.isEmpty
            && model.state.buttons.isEmpty && model.state.modifiers == 0, "Focus loss releases held state")
        require(model.blur().isEmpty && model.text("late").isEmpty, "Repeated loss and retained input harmless")
        _ = model.intent(); _ = model.button(0, down: true, clicks: 1, point: .zero, modifiers: 0)
        control.revision = 2; control.enabled = false; control.remoteFocused = false
        require(model.configure(control) && !model.focused && model.state.buttons.isEmpty, "Loss revision clears pending first click")
        require(!model.configure(capabilities()) && model.capabilities.revision == 2, "Stale state cannot revive input")

        var composition = ViewerComposition()
        require(composition.mark("draft") && composition.marked == "draft", "Composition retained locally")
        require(composition.commit("最终 👩‍💻") == "最终 👩‍💻" && composition.finish() == nil,
            "Explicit commit followed by finish delivers once")
        _ = composition.mark("résumé")
        require(composition.finish() == "résumé" && composition.finish() == nil, "Draft finish delivers once")
        _ = composition.mark("private draft"); composition.cancel()
        require(composition.finish() == nil, "Blur discards composition")
        require(!composition.mark(String(repeating: "x", count: 4097)) && composition.marked == nil,
            "Bound composition memory")

        // Block only the private writer, then prove enqueue stays responsive
        // and overflow replaces stale intent with one focus-loss packet.
        let started = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0), finished = DispatchSemaphore(value: 0)
        let lock = NSLock(); var rows: [[String: Any]] = []; var first = true; var overflow = false
        let output = ViewerInputOutput { bytes in
            lock.lock(); let block = first; first = false; lock.unlock()
            if block { started.signal(); require(release.wait(timeout: .now() + 2) == .success, "Release writer fixture") }
            let row = try! JSONSerialization.jsonObject(with: bytes) as! [String: Any]
            lock.lock(); rows.append(row); lock.unlock()
            if row["type"] as? String == "focus", row["active"] as? Bool == false { finished.signal() }
        }
        output.overflow = { overflow = true }
        output.enqueue([["type": "focus", "active": true, "revision": UInt64(1)]])
        require(started.wait(timeout: .now() + 2) == .success, "Writer started")
        let before = Date()
        for _ in 0..<100 {
            output.enqueue([["type": "input", "revision": UInt64(1),
                "command": ["kind": "move", "point": ["x": 0.4, "y": 0.5]]]])
        }
        require(!overflow, "Consecutive moves coalesce under backpressure")
        for _ in 0..<65 {
            output.enqueue([["type": "focus", "active": true, "revision": UInt64(1)]])
            if overflow { break }
        }
        require(overflow && Date().timeIntervalSince(before) < 1, "Bounded writer cannot freeze caller")
        release.signal()
        require(finished.wait(timeout: .now() + 2) == .success, "Overflow emits focus loss")
        lock.lock(); let retained = rows; lock.unlock()
        require(retained.count <= 3 && retained.last?["active"] as? Bool == false, "Overflow discards queued input")
        print("viewer input geometry, lease intent, lifecycle, key, Unicode, composition and bounded writer checks passed")
    }
}
