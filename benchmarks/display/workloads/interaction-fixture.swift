import AppKit
import ApplicationServices
import Darwin

// Real OS events target only this fixture's focused, owned external-display window.
// This verifies source interaction outcomes, not hardware input-to-photon latency.
final class InteractionWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}
final class InteractionView: NSView {
    override var isFlipped: Bool { true }
    var box = NSRect(x: 180, y: 280, width: 120, height: 90)
    var dragStart: NSPoint?
    var dragOrigin = NSPoint.zero
    var dragEvents = 0
    var showMarkers = false
    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedWhite: 0.93, alpha: 1).setFill(); bounds.fill()
        if showMarkers {
            // Keep markers clear of macOS's protected recording indicator.
            for origin in [NSPoint(x: 40, y: 40), NSPoint(x: bounds.width - 68, y: 40),
                           NSPoint(x: 40, y: bounds.height - 68), NSPoint(x: bounds.width - 68, y: bounds.height - 68)] {
                for row in 0..<4 { for column in 0..<4 {
                    ((row + column) % 2 == 0 ? NSColor.white : NSColor.black).setFill()
                    NSRect(x: origin.x + CGFloat(column * 7), y: origin.y + CGFloat(row * 7), width: 7, height: 7).fill()
                } }
            }
        }
        NSColor.systemBlue.setFill(); box.fill()
        ("Drag target" as NSString).draw(at: NSPoint(x: box.minX + 10, y: box.minY + 30),
            withAttributes: [.font: NSFont.systemFont(ofSize: 16), .foregroundColor: NSColor.white])
    }
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if box.contains(point) { dragStart = point; dragOrigin = box.origin }
    }
    override func mouseDragged(with event: NSEvent) {
        guard let start = dragStart else { return }
        let point = convert(event.locationInWindow, from: nil)
        box.origin = NSPoint(x: dragOrigin.x + point.x - start.x, y: dragOrigin.y + point.y - start.y)
        dragEvents += 1; needsDisplay = true
    }
    override func mouseUp(with event: NSEvent) { dragStart = nil }
}
final class DocumentView: NSView {
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        for i in 0..<60 {
            (i % 2 == 0 ? NSColor.white : NSColor.lightGray).setFill()
            NSRect(x: 0, y: i * 40, width: Int(bounds.width), height: 40).fill()
            ("Scroll row \(i)" as NSString).draw(at: NSPoint(x: 12, y: i * 40 + 10),
                withAttributes: [.font: NSFont.monospacedSystemFont(ofSize: 16, weight: .regular)])
        }
    }
}
final class InteractionFixture: NSObject, NSApplicationDelegate {
    var window: InteractionWindow!
    var view: InteractionView!
    var field: NSTextField!
    var scroll: NSScrollView!
    var status: NSTextField!
    var display: CGDirectDisplayID = 0
    var geometry = CGRect.zero
    var output = ""
    var clicks = 0
    var finished = false
    var observing = false
    var snapshotReady = false
    var scale: CGFloat = 1
    var exitCode: Int32 = 1
    var buttonHeld = false
    var cursorOriginal: CGPoint?
    var previousApplication: NSRunningApplication?
    var records: [[String: Any]] = []
    var signals: [DispatchSourceSignal] = []
    func stamp() -> UInt64 { DispatchTime.now().uptimeNanoseconds }
    func applicationWillTerminate(_ notification: Notification) {
        if !finished { finish("cancelled", error: "Fixture terminated") }
    }
    @objc func clicked(_ sender: NSButton) { clicks += 1; sender.title = "Click received" }
    func focused() -> Bool {
        NSWorkspace.shared.frontmostApplication?.processIdentifier == getpid() && window.isKeyWindow
    }
    func schedule(_ delay: Double, _ operation: @escaping () -> Void) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, !self.finished else { return }
            guard self.focused(), CGDisplayIsActive(self.display) != 0,
                  CGDisplayBounds(self.display) == self.geometry else {
                self.finish("failed", error: "Fixture focus or display geometry changed"); return
            }
            operation()
        }
    }
    func mouse(_ type: CGEventType, _ local: CGPoint) {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: type,
            mouseCursorPosition: CGPoint(x: geometry.minX + local.x, y: geometry.minY + local.y), mouseButton: .left) else {
            finish("failed", error: "Mouse event allocation failed"); return
        }
        event.setIntegerValueField(.mouseEventClickState, value: 1)
        event.post(tap: .cghidEventTap)
        if type == .leftMouseDown { buttonHeld = true }
        if type == .leftMouseUp { buttonHeld = false }
    }
    func text(_ value: String) {
        let units = Array(value.utf16)
        for down in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down) else {
                finish("failed", error: "Keyboard event allocation failed"); return
            }
            units.withUnsafeBufferPointer { event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: $0.baseAddress!) }
            event.flags = []; event.post(tap: .cghidEventTap)
        }
    }
    func check(_ name: String, _ accepted: Bool, observed: Any) {
        records.append(["action": name, "observed_at_ns": stamp(), "accepted": accepted, "observed": observed])
        if !accepted { finish("failed", error: "Interaction failed: " + name) }
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments
        guard args.count == 3 || args.count == 4 && args[3] == "--observe" else { exit(2) }
        observing = args.count == 4
        output = args[1]
        guard AXIsProcessTrusted() else { finish("unavailable", error: "OS event posting requires existing Accessibility permission"); return }
        let candidates = NSScreen.screens.filter {
            let id = ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! NSNumber).uint32Value
            return CGDisplayIsBuiltin(id) == 0 && CGDisplayIsInMirrorSet(id) == 0
        }
        let selected = args[2] == "auto" && candidates.count == 1 ? candidates.first : candidates.first {
            String(($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! NSNumber).uint32Value) == args[2]
        }
        guard let screen = selected, screen.frame.width >= 1000, screen.frame.height >= 700 else {
            finish("unavailable", error: "One unmirrored external display of at least 1000 by 700 required"); return
        }
        display = (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! NSNumber).uint32Value
        scale = screen.backingScaleFactor
        geometry = CGDisplayBounds(display); cursorOriginal = CGEvent(source: nil)?.location
        previousApplication = NSWorkspace.shared.frontmostApplication
        // Native text editing shortcuts use the responder chain through the
        // application's standard Edit menu, as they do in ordinary macOS apps.
        let menu = NSMenu()
        let editItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        let edit = NSMenu(title: "Edit")
        edit.addItem(NSMenuItem(title: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a"))
        editItem.submenu = edit; menu.addItem(editItem); NSApp.mainMenu = menu
        view = InteractionView(frame: NSRect(origin: .zero, size: screen.frame.size))
        view.showMarkers = observing
        // Fix fixture appearance so native labels remain legible on its light
        // background, regardless of either host's system theme.
        view.appearance = NSAppearance(named: .aqua)
        let button = NSButton(title: "Click target", target: self, action: #selector(clicked(_:)))
        button.frame = NSRect(x: 80, y: 70, width: 200, height: 44); view.addSubview(button)
        field = NSTextField(frame: NSRect(x: 80, y: 140, width: 360, height: 36)); field.stringValue = "initial"; view.addSubview(field)
        scroll = NSScrollView(frame: NSRect(x: 600, y: 80, width: 320, height: 450))
        scroll.hasVerticalScroller = true; scroll.documentView = DocumentView(frame: NSRect(x: 0, y: 0, width: 300, height: 2400))
        view.addSubview(scroll)
        status = NSTextField(labelWithString: "Running owned input checks")
        status.frame = NSRect(x: 80, y: 560, width: 820, height: 50); view.addSubview(status)
        window = InteractionWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false, screen: screen)
        window.setFrame(screen.frame, display: true); window.contentView = view
        window.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 2)
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
        for number in [SIGTERM, SIGINT] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
            source.setEventHandler { [weak self] in guard let self else { return }
                self.finish(self.snapshotReady ? "passed" : "cancelled", error: self.snapshotReady ? nil : "Fixture interrupted") }
            source.resume(); signals.append(source)
        }
        schedule(0.5) { self.mouse(.leftMouseDown, CGPoint(x: 180, y: 92)) }
        schedule(0.55) { self.mouse(.leftMouseUp, CGPoint(x: 180, y: 92)) }
        schedule(0.8) { self.check("click", self.clicks == 1, observed: self.clicks) }
        schedule(1.0) { self.mouse(.leftMouseDown, CGPoint(x: 230, y: 158)) }
        schedule(1.05) { self.mouse(.leftMouseUp, CGPoint(x: 230, y: 158)) }
        schedule(1.2) {
            self.check("field-focus", self.field.currentEditor() != nil, observed: self.field.currentEditor() != nil)
            if !self.finished { self.text(" test123") }
        }
        schedule(1.5) {
            let observed = self.field.currentEditor()?.string ?? self.field.stringValue
            self.check("typing", observed.contains("test123"), observed: observed)
        }
        schedule(1.7) {
            for down in [true, false] {
                guard let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down) else { self.finish("failed", error: "Shortcut allocation failed"); return }
                event.flags = .maskCommand; event.post(tap: .cghidEventTap)
            }
        }
        schedule(1.9) { self.text("replacement456") }
        schedule(2.2) {
            let observed = self.field.currentEditor()?.string ?? self.field.stringValue
            self.check("select-all-replacement", observed == "replacement456", observed: observed)
        }
        schedule(2.5) {
            self.mouse(.leftMouseDown, CGPoint(x: 240, y: 325))
            print("drag-started"); fflush(stdout)
        }
        for step in 1...10 {
            schedule(2.5 + Double(step) * 0.03) { self.mouse(.leftMouseDragged, CGPoint(x: 240 + step * 15, y: 325 + step * 8)) }
        }
        schedule(2.85) { self.mouse(.leftMouseUp, CGPoint(x: 390, y: 405)) }
        schedule(3.1) { self.check("drag", abs(self.view.box.minX - 330) < 2 && abs(self.view.box.minY - 360) < 2 && self.view.dragEvents > 0,
            observed: ["x": self.view.box.minX, "y": self.view.box.minY, "events": self.view.dragEvents]) }
        schedule(3.4) {
            guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: -180, wheel2: 0, wheel3: 0) else {
                self.finish("failed", error: "Scroll event allocation failed"); return
            }
            event.location = CGPoint(x: self.geometry.minX + 760, y: self.geometry.minY + 300)
            event.post(tap: .cghidEventTap)
        }
        schedule(4.0) { self.check("scroll", self.scroll.contentView.bounds.minY > 20, observed: self.scroll.contentView.bounds.minY) }
        schedule(4.5) {
            if self.observing { self.snapshot() } else { self.finish("passed") }
        }
        if observing { schedule(14) { self.finish("failed", error: "Receiver did not finish observation before deadline") } }
        print("ready"); fflush(stdout)
    }
    func snapshot() {
        // Capture only our fully covering synthetic window, never another app.
        window.makeFirstResponder(nil)
        status.stringValue = "Source checks passed; receiver observation pending"
        view.displayIfNeeded()
        do {
            let capture = Process()
            capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), "-t", "png", output + ".png"]
            try capture.run(); capture.waitUntilExit()
            guard capture.terminationStatus == 0 else { throw NSError(domain: "Owned interaction capture failed", code: 1) }
            let data: [String: Any] = ["status": "ready", "actions": records, "display_id": display,
                "logical_size": [view.bounds.width, view.bounds.height], "scale": scale,
                "registration_marker_inset": 40,
                "capture_scope": "Owned interaction window covering source extended display",
                "visual_regions": ["button": [80, 70, 200, 44], "replacement": [80, 140, 360, 36],
                    "drag": [320, 350, 150, 120], "scroll": [600, 80, 300, 450]]]
            try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]).write(to: URL(fileURLWithPath: output + ".ready.json"), options: .atomic)
            snapshotReady = true
        } catch { finish("failed", error: String(describing: error)) }
    }
    func finish(_ result: String, error: String? = nil) {
        guard !finished else { return }; finished = true
        exitCode = result == "passed" ? 0 : result == "unavailable" ? 3 : 1
        let releaseRequested = buttonHeld
        if buttonHeld, let point = CGEvent(source: nil)?.location {
            CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
            buttonHeld = false
        }
        let restored = cursorOriginal.map { original -> Bool in
            guard CGWarpMouseCursorPosition(original) == .success,
                  let actual = CGEvent(source: nil)?.location else { return false }
            return abs(actual.x - original.x) < 2 && abs(actual.y - original.y) < 2
        } ?? false
        let focusRestoreRequested = previousApplication?.activate(options: []) ?? false
        let data: [String: Any] = ["schema_version": 1, "status": result, "error": error ?? NSNull(),
            "actions": records, "display_id": display, "cursor_restored": restored,
            "mouse_release_requested": releaseRequested,
            "focus_restore_requested": focusRestoreRequested,
            "scope": "Real source OS events and owned control outcomes; not physical input latency or receiver acceptance"]
        do { try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]).write(to: URL(fileURLWithPath: output), options: .atomic) }
        catch { fputs("Cannot save interaction evidence\n", stderr); exit(1) }
        // NSApplication.terminate exits with zero itself; use the verified
        // outcome after saving evidence and cleanup so failures cannot look green.
        exit(exitCode)
    }
}
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let fixture = InteractionFixture(); app.delegate = fixture; app.run(); exit(fixture.exitCode)
