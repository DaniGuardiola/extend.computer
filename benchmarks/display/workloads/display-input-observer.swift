import AppKit
import CoreGraphics
import Darwin

// Acceptance evidence comes exclusively from native events delivered to these
// owned windows. This observer never intercepts or synthesizes global input.
func screenID(_ screen: NSScreen) -> UInt32 {
    (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
}
func rectangle(_ rect: CGRect) -> [String: Double] {
    ["x": rect.minX, "y": rect.minY, "width": rect.width, "height": rect.height]
}
func position(_ point: CGPoint) -> [String: Double] { ["x": point.x, "y": point.y] }

struct Options {
    let display: UInt32
    let output: String
    let fill: Bool
    let activate: Bool
    let duration: Double
    static func parse() -> Options {
        var display: UInt32?, output: String?, fill = false, activate = false, duration = 600.0
        let args = Array(CommandLine.arguments.dropFirst())
        if args.contains("--help") {
            print("Usage: display-input-observer --target-display ID --output PATH [--fill] [--activate] [--seconds 1..3600]")
            print("Writes PATH and PATH.ready.json. SIGTERM/SIGINT closes only owned windows; active focus restored on exit.")
            exit(0)
        }
        var index = 0
        while index < args.count {
            switch args[index] {
            case "--target-display": index += 1; if index < args.count { display = UInt32(args[index]) }
            case "--output": index += 1; if index < args.count { output = args[index] }
            case "--seconds": index += 1; if index < args.count { duration = Double(args[index]) ?? .nan }
            case "--fill": fill = true
            case "--activate": activate = true
            default: fputs("Unknown observer option\n", stderr); exit(2)
            }
            index += 1
        }
        guard let display, display != 0, let output, !output.isEmpty, duration.isFinite, (1...3600).contains(duration) else {
            fputs("Exact target display ID, output path and bounded duration required; use --help\n", stderr); exit(2)
        }
        return Options(display: display, output: output, fill: fill, activate: activate, duration: duration)
    }
}

final class ObserverWindow: NSWindow {
    var keyboardRecording: ((NSEvent, String) -> Void)?
    override func sendEvent(_ event: NSEvent) {
        switch event.type {
        case .keyDown: keyboardRecording?(event, "key_down")
        case .keyUp: keyboardRecording?(event, "key_up")
        case .flagsChanged: keyboardRecording?(event, "modifiers")
        default: break
        }
        super.sendEvent(event)
    }
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}
final class ObserverView: NSView, NSTextViewDelegate {
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    let role: String
    let expectedDisplay: UInt32
    var recording: ((ObserverView, NSEvent, String) -> Void)?
    var counters: [String: Int] = [:]
    var last = "No input received"
    var point: CGPoint?
    var dragTrail: [CGPoint] = []
    var scrollTotal = CGPoint.zero
    var tracking: NSTrackingArea?
    let keyboard = NSTextView(frame: .zero)
    var textChanged: (() -> Void)?
    static let textLimit = 256
    let labels = ["1 · Tap", "2 · Double tap", "3 · Right click", "4 · Drag", "5 · Scroll", "6 · Pointer"]
    let colors: [NSColor] = [.systemRed, .systemBlue, .systemPurple, .systemGreen, .systemOrange, .systemTeal]
    init(frame: CGRect, role: String, display: UInt32) {
        self.role = role; self.expectedDisplay = display
        super.init(frame: frame)
        keyboard.isRichText = false; keyboard.isEditable = true; keyboard.isSelectable = true
        keyboard.isContinuousSpellCheckingEnabled = false
        keyboard.isAutomaticSpellingCorrectionEnabled = false
        keyboard.isAutomaticQuoteSubstitutionEnabled = false
        keyboard.isAutomaticDashSubstitutionEnabled = false
        keyboard.isAutomaticTextReplacementEnabled = false
        keyboard.font = .monospacedSystemFont(ofSize: 16, weight: .regular)
        keyboard.textColor = .white; keyboard.backgroundColor = NSColor(calibratedWhite: 0.13, alpha: 1)
        keyboard.textContainerInset = NSSize(width: 12, height: 8)
        keyboard.delegate = self; keyboard.setAccessibilityLabel("Keyboard input test field")
        addSubview(keyboard); layoutKeyboard()
    }
    private func layoutKeyboard() {
        keyboard.frame = CGRect(x: 24, y: max(0, bounds.height - 72), width: max(1, bounds.width - 48), height: 48)
    }
    override func layout() { super.layout(); layoutKeyboard() }
    func textView(_ textView: NSTextView, shouldChangeTextIn affectedCharRange: NSRange, replacementString: String?) -> Bool {
        let value = textView.string as NSString
        guard affectedCharRange.location <= value.length, affectedCharRange.length <= value.length - affectedCharRange.location else { return false }
        return value.replacingCharacters(in: affectedCharRange, with: replacementString ?? "").utf16.count <= Self.textLimit
    }
    func textDidChange(_ notification: Notification) { textChanged?() }
    func receiveKeyboard(_ event: NSEvent, _ kind: String) {
        counters[kind, default: 0] += 1
        last = "\(kind) · key \(event.keyCode) · repeat \(event.type == .keyDown && event.isARepeat)"
        recording?(self, event, kind); needsDisplay = true
    }
    required init?(coder: NSCoder) { fatalError("Unsupported") }
    func zones() -> [CGRect] {
        let gap = 12.0, left = 24.0, top = 96.0
        let width = max(1, (bounds.width - left * 2 - gap * 2) / 3)
        let height = max(1, (bounds.height - top - 112 - gap) / 2)
        return (0..<6).map { CGRect(x: left + Double($0 % 3) * (width + gap),
                                  y: top + Double($0 / 3) * (height + gap), width: width, height: height) }
    }
    func zone(at point: CGPoint) -> Int? { zones().firstIndex { $0.contains(point) }.map { $0 + 1 } }
    override func updateTrackingAreas() {
        if let tracking { removeTrackingArea(tracking) }
        tracking = NSTrackingArea(rect: bounds, options: [.mouseMoved, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(tracking!); super.updateTrackingAreas()
    }
    func text(_ value: String, _ at: CGPoint, _ size: CGFloat, color: NSColor = .white) {
        (value as NSString).draw(at: at, withAttributes: [.font: NSFont.monospacedSystemFont(ofSize: size, weight: .medium), .foregroundColor: color])
    }
    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedWhite: 0.07, alpha: 1).setFill(); bounds.fill()
        text(role == "target" ? "Target display \(expectedDisplay)" : "Main display guard \(expectedDisplay)", CGPoint(x: 24, y: 18), 24)
        text(last, CGPoint(x: 24, y: 56), 14)
        for (index, zone) in zones().enumerated() {
            colors[index].withAlphaComponent(0.85).setFill(); NSBezierPath(roundedRect: zone, xRadius: 12, yRadius: 12).fill()
            text(labels[index], CGPoint(x: zone.minX + 16, y: zone.minY + 16), 19)
            text("\(Int(zone.midX)), \(Int(zone.midY))", CGPoint(x: zone.minX + 16, y: zone.minY + 44), 13)
            NSColor.white.setStroke()
            let cross = NSBezierPath(); cross.move(to: CGPoint(x: zone.midX - 12, y: zone.midY)); cross.line(to: CGPoint(x: zone.midX + 12, y: zone.midY))
            cross.move(to: CGPoint(x: zone.midX, y: zone.midY - 12)); cross.line(to: CGPoint(x: zone.midX, y: zone.midY + 12)); cross.stroke()
        }
        if dragTrail.count > 1 {
            NSColor.yellow.setStroke(); let path = NSBezierPath(); path.lineWidth = 4; path.move(to: dragTrail[0])
            for point in dragTrail.dropFirst() { path.line(to: point) }; path.stroke()
        }
        if let point { NSColor.white.setFill(); NSBezierPath(ovalIn: CGRect(x: point.x - 6, y: point.y - 6, width: 12, height: 12)).fill() }
        let counts = "Down \(counters["down", default: 0])  Up \(counters["up", default: 0])  Drag \(counters["drag", default: 0])  Scroll \(counters["scroll", default: 0])  Δ \(Int(scrollTotal.x)),\(Int(scrollTotal.y))"
        text(counts, CGPoint(x: 24, y: bounds.height - 104), 14)
        text("Keyboard input · up to \(Self.textLimit) characters", CGPoint(x: 24, y: bounds.height - 86), 12)
    }
    func receive(_ event: NSEvent, _ kind: String) {
        point = convert(event.locationInWindow, from: nil)
        counters[kind, default: 0] += 1
        let button = ["down", "up", "drag"].contains(kind) ? event.buttonNumber : 0
        let clicks = ["down", "up"].contains(kind) ? event.clickCount : 0
        last = "\(kind) · button \(button) · clicks \(clicks) · zone \(point.flatMap { zone(at: $0) }.map(String.init) ?? "outside")"
        if kind == "down" { dragTrail.removeAll() }
        if kind == "drag", let point {
            if dragTrail.count >= 256 { dragTrail.removeFirst() }; dragTrail.append(point)
        }
        if kind == "scroll" { scrollTotal.x += event.scrollingDeltaX; scrollTotal.y += event.scrollingDeltaY }
        recording?(self, event, kind); needsDisplay = true
    }
    override func mouseMoved(with event: NSEvent) { receive(event, "move") }
    override func mouseDown(with event: NSEvent) { receive(event, "down") }
    override func mouseUp(with event: NSEvent) { receive(event, "up") }
    override func mouseDragged(with event: NSEvent) { receive(event, "drag") }
    override func rightMouseDown(with event: NSEvent) { receive(event, "down") }
    override func rightMouseUp(with event: NSEvent) { receive(event, "up") }
    override func rightMouseDragged(with event: NSEvent) { receive(event, "drag") }
    override func otherMouseDown(with event: NSEvent) { receive(event, "down") }
    override func otherMouseUp(with event: NSEvent) { receive(event, "up") }
    override func otherMouseDragged(with event: NSEvent) { receive(event, "drag") }
    override func scrollWheel(with event: NSEvent) { receive(event, "scroll") }
}

final class Observer: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let options: Options
    var windows: [ObserverWindow] = []
    var views: [ObserverView] = []
    var previous: NSRunningApplication?
    var signals: [DispatchSourceSignal] = []
    var timer: Timer?
    var events: [[String: Any]] = []
    var sequence: UInt64 = 0
    var dirty = false, finished = false
    var focusSequence: UInt64 = 0
    var focusEvents: [[String: Any]] = []
    var lastFocusSignature = ""
    init(_ options: Options) { self.options = options }
    func global(_ point: CGPoint, screen: NSScreen) -> CGPoint {
        let cg = CGDisplayBounds(screenID(screen))
        return CGPoint(x: cg.minX + point.x - screen.frame.minX, y: cg.minY + screen.frame.maxY - point.y)
    }
    func geometry(_ view: ObserverView) -> [String: Any] {
        guard let window = view.window, let screen = window.screen else { return ["role": view.role, "missing_window": true] }
        let cocoa = window.convertToScreen(view.convert(view.bounds, to: nil))
        let cgOrigin = global(CGPoint(x: cocoa.minX, y: cocoa.maxY), screen: screen)
        let content = CGRect(origin: cgOrigin, size: cocoa.size)
        let displayBounds = CGDisplayBounds(screenID(screen))
        let zones: [[String: Any]] = view.zones().enumerated().map { index, zone in
            let local = CGPoint(x: zone.midX, y: zone.midY)
            let screenPoint = window.convertPoint(toScreen: view.convert(local, to: nil))
            let actual = global(screenPoint, screen: screen)
            return ["number": index + 1, "label": view.labels[index], "local_center": position(local), "global_center": position(actual),
                    "source_normalized": ["x": (actual.x - displayBounds.minX) / displayBounds.width, "y": (actual.y - displayBounds.minY) / displayBounds.height],
                    "local_bounds": rectangle(zone)]
        }
        let editorCenter = CGPoint(x: view.keyboard.frame.midX, y: view.keyboard.frame.midY)
        let editorScreen = window.convertPoint(toScreen: view.convert(editorCenter, to: nil))
        let editorGlobal = global(editorScreen, screen: screen)
        return ["role": view.role, "expected_display_id": view.expectedDisplay, "actual_display_id": screenID(screen),
                "window_number": window.windowNumber, "cocoa_window_frame": rectangle(window.frame),
                "source_content_bounds": rectangle(content), "source_display_bounds": rectangle(displayBounds),
                "cocoa_screen_frame": rectangle(screen.frame), "cocoa_visible_frame": rectangle(screen.visibleFrame),
                "scale": screen.backingScaleFactor, "content_logical_size": ["width": view.bounds.width, "height": view.bounds.height],
                "zones": zones, "counters": view.counters,
                "window_key": window.isKeyWindow, "keyboard_editor_first_responder": window.firstResponder === view.keyboard,
                "keyboard_focus_active": NSApp.isActive && window.isKeyWindow && window.firstResponder === view.keyboard,
                "keyboard_text": view.keyboard.string, "keyboard_text_limit": ObserverView.textLimit,
                "keyboard_field": ["local_bounds": rectangle(view.keyboard.frame), "local_center": position(editorCenter),
                    "global_center": position(editorGlobal), "source_normalized": ["x": (editorGlobal.x - displayBounds.minX) / displayBounds.width,
                                                                                 "y": (editorGlobal.y - displayBounds.minY) / displayBounds.height]]]
    }
    func record(_ view: ObserverView, _ event: NSEvent, _ kind: String) {
        guard !finished, let window = view.window, let screen = window.screen else { return }
        sequence += 1
        let local = view.convert(event.locationInWindow, from: nil)
        let cocoa = window.convertPoint(toScreen: event.locationInWindow)
        let isKeyboard = ["key_down", "key_up", "modifiers"].contains(kind)
        let sourceTag = event.cgEvent?.getIntegerValueField(.eventSourceUserData) ?? 0
        var entry: [String: Any] = ["sequence": sequence, "role": view.role, "kind": kind,
            "timestamp_ns": UInt64(max(0, event.timestamp) * 1e9), "observed_uptime_ns": DispatchTime.now().uptimeNanoseconds,
            "button": ["down", "up", "drag"].contains(kind) ? event.buttonNumber : 0, "click_count": ["down", "up"].contains(kind) ? event.clickCount : 0, "local_point": position(local),
            "global_point": position(global(cocoa, screen: screen)), "cocoa_global_point": position(cocoa),
            "zone": isKeyboard ? NSNull() : (view.zone(at: local) as Any? ?? NSNull()), "window_number": window.windowNumber,
            "position_valid": !isKeyboard, "source_user_data": sourceTag,
            "input_origin": sourceTag == 0x504f5254 ? "extend-tagged" : "unmarked",
            "expected_display_id": view.expectedDisplay, "actual_display_id": screenID(screen),
            "scroll": ["dx": kind == "scroll" ? event.scrollingDeltaX : 0, "dy": kind == "scroll" ? event.scrollingDeltaY : 0, "precise": kind == "scroll" && event.hasPreciseScrollingDeltas],
            "phase": kind == "scroll" ? event.phase.rawValue : 0, "momentum_phase": kind == "scroll" ? event.momentumPhase.rawValue : 0]
        if isKeyboard {
            let flags = event.modifierFlags
            let mask = (flags.contains(.shift) ? 1 : 0) | (flags.contains(.control) ? 2 : 0)
                | (flags.contains(.option) ? 4 : 0) | (flags.contains(.command) ? 8 : 0)
            entry["keyboard"] = ["code": event.keyCode, "repeat": event.type == .keyDown && event.isARepeat,
                "modifier_flags": flags.rawValue, "modifier_mask": mask,
                "characters": kind != "modifiers" && sourceTag == 0x504f5254 ? String((event.characters ?? "").prefix(ObserverView.textLimit)) : "",
                "editor_first_responder": window.firstResponder === view.keyboard]
        }
        if events.count >= 1024 { events.removeFirst() }; events.append(entry); dirty = true
    }
    func snapshot(_ status: String) -> [String: Any] {
        ["status": status, "target_display_id": options.display, "main_display_id": CGMainDisplayID(),
         "snapshot_uptime_ns": DispatchTime.now().uptimeNanoseconds, "application_active": NSApp.isActive,
         "focus_sequence": focusSequence, "focus_events": focusEvents,
         "focus_events_dropped": focusSequence > 64 ? focusSequence - 64 : 0,
         "event_source": "native-window-events", "coordinate_system": "global_point: CoreGraphics logical top-left; local_point: view logical top-left",
         "event_limit": 1024, "events_dropped": sequence > 1024 ? sequence - 1024 : 0,
         "sequence": sequence, "windows": views.map(geometry), "events": events]
    }
    func save(_ status: String, path: String? = nil) {
        do { try JSONSerialization.data(withJSONObject: snapshot(status), options: [.sortedKeys]).write(to: URL(fileURLWithPath: path ?? options.output), options: .atomic); dirty = false }
        catch { fputs("Observer output failed: \(error)\n", stderr); finish("output-failed") }
    }
    func refreshFocus(_ reason: String) {
        guard !finished, !views.isEmpty else { return }
        let signature = "\(NSApp.isActive):" + views.map { view in
            "\(view.window?.isKeyWindow == true):\(view.window?.firstResponder === view.keyboard)"
        }.joined(separator: ";")
        guard signature != lastFocusSignature else { return }
        lastFocusSignature = signature; focusSequence += 1
        let state: [[String: Any]] = views.map { view in
            ["role": view.role, "window_number": view.window?.windowNumber ?? -1,
             "window_key": view.window?.isKeyWindow == true,
             "editor_first_responder": view.window?.firstResponder === view.keyboard]
        }
        if focusEvents.count >= 64 { focusEvents.removeFirst() }
        focusEvents.append(["sequence": focusSequence, "observed_uptime_ns": DispatchTime.now().uptimeNanoseconds,
                            "reason": reason, "application_active": NSApp.isActive, "windows": state])
        // Focus-only changes must become evidence without waiting for another input.
        save("observing")
    }
    func windowDidBecomeKey(_ notification: Notification) { refreshFocus("window-became-key") }
    func windowDidResignKey(_ notification: Notification) { refreshFocus("window-resigned-key") }
    func applicationDidBecomeActive(_ notification: Notification) { refreshFocus("application-became-active") }
    func applicationDidResignActive(_ notification: Notification) { refreshFocus("application-resigned-active") }
    func applicationDidFinishLaunching(_ notification: Notification) {
        previous = NSWorkspace.shared.frontmostApplication
        guard let target = NSScreen.screens.first(where: { screenID($0) == options.display }), CGDisplayIsActive(options.display) != 0 else {
            fputs("Exact target display is not active\n", stderr); NSApp.terminate(nil); return
        }
        let primary = NSScreen.screens.first { screenID($0) == CGMainDisplayID() }
        var selected: [(String, NSScreen)] = [("target", target)]
        if let primary, screenID(primary) != options.display { selected.append(("main-guard", primary)) }
        for (role, screen) in selected {
            let usable = screen.visibleFrame
            let size = options.fill && role == "target" ? usable.size : CGSize(width: min(1000, usable.width - 48), height: min(720, usable.height - 72))
            guard size.width >= 300, size.height >= 300 else { fputs("Target screen too small\n", stderr); finish("invalid-geometry"); return }
            let frame = CGRect(x: usable.midX - size.width / 2, y: usable.midY - size.height / 2, width: size.width, height: size.height)
            let window = ObserverWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false, screen: screen)
            window.setFrame(frame, display: true); window.isReleasedWhenClosed = false; window.delegate = self
            window.title = role == "target" ? "Display input target" : "Main display input guard"
            window.acceptsMouseMovedEvents = true; window.colorSpace = .sRGB
            let view = ObserverView(frame: CGRect(origin: .zero, size: size), role: role, display: screenID(screen))
            view.appearance = NSAppearance(named: .aqua)
            view.recording = { [weak self] view, event, kind in self?.record(view, event, kind) }
            view.textChanged = { [weak self] in self?.dirty = true }
            window.keyboardRecording = { [weak view] event, kind in view?.receiveKeyboard(event, kind) }
            window.contentView = view; windows.append(window); views.append(view); window.orderFrontRegardless()
        }
        if options.activate {
            windows.first?.makeKeyAndOrderFront(nil)
            if let view = views.first { windows.first?.makeFirstResponder(view.keyboard) }
            NSApp.activate(ignoringOtherApps: true)
        }
        for number in [SIGTERM, SIGINT] {
            signal(number, SIG_IGN); let signalSource = DispatchSource.makeSignalSource(signal: number, queue: .main)
            signalSource.setEventHandler { [weak self] in self?.finish("stopped") }; signalSource.resume(); signals.append(signalSource)
        }
        refreshFocus("initial-focus")
        save("ready", path: options.output + ".ready.json"); save("observing")
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            guard let self else { return }
            if self.views.contains(where: { $0.window?.screen.map(screenID) != $0.expectedDisplay }) || CGDisplayIsActive(self.options.display) == 0 {
                self.finish("display-changed"); return
            }
            // First-responder changes need not produce a key-window notification.
            self.refreshFocus("first-responder-observed")
            if self.dirty { self.save("observing") }
        }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
        DispatchQueue.main.asyncAfter(deadline: .now() + options.duration) { [weak self] in self?.finish("duration-ended") }
    }
    func windowWillClose(_ notification: Notification) { if !finished { finish("window-closed") } }
    func finish(_ status: String) {
        guard !finished else { return }; finished = true; timer?.invalidate(); save(status)
        for window in windows { window.orderOut(nil); window.close() }
        if NSWorkspace.shared.frontmostApplication?.processIdentifier == ProcessInfo.processInfo.processIdentifier {
            previous?.activate(options: [])
        }
        NSApp.terminate(nil)
    }
    func applicationWillTerminate(_ notification: Notification) { if !finished { finished = true; save("terminated") } }
}

let options = Options.parse()
let app = NSApplication.shared
let observer = Observer(options)
app.setActivationPolicy(.accessory); app.delegate = observer
app.run()
