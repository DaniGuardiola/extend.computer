import AppKit
import MetalKit
import CoreImage
import CoreVideo

final class VideoView: MTKView, MTKViewDelegate, NSTextInputClient {
    private let context: CIContext
    private let commands: MTLCommandQueue
    private let pipelineColorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    let cursor = CursorOverlay()
    var logicalSource = CGSize.zero
    var separateCursor = false
    var input: (([[String: Any]]) -> Void)?
    private var inputModel = ViewerInputModel()
    private var tracking: NSTrackingArea?
    private var marked = NSAttributedString(string: "")
    private var markedSelection = NSRange(location: 0, length: 0)
    private var composition = ViewerComposition()
    private var interpreting: NSEvent?
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    private var ownsFocus: Bool {
        NSApp.isActive && window?.isKeyWindow == true && window?.firstResponder === self
    }
    private func send(_ messages: [[String: Any]]) { input?(messages) }
    private func point(_ event: NSEvent) -> CGPoint? {
        ViewerInputGeometry.point(convert(event.locationInWindow, from: nil),
            viewport: bounds, canvas: logicalSource, content: inputModel.capabilities.content)
    }
    private func mask(_ flags: NSEvent.ModifierFlags) -> Int {
        (flags.contains(.shift) ? 1 : 0) | (flags.contains(.control) ? 2 : 0)
            | (flags.contains(.option) ? 4 : 0) | (flags.contains(.command) ? 8 : 0)
    }
    private func intent() -> Bool {
        guard ownsFocus, inputModel.capabilities.available else { return false }
        send(inputModel.intent()); return inputModel.acceptsInput
    }
    func configureInput(_ capabilities: ViewerInputCapabilities) {
        if inputModel.configure(capabilities) { clearComposition() }
        window?.invalidateCursorRects(for: self)
    }
    func stopInput() { send(inputModel.blur()); clearComposition() }
    func inputOverflow() { _ = inputModel.blur(); clearComposition() }
    private func clearComposition() {
        composition.cancel()
        marked = NSAttributedString(string: ""); markedSelection = NSRange(location: 0, length: 0)
        interpreting = nil; inputContext?.discardMarkedText()
    }
    override func resignFirstResponder() -> Bool { stopInput(); return super.resignFirstResponder() }
    override func updateTrackingAreas() {
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect], owner: self, userInfo: nil)
        tracking = area; addTrackingArea(area); super.updateTrackingAreas()
    }
    private func button(_ event: NSEvent, number: Int, down: Bool) {
        let location = point(event)
        if down {
            guard location != nil else { return }
            window?.makeFirstResponder(self)
            guard intent() else { return }
        } else if !ownsFocus { stopInput(); return }
        send(inputModel.button(number, down: down, clicks: event.clickCount,
            point: location, modifiers: mask(event.modifierFlags)))
    }
    override func mouseDown(with event: NSEvent) { button(event, number: 0, down: true) }
    override func mouseUp(with event: NSEvent) { button(event, number: 0, down: false) }
    override func rightMouseDown(with event: NSEvent) { button(event, number: 1, down: true) }
    override func rightMouseUp(with event: NSEvent) { button(event, number: 1, down: false) }
    override func otherMouseDown(with event: NSEvent) { if event.buttonNumber == 2 { button(event, number: 2, down: true) } }
    override func otherMouseUp(with event: NSEvent) { if event.buttonNumber == 2 { button(event, number: 2, down: false) } }
    override func mouseMoved(with event: NSEvent) {
        guard ownsFocus, let point = point(event) else { return }
        send(inputModel.move(point))
    }
    override func mouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func rightMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func otherMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func scrollWheel(with event: NSEvent) {
        guard let point = point(event) else { return }
        window?.makeFirstResponder(self)
        guard intent() else { return }
        // The source injects pixel scroll events. Preserve precise trackpad
        // motion and convert coarse wheel lines using CoreGraphics' default.
        send(inputModel.scroll(dx: ViewerInputGeometry.scrollDelta(Double(event.scrollingDeltaX),
                precise: event.hasPreciseScrollingDeltas),
            dy: ViewerInputGeometry.scrollDelta(Double(event.scrollingDeltaY),
                precise: event.hasPreciseScrollingDeltas), point: point,
            modifiers: mask(event.modifierFlags)))
    }
    override func flagsChanged(with event: NSEvent) {
        guard ownsFocus, inputModel.acceptsInput else { return }
        send(inputModel.modifiers(mask(event.modifierFlags)))
    }
    private func physicalKey(_ event: NSEvent, down: Bool) {
        send(inputModel.key(code: Int(event.keyCode), down: down,
            repeat: down && event.isARepeat, modifiers: mask(event.modifierFlags)))
    }
    override func keyDown(with event: NSEvent) {
        guard intent() else { super.keyDown(with: event); return }
        if inputModel.capabilities.textSupported && event.modifierFlags.intersection([.command, .control]).isEmpty {
            interpreting = event; interpretKeyEvents([event]); interpreting = nil
        } else { if hasMarkedText() { clearComposition() }; physicalKey(event, down: true) }
    }
    override func keyUp(with event: NSEvent) {
        guard ownsFocus, inputModel.acceptsInput else { return }
        physicalKey(event, down: false)
    }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard ownsFocus, inputModel.capabilities.available, event.type == .keyDown,
              !event.modifierFlags.intersection([.command, .control]).isEmpty else { return super.performKeyEquivalent(with: event) }
        keyDown(with: event); return true
    }
    func insertText(_ string: Any, replacementRange: NSRange) {
        guard ownsFocus, inputModel.acceptsInput else { return }
        let value = (string as? NSAttributedString)?.string ?? string as? String ?? ""
        marked = NSAttributedString(string: ""); markedSelection = NSRange(location: 0, length: 0)
        if let committed = composition.commit(value) { send(inputModel.text(committed)) }
    }
    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard ownsFocus, inputModel.acceptsInput, inputModel.capabilities.textSupported else { return }
        let value = (string as? NSAttributedString)?.string ?? string as? String ?? ""
        // Composition is local and bounded; only insertText commits remotely.
        guard composition.mark(value) else { clearComposition(); return }
        marked = NSAttributedString(string: value)
        markedSelection = NSRange(location: min(selectedRange.location, marked.length),
            length: min(selectedRange.length, max(0, marked.length - min(selectedRange.location, marked.length))))
    }
    func unmarkText() {
        if let committed = composition.finish(), ownsFocus, inputModel.acceptsInput {
            send(inputModel.text(committed))
        }
        marked = NSAttributedString(string: ""); markedSelection = NSRange(location: 0, length: 0)
    }
    func selectedRange() -> NSRange { markedSelection }
    func markedRange() -> NSRange { marked.length == 0 ? NSRange(location: NSNotFound, length: 0) : NSRange(location: 0, length: marked.length) }
    func hasMarkedText() -> Bool { marked.length != 0 }
    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }
    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        guard range.location != NSNotFound, range.location <= marked.length else { return nil }
        let actual = NSRange(location: range.location, length: min(range.length, marked.length - range.location))
        actualRange?.pointee = actual; return marked.attributedSubstring(from: actual)
    }
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        actualRange?.pointee = NSRange(location: NSNotFound, length: 0)
        let point = inputModel.lastPoint ?? CGPoint(x: 0.5, y: 0.5)
        let local = NSRect(x: bounds.minX + bounds.width * point.x,
            y: bounds.minY + bounds.height * (1 - point.y), width: 1, height: 18)
        return window?.convertToScreen(convert(local, to: nil)) ?? .zero
    }
    func characterIndex(for point: NSPoint) -> Int { NSNotFound }
    override func doCommand(by selector: Selector) {
        guard ownsFocus, inputModel.acceptsInput else { return }
        if hasMarkedText() { if selector == #selector(NSResponder.cancelOperation(_:)) { clearComposition() }; return }
        if let event = interpreting { physicalKey(event, down: true) }
    }
    private let transparentCursor = NSCursor(image: NSImage(size: CGSize(width: 1, height: 1), flipped: false) { _ in
        NSColor.clear.set(); NSRect(x: 0, y: 0, width: 1, height: 1).fill(); return true
    }, hotSpot: .zero)
    override func resetCursorRects() {
        super.resetCursorRects()
        if separateCursor && inputModel.capabilities.enabled { addCursorRect(bounds, cursor: transparentCursor) }
    }
    private var latest: (CVPixelBuffer, UInt64)?
    private var submittedTimestamp: UInt64?
    private var redraw = true
    private let paced = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_PACED_RENDER"] == "1"
    private lazy var buffered = paced && ProcessInfo.processInfo.environment["EXTEND_DISPLAY_BUFFERED_RENDER"] == "1"
    private var presentationQueue = PresentationQueue<CVPixelBuffer>()
    // Render a newly selected frame immediately; notification-driven redraws
    // may coalesce multiple desktop frames before the next display cycle.
    // Launch options do not change mid-session. Foundation materializes the
    // environment dictionary on every lookup, so read this once at startup.
    private lazy var explicit: Bool = !paced && ProcessInfo.processInfo.environment["EXTEND_DISPLAY_EXPLICIT_RENDER"] != "0"
    private let presentationLock = NSLock()
    private var lastPresentedTimestamp: UInt64?
    private let renderReadiness = RenderReadiness()
    var renderedGPU: (() -> Void)? {
        get { renderReadiness.callback }
        set { renderReadiness.callback = newValue }
    }
    var presented: (() -> Void)?
    init(viewport: CGSize) throws {
        guard let device = MTLCreateSystemDefaultDevice(), let commands = device.makeCommandQueue() else {
            throw DisplayFailure("Metal rendering unavailable.")
        }
        self.commands = commands; self.context = CIContext(mtlDevice: device)
        super.init(frame: CGRect(origin: .zero, size: viewport), device: device)
        framebufferOnly = false; isPaused = explicit || !paced; enableSetNeedsDisplay = !paced && !explicit
        DisplayMetrics.shared.event("render_paced", bytes: paced ? 1 : 0)
        DisplayMetrics.shared.event("render_explicit", bytes: explicit ? 1 : 0)
        DisplayMetrics.shared.event("render_buffered", bytes: buffered ? 1 : 0)
        colorPixelFormat = .bgra8Unorm; delegate = self
        // CI renders sRGB bytes below. Tell Core Animation their source space
        // so wide-gamut or calibrated panels receive the correct conversion.
        colorspace = pipelineColorSpace
        wantsLayer = true; layer?.addSublayer(cursor.layer)
    }
    required init(coder: NSCoder) { fatalError("Not supported") }
    func present(_ image: CVPixelBuffer, timestamp: UInt64) {
        if buffered {
            presentationQueue.append(image, timestamp: timestamp, arrival: DispatchTime.now().uptimeNanoseconds)
        } else { latest = (image, timestamp) }
        if explicit { draw() } else { needsDisplay = true }
    }
    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        // Local window/fullscreen/backing-scale changes precede the negotiated
        // canvas update. End the old gesture; a fresh user action can request
        // ownership using the new viewport after the Rust revision arrives.
        stopInput()
        redraw = true; needsDisplay = true
        cursor.layout(viewport: bounds, source: logicalSource)
        if explicit { DispatchQueue.main.async { [weak self] in self?.draw() } }
    }
    func draw(in view: MTKView) {
        if buffered, let ready = presentationQueue.take(now: DispatchTime.now().uptimeNanoseconds,
                delay: UInt64(1_000_000_000 / max(1, preferredFramesPerSecond))) {
            latest = ready
        }
        guard let (image, timestamp) = latest else { return }
        // The timed mode samples the latest frame at the panel cadence. Skip
        // unchanged frames before acquiring a drawable or submitting GPU work.
        if paced && !redraw && submittedTimestamp == timestamp { return }
        let started = DispatchTime.now().uptimeNanoseconds
        DisplayMetrics.shared.event("render_started", timestamp: timestamp)
        guard let drawable = currentDrawable, let buffer = commands.makeCommandBuffer() else { return }
        // The SDR encoder declares sRGB transfer/709 primaries. Some hardware
        // decoded buffers have those tags but CIImage still reports no source
        // color space; supply the declared pipeline space explicitly.
        let source = CIImage(cvPixelBuffer: image,
            options: [.colorSpace: pipelineColorSpace])
        let size = drawableSize
        let scale = min(size.width / source.extent.width, size.height / source.extent.height)
        let transform = CGAffineTransform(translationX: (size.width - source.extent.width * scale) / 2,
                                         y: (size.height - source.extent.height * scale) / 2).scaledBy(x: scale, y: scale)
        let bounds = CGRect(origin: .zero, size: size)
        let fitted = source.transformed(by: transform).composited(over: CIImage(color: .black).cropped(to: bounds))
        context.render(fitted, to: drawable.texture, commandBuffer: buffer, bounds: bounds,
                       colorSpace: pipelineColorSpace)
        buffer.addCompletedHandler { [weak self] command in
            if command.status == .completed { DisplayMetrics.shared.event("gpu_completed", timestamp: timestamp) }
            self?.renderReadiness.complete(success: command.status == .completed)
        }
        drawable.addPresentedHandler { [weak self] drawable in
            guard let self, drawable.presentedTime.isFinite, drawable.presentedTime > 0 else { return }
            // Window redraws may present the same decoded frame again. Count
            // only new source frames, and timestamp compositor presentation,
            // rather than GPU completion or this handler's invocation time.
            self.presentationLock.lock()
            let fresh = self.lastPresentedTimestamp.map { timestamp > $0 } ?? true
            if fresh { self.lastPresentedTimestamp = timestamp }
            self.presentationLock.unlock()
            guard fresh else { return }
            DisplayMetrics.shared.event("presented", timestamp: timestamp,
                atNanoseconds: UInt64(drawable.presentedTime * 1e9))
            self.presented?()
        }
        buffer.present(drawable); buffer.commit()
        DisplayMetrics.shared.event("render_draw_us", timestamp: timestamp,
            bytes: Int((DispatchTime.now().uptimeNanoseconds - started) / 1000))
        submittedTimestamp = timestamp; redraw = false
    }
}

final class DisplayViewer: NSObject, NSWindowDelegate {
    let window: NSWindow
    var viewport: CGSize { view.bounds.size }
    var viewportCG: CGRect? {
        guard let screen = window.screen,
              let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return nil }
        return ViewerInputGeometry.screenRect(window.convertToScreen(view.convert(view.bounds, to: nil)),
            cocoaScreen: screen.frame, display: CGDisplayBounds(number.uint32Value))
    }
    private let view: VideoView
    private let lock = NSLock()
    private var pending: (CVPixelBuffer, UInt64)?
    private var pendingFrames: [(CVPixelBuffer, UInt64)] = []
    private let buffered = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_PACED_RENDER"] == "1"
        && ProcessInfo.processInfo.environment["EXTEND_DISPLAY_BUFFERED_RENDER"] == "1"
    private var scheduled = false
    private var pendingCursor: [Data?] = [nil, nil]
    private var cursorScheduled = false
    private var cursorExpiry: Timer?
    var input: ((Data) -> Void)?
    private lazy var inputOutput: ViewerInputOutput = {
        let output = ViewerInputOutput { [weak self] in self?.input?($0) }
        output.overflow = { [weak self] in self?.view.inputOverflow() }
        return output
    }()
    private var activationObserver: NSObjectProtocol?
    var closed: (() -> Void)?
    var viewportEvidence: ((Data) -> Void)?
    var renderedGPU: (() -> Void)? { get { view.renderedGPU } set { view.renderedGPU = newValue } }
    var presented: (() -> Void)? { get { view.presented } set { view.presented = newValue } }
    func configure(width: Int, height: Int, fps: Int) {
        view.logicalSource = CGSize(width: width, height: height)
        view.preferredFramesPerSecond = fps
        view.cursor.layout(viewport: view.bounds, source: view.logicalSource)
        view.needsDisplay = true
    }
    init(width: Int, height: Int, separateCursor: Bool = false) throws {
        view = try VideoView(viewport: CGSize(width: min(width, 1280), height: min(height, 800)))
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: min(width, 1280), height: min(height, 800)),
            styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        super.init()
        view.input = { [weak self] in self?.inputOutput.enqueue($0) }
        view.separateCursor = separateCursor
        view.logicalSource = CGSize(width: width, height: height)
        cursorExpiry = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            guard let self else { return }; self.view.cursor.layout(viewport: self.view.bounds, source: self.view.logicalSource)
        }
        window.title = "extend.computer"; window.contentView = view; window.delegate = self
        window.makeFirstResponder(view)
        window.acceptsMouseMovedEvents = true
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didResignActiveNotification, object: NSApp, queue: .main) { [weak self] _ in self?.view.stopInput() }
        window.collectionBehavior = [.fullScreenPrimary]
        window.center(); window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        DispatchQueue.main.async { [weak self] in self?.window.toggleFullScreen(nil) }
    }
    func inputState(_ bytes: Data) throws {
        guard bytes.count <= 1024,
              let message = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            throw DisplayFailure("Invalid viewer input state.")
        }
        let capabilities = try ViewerInputCapabilities(message: message)
        DispatchQueue.main.async { [weak self] in self?.view.configureInput(capabilities) }
    }
    func cursor(_ bytes: Data) {
        let value = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        let isShape = value?["png"] != nil
        let sequence = (value?[value?["png"] != nil ? "revision" : "sequence"] as? NSNumber)?.uint64Value ?? 0
        DisplayMetrics.shared.event(isShape ? "cursor_shape_received" : "cursor_position_received", timestamp: sequence)
        let queuedAt = DispatchTime.now().uptimeNanoseconds
        lock.lock(); pendingCursor[isShape ? 1 : 0] = bytes
        if cursorScheduled { lock.unlock(); return }; cursorScheduled = true; lock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            DisplayMetrics.shared.event("cursor_dispatch_wait_us",
                bytes: Int((DispatchTime.now().uptimeNanoseconds - queuedAt) / 1000))
            self.lock.lock(); let pending = self.pendingCursor; self.pendingCursor = [nil, nil]; self.cursorScheduled = false; self.lock.unlock()
            for bytes in pending.reversed().compactMap({ $0 }) {
                do {
                    try self.view.cursor.apply(bytes, viewport: self.view.bounds, source: self.view.logicalSource)
                    if DisplayMetrics.shared.enabled {
                        let value = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
                        let sequence = (value?[value?["png"] != nil ? "revision" : "sequence"] as? NSNumber)?.uint64Value ?? 0
                        DisplayMetrics.shared.event(value?["png"] != nil ? "cursor_shape_layer_submit" : "cursor_position_layer_submit", timestamp: sequence)
                    }
                }
                catch { self.view.cursor.layer.isHidden = true; DisplayMetrics.shared.event("cursor_invalid") }
            }
            DisplayMetrics.shared.event("cursor_layer_submit")
        }
    }
    func present(_ image: CVPixelBuffer, timestamp: UInt64) {
        lock.lock()
        if buffered {
            pendingFrames.append((image, timestamp))
            if pendingFrames.count > 3 { pendingFrames.removeFirst(pendingFrames.count - 3) }
        } else { pending = (image, timestamp) }
        if scheduled { lock.unlock(); return }
        scheduled = true; lock.unlock()
        // Only one main-thread presentation callback can wait at a time.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let frames = self.buffered ? self.pendingFrames : self.pending.map { [$0] } ?? []
            self.pending = nil; self.pendingFrames.removeAll(keepingCapacity: true)
            self.scheduled = false; self.lock.unlock()
            for image in frames {
                DisplayMetrics.shared.event("viewer_selected", timestamp: image.1)
                self.view.present(image.0, timestamp: image.1)
            }
        }
    }
    func windowWillClose(_ notification: Notification) { view.stopInput(); closed?() }
    func windowDidResignKey(_ notification: Notification) { view.stopInput() }
    func windowDidMiniaturize(_ notification: Notification) { view.stopInput() }
    func windowDidEnterFullScreen(_ notification: Notification) {
        if ProcessInfo.processInfo.environment["EXTEND_DISPLAY_METRICS"] != nil {
            // Report the actual video view even when auto-size is disabled and
            // the parent never requests a viewport fit. No coordinate guesses
            // or window text are needed for a scoped input verification.
            if let rect = viewportCG {
                let message: [String: Any] = ["type": "viewport_evidence",
                    "width": Int(floor(viewport.width)), "height": Int(floor(viewport.height)),
                    "scale": window.backingScaleFactor, "fullscreen": true,
                    "window_number": window.windowNumber,
                    "viewport_cg": ["x": Double(rect.minX), "y": Double(rect.minY),
                        "width": Double(rect.width), "height": Double(rect.height)]]
                if let data = try? JSONSerialization.data(withJSONObject: message) { viewportEvidence?(data) }
            }
            let geometry = "Display viewport: window=\(window.frame) view=\(view.frame) screen=\(window.screen?.frame ?? .zero) drawable=\(view.drawableSize)\n"
            FileHandle.standardError.write(Data(geometry.utf8))
            DisplayMetrics.shared.event("viewport_window_width", bytes: Int(window.frame.width))
            DisplayMetrics.shared.event("viewport_window_height", bytes: Int(window.frame.height))
            DisplayMetrics.shared.event("viewport_view_width", bytes: Int(view.frame.width))
            DisplayMetrics.shared.event("viewport_view_height", bytes: Int(view.frame.height))
            DisplayMetrics.shared.event("viewport_screen_width", bytes: Int(window.screen?.frame.width ?? 0))
            DisplayMetrics.shared.event("viewport_screen_height", bytes: Int(window.screen?.frame.height ?? 0))
        }
        DisplayMetrics.shared.event("fullscreen"); DisplayMetrics.shared.flush()
    }
    deinit {
        cursorExpiry?.invalidate()
        if let activationObserver { NotificationCenter.default.removeObserver(activationObserver) }
    }
}
