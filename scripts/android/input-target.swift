import AppKit
import CoreGraphics

// Logs only Extend-tagged events delivered to this disposable test window.
// It never monitors another application or records physical keyboard input.
private final class Target: NSView {
    let output: FileHandle
    var text = ""
    init(frame: NSRect, path: String) throws {
        FileManager.default.createFile(atPath: path, contents: nil)
        output = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
        super.init(frame: frame)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.activeAlways, .mouseMoved, .inVisibleRect], owner: self))
    }
    required init?(coder: NSCoder) { fatalError("Unsupported") }
    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }
    func record(_ event: NSEvent, kind: String) {
        guard event.cgEvent?.getIntegerValueField(.eventSourceUserData) == 0x504f5254 else {
            // Metadata only: no physical key text is captured.
            print("Ignored event: \(kind), marker=\(event.cgEvent?.getIntegerValueField(.eventSourceUserData) ?? -1)")
            return
        }
        let point = convert(event.locationInWindow, from: nil)
        let isKey = kind.hasPrefix("key_") || kind == "modifiers"
        let value: [String: Any] = ["kind": kind, "key": isKey ? event.keyCode : 0, "text": kind.hasPrefix("key_") ? (event.characters ?? "") : "",
                                   "x": point.x, "y": point.y, "dx": kind == "scroll" ? event.scrollingDeltaX : 0, "dy": kind == "scroll" ? event.scrollingDeltaY : 0,
                                   "modifiers": event.modifierFlags.rawValue, "time": event.timestamp]
        if let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) { output.write(data); output.write(Data([10])) }
        if kind == "key_down" { text += event.characters ?? ""; needsDisplay = true }
    }
    override func mouseDown(with event: NSEvent) { window?.makeFirstResponder(self); record(event,kind:"left_down") }
    override func mouseUp(with event: NSEvent) { record(event,kind:"left_up") }
    override func rightMouseDown(with event: NSEvent) { record(event,kind:"right_down") }
    override func rightMouseUp(with event: NSEvent) { record(event,kind:"right_up") }
    override func mouseMoved(with event: NSEvent) { record(event,kind:"move") }
    override func mouseDragged(with event: NSEvent) { record(event,kind:"drag") }
    override func scrollWheel(with event: NSEvent) { record(event,kind:"scroll") }
    override func keyDown(with event: NSEvent) { record(event,kind:"key_down") }
    override func keyUp(with event: NSEvent) { record(event,kind:"key_up") }
    override func flagsChanged(with event: NSEvent) { record(event,kind:"modifiers") }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill(); bounds.fill()
        let style: [NSAttributedString.Key: Any] = [.font:NSFont.systemFont(ofSize:22), .foregroundColor:NSColor.labelColor]
        ("Android input check\n\nTablet click, movement and keys appear here.\n\n" + text).draw(in: bounds.insetBy(dx:30,dy:30),withAttributes:style)
    }
}
@main private enum Main {
    static func main() throws {
        guard CommandLine.arguments.count == 2 else { throw NSError(domain:"Expected output path",code:1) }
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        let screen = NSScreen.screens.first { ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == CGMainDisplayID() }!
        let area = screen.visibleFrame
        let frame = NSRect(x:area.midX-400,y:area.midY-250,width:800,height:500)
        let window = NSWindow(contentRect:frame,styleMask:[.titled,.closable,.resizable],backing:.buffered,defer:false)
        window.title = "Extend Android Input Check"
        let target = try Target(frame:NSRect(origin:.zero,size:frame.size),path:CommandLine.arguments[1])
        window.contentView = target; window.acceptsMouseMovedEvents = true
        window.makeKeyAndOrderFront(nil); window.makeFirstResponder(target)
        app.activate(ignoringOtherApps:true)
        app.run()
    }
}
