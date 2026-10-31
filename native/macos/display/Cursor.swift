import AppKit
import ImageIO
import QuartzCore
import Darwin

// Compare the image bytes before doing PNG compression. NSImage can change in
// place, so object identity alone cannot safely detect an unchanged cursor.
private final class CursorImageCache {
    private var previousTIFF: Data?
    private var previousPNG: Data?
    func png(for image: NSImage) -> Data? {
        guard let tiff = image.tiffRepresentation else { return nil }
        if tiff == previousTIFF { return previousPNG }
        guard let bitmap = NSBitmapImageRep(data: tiff),
              bitmap.pixelsWide <= 512, bitmap.pixelsHigh <= 512,
              let png = bitmap.representation(using: .png, properties: [:]),
              png.count <= 90 * 1024 else { return nil }
        previousTIFF = tiff; previousPNG = png
        return png
    }
}

// Both capture and cursor use the same fitted source rectangle. Mirror output
// can have a different aspect or density from the physical source display.
struct CursorCaptureGeometry: Equatable {
    let source: CGRect
    let canvas: CGSize
    init?(source: CGRect, canvas: CGSize) {
        guard source.origin.x.isFinite, source.origin.y.isFinite,
            source.width.isFinite, source.height.isFinite,
            canvas.width.isFinite, canvas.height.isFinite,
            source.width > 0, source.height > 0, canvas.width > 0, canvas.height > 0 else { return nil }
        self.source = source; self.canvas = canvas
    }
    var scale: CGFloat { min(canvas.width / source.width, canvas.height / source.height) }
    var fitted: CGRect {
        let width = source.width * scale, height = source.height * scale
        return CGRect(x: (canvas.width - width) / 2, y: (canvas.height - height) / 2,
            width: width, height: height)
    }
    func normalized(_ point: CGPoint) -> CGPoint {
        let rectangle = fitted
        return CGPoint(x: min(1, max(0, (rectangle.minX + (point.x - source.minX) * scale) / canvas.width)),
            y: min(1, max(0, (rectangle.minY + (point.y - source.minY) * scale) / canvas.height)))
    }
}

// A single immutable shape/geometry snapshot is shared with the position
// worker. No pending cursor FIFO grows while output is blocked.
private struct CursorCaptureSnapshot {
    let revision: UInt64
    let geometry: CursorCaptureGeometry
    let shapePacket: Data
}
private final class CursorCaptureSnapshotStore: @unchecked Sendable {
    private let lock = NSLock()
    private var generation: UInt64 = 0
    private var active = false
    private var latest: CursorCaptureSnapshot?
    func activate() -> UInt64 {
        lock.lock(); defer { lock.unlock() }
        generation &+= 1; active = true; latest = nil
        return generation
    }
    func publish(_ snapshot: CursorCaptureSnapshot, generation expected: UInt64) {
        lock.lock(); defer { lock.unlock() }
        if active && generation == expected { latest = snapshot }
    }
    func current(generation expected: UInt64) -> CursorCaptureSnapshot? {
        lock.lock(); defer { lock.unlock() }
        return active && generation == expected ? latest : nil
    }
    func stop() {
        lock.lock(); defer { lock.unlock() }
        active = false; latest = nil
    }
}

// AppKit shape queries stay on the main run loop. Position and framed output
// share one serial user-interactive worker, independent of shape-query cost.
final class CursorCapture: @unchecked Sendable {
    private var timer: Timer?
    private var positionTimer: DispatchSourceTimer?
    private var shapeWake: DispatchSourceUserDataAdd?
    private let positionQueue = DispatchQueue(label: "computer.extend.display.cursor", qos: .userInteractive)
    private let snapshots = CursorCaptureSnapshotStore()
    private let visibility: (@convention(c) () -> Int32)? = {
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGCursorIsVisible") else { return nil }
        return unsafeBitCast(symbol, to: (@convention(c) () -> Int32).self)
    }()
    // Main-run-loop state.
    private var revision: UInt64 = 0
    private var previousImage: Data?
    private let imageCache = CursorImageCache()
    private var previousHotspot = CGPoint.zero
    private var previousSize = CGSize.zero
    private var canvas = CGSize.zero
    private var previousGeometry: CursorCaptureGeometry?
    // Serial position-worker state; main never reads or mutates these fields.
    private var workerGeneration: UInt64 = 0
    private var sequence: UInt64 = 0
    private var sentRevision: UInt64?
    private var previousPosition: CGPoint?
    private var previousVisible = false
    private var lastSend = Date.distantPast
    private var previousSampleTime: UInt64 = 0

    func setCanvas(_ size: CGSize) { canvas = size }
    func start(display: UInt32, canvas: CGSize) {
        stop()
        self.canvas = canvas
        // Every start must publish an initial shape even if its image is unchanged.
        previousGeometry = nil
        let generation = snapshots.activate()
        // A data source merges pending publications into one wake. Both it and
        // the position timer execute on the same serial writer queue.
        let wake = DispatchSource.makeUserDataAddSource(queue: positionQueue)
        wake.setEventHandler { [weak self] in self?.samplePosition(generation: generation, timerTick: false) }
        shapeWake = wake; wake.resume()
        queryShape(display: display, generation: generation)
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 120, repeats: true) { [weak self] _ in
            self?.queryShape(display: display, generation: generation)
        }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
        let worker = DispatchSource.makeTimerSource(queue: positionQueue)
        worker.schedule(deadline: .now(), repeating: 1.0 / 120, leeway: .microseconds(250))
        worker.setEventHandler { [weak self] in self?.samplePosition(generation: generation, timerTick: true) }
        positionTimer = worker; worker.resume()
    }
    private func queryShape(display: UInt32, generation: UInt64) {
        let bounds = CGDisplayBounds(display)
        guard let geometry = CursorCaptureGeometry(source: bounds, canvas: canvas) else { return }
        let measuring = DisplayMetrics.shared.enabled
        let queryStart = measuring ? DispatchTime.now().uptimeNanoseconds : 0
        let cursor = NSCursor.currentSystem ?? .arrow
        let image = cursor.image
        let png = imageCache.png(for: image)
        let queryEnd = measuring ? DispatchTime.now().uptimeNanoseconds : 0
        if queryStart != 0 {
            DisplayMetrics.shared.event("cursor_shape_query_us", bytes: Int((queryEnd - queryStart) / 1000), atNanoseconds: queryEnd)
        }
        guard let png,
            geometry != previousGeometry || png != previousImage || cursor.hotSpot != previousHotspot || image.size != previousSize else { return }
        let nextRevision = revision + 1
        let state: [String: Any] = ["revision": nextRevision, "png": png.base64EncodedString(),
            "width": image.size.width * geometry.scale, "height": image.size.height * geometry.scale,
            "hotspot_x": cursor.hotSpot.x * geometry.scale, "hotspot_y": cursor.hotSpot.y * geometry.scale]
        guard let data = try? JSONSerialization.data(withJSONObject: state) else { return }
        revision = nextRevision; previousImage = png; previousHotspot = cursor.hotSpot
        previousSize = image.size; previousGeometry = geometry
        DisplayMetrics.shared.event("cursor_shape_query_completed", timestamp: revision, atNanoseconds: measuring ? queryEnd : nil)
        snapshots.publish(CursorCaptureSnapshot(revision: revision, geometry: geometry, shapePacket: data), generation: generation)
        shapeWake?.add(data: 1)
    }
    private func samplePosition(generation: UInt64, timerTick: Bool) {
        guard let snapshot = snapshots.current(generation: generation) else { return }
        let measuring = DisplayMetrics.shared.enabled
        let tick = measuring ? DispatchTime.now().uptimeNanoseconds : 0
        if workerGeneration != generation {
            workerGeneration = generation; sentRevision = nil; previousPosition = nil
            previousVisible = false; lastSend = .distantPast; previousSampleTime = 0
        }
        if timerTick && tick != 0 {
            if previousSampleTime != 0 {
                DisplayMetrics.shared.event("cursor_sample_interval_us", bytes: Int((tick - previousSampleTime) / 1000))
            }
            previousSampleTime = tick
        }
        let shapeChanged = sentRevision != snapshot.revision
        if shapeChanged {
            DisplayMetrics.shared.event("cursor_shape_submitted", timestamp: snapshot.revision)
            packet(3, snapshot.shapePacket, cursorIdentity: snapshot.revision, cursorShape: true)
            sentRevision = snapshot.revision
        }
        // Query the position AFTER a potentially blocking shape write. Use the
        // exact immutable geometry/revision whose shape was just sent.
        guard snapshots.current(generation: generation) != nil else { return }
        let sampled = measuring ? DispatchTime.now().uptimeNanoseconds : 0
        guard let point = CGEvent(source: nil)?.location else { return }
        let located = measuring ? DispatchTime.now().uptimeNanoseconds : 0
        let visible = snapshot.geometry.source.contains(point) && (visibility.map { $0() != 0 } ?? true)
        let normalized = snapshot.geometry.normalized(point), now = Date()
        if shapeChanged || normalized != previousPosition || visible != previousVisible || now.timeIntervalSince(lastSend) >= 0.5 {
            sequence += 1; previousPosition = normalized; previousVisible = visible; lastSend = now
            if measuring {
                DisplayMetrics.shared.event("cursor_position_sample_started", timestamp: sequence, atNanoseconds: sampled)
                DisplayMetrics.shared.event("cursor_position_sampled", timestamp: sequence, atNanoseconds: located)
                DisplayMetrics.shared.event("cursor_location_query_us", timestamp: sequence, bytes: Int((located - sampled) / 1000), atNanoseconds: located)
            }
            DisplayMetrics.shared.event("cursor_position_submitted", timestamp: sequence)
            let state: [String: Any] = ["revision": snapshot.revision, "sequence": sequence,
                "x": normalized.x, "y": normalized.y, "visible": visible]
            if let data = try? JSONSerialization.data(withJSONObject: state), snapshots.current(generation: generation) != nil {
                packet(3, data, cursorIdentity: sequence, cursorShape: false)
            }
        }
    }
    func stop() {
        snapshots.stop()
        timer?.invalidate(); timer = nil
        positionTimer?.cancel(); positionTimer = nil
        shapeWake?.cancel(); shapeWake = nil
    }
    deinit { stop() }
}

struct CursorPosition: Decodable {
    let revision: UInt64, sequence: UInt64
    let x: Double, y: Double, visible: Bool
}
struct CursorShape: Decodable {
    let revision: UInt64, png: String
    let width: Double, height: Double, hotspot_x: Double, hotspot_y: Double
}

final class CursorOverlay {
    let layer = CALayer()
    private var shape: CursorShape?
    private var position: CursorPosition?
    private var lastSequence: UInt64 = 0
    private var lastVisibleRevision: UInt64 = 0
    private var lastPosition = Date.distantPast
    init() {
        layer.isHidden = true
        layer.actions = ["position": NSNull(), "bounds": NSNull(), "contents": NSNull(), "hidden": NSNull()]
    }
    func apply(_ bytes: Data, viewport: CGRect, source: CGSize) throws {
        guard bytes.count <= 128 * 1024 else { throw DisplayFailure("Cursor packet too large.") }
        let object = try JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        if object?["png"] != nil {
            let next = try JSONDecoder().decode(CursorShape.self, from: bytes)
            guard next.width.isFinite, next.height.isFinite, next.width > 0, next.height > 0,
                next.width <= 512, next.height <= 512,
                next.hotspot_x.isFinite, next.hotspot_y.isFinite,
                (0...next.width).contains(next.hotspot_x), (0...next.height).contains(next.hotspot_y),
                let data = Data(base64Encoded: next.png), data.count <= 90 * 1024,
                let imageSource = CGImageSourceCreateWithData(data as CFData, nil),
                let properties = CGImageSourceCopyPropertiesAtIndex(imageSource, 0, nil) as? [CFString: Any],
                let width = properties[kCGImagePropertyPixelWidth] as? Int,
                let height = properties[kCGImagePropertyPixelHeight] as? Int,
                width > 0, height > 0, width <= 512, height <= 512,
                let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
                throw DisplayFailure("Invalid cursor image.")
            }
            if next.revision >= (shape?.revision ?? 0) { shape = next; layer.contents = image }
        } else {
            let next = try JSONDecoder().decode(CursorPosition.self, from: bytes)
            guard next.x.isFinite, next.y.isFinite, (0...1).contains(next.x), (0...1).contains(next.y) else {
                throw DisplayFailure("Invalid cursor position.")
            }
            if next.sequence > lastSequence { position = next; lastSequence = next.sequence; lastPosition = Date() }
        }
        layout(viewport: viewport, source: source)
    }
    func layout(viewport: CGRect, source: CGSize) {
        guard let shape, let position, shape.revision == position.revision, position.visible,
              Date().timeIntervalSince(lastPosition) < 2, source.width > 0, source.height > 0 else {
            layer.isHidden = true; return
        }
        let scale = min(viewport.width / source.width, viewport.height / source.height)
        let fitted = CGRect(x: viewport.midX - source.width * scale / 2,
                            y: viewport.midY - source.height * scale / 2,
                            width: source.width * scale, height: source.height * scale)
        layer.frame = CGRect(x: fitted.minX + position.x * fitted.width - shape.hotspot_x * scale,
                             y: fitted.maxY - position.y * fitted.height - (shape.height - shape.hotspot_y) * scale,
                             width: shape.width * scale, height: shape.height * scale)
        layer.isHidden = false
        if shape.revision != lastVisibleRevision {
            lastVisibleRevision = shape.revision
            DisplayMetrics.shared.event("cursor_shape_visible_layer_submit", timestamp: shape.revision)
        }
    }
}

func cursorSelfTest() throws {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 24, pixelsHigh: 24,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    memset(bitmap.bitmapData, 255, bitmap.bytesPerRow * bitmap.pixelsHigh)
    let png = bitmap.representation(using: .png, properties: [:])!.base64EncodedString()
    let image = NSImage(size: CGSize(width: 24, height: 24))
    image.addRepresentation(bitmap)
    let cache = CursorImageCache()
    guard let initial = cache.png(for: image), cache.png(for: image) == initial else {
        throw DisplayFailure("Unchanged cursor image cache failed.")
    }
    // A mutable representation can change while the NSImage identity remains
    // unchanged. The cache must pick up that change rather than reuse old PNG.
    memset(bitmap.bitmapData, 0, bitmap.bytesPerRow * bitmap.pixelsHigh)
    guard let changed = cache.png(for: image), changed != initial else {
        throw DisplayFailure("Changed cursor image cache failed.")
    }
    let snapshotGeometry = CursorCaptureGeometry(source: CGRect(x: 0, y: 0, width: 100, height: 100), canvas: CGSize(width: 100, height: 100))!
    let snapshots = CursorCaptureSnapshotStore(), firstGeneration = snapshots.activate()
    snapshots.publish(CursorCaptureSnapshot(revision: 1, geometry: snapshotGeometry, shapePacket: Data([1])), generation: firstGeneration)
    snapshots.publish(CursorCaptureSnapshot(revision: 2, geometry: snapshotGeometry, shapePacket: Data([2])), generation: firstGeneration)
    guard snapshots.current(generation: firstGeneration)?.revision == 2 else {
        throw DisplayFailure("Cursor snapshot did not retain the latest complete shape.")
    }
    snapshots.stop()
    let nextGeneration = snapshots.activate()
    snapshots.publish(CursorCaptureSnapshot(revision: 3, geometry: snapshotGeometry, shapePacket: Data([3])), generation: firstGeneration)
    guard snapshots.current(generation: firstGeneration) == nil, snapshots.current(generation: nextGeneration) == nil else {
        throw DisplayFailure("Stopped cursor generation published into its replacement.")
    }
    snapshots.publish(CursorCaptureSnapshot(revision: 4, geometry: snapshotGeometry, shapePacket: Data([4])), generation: nextGeneration)
    guard snapshots.current(generation: nextGeneration)?.shapePacket == Data([4]) else {
        throw DisplayFailure("Fresh cursor generation did not accept its immutable shape.")
    }
    let overlay = CursorOverlay(); let viewport = CGRect(x: 0, y: 0, width: 1000, height: 800)
    let source = CGSize(width: 1000, height: 500)
    func apply(_ value: [String: Any]) throws {
        try overlay.apply(JSONSerialization.data(withJSONObject: value), viewport: viewport, source: source)
    }
    try apply(["revision": 1, "png": png, "width": 20, "height": 30, "hotspot_x": 3, "hotspot_y": 5])
    try apply(["revision": 1, "sequence": 1, "x": 0.25, "y": 0.75, "visible": true])
    guard !overlay.layer.isHidden, overlay.layer.frame == CGRect(x: 247, y: 250, width: 20, height: 30) else {
        throw DisplayFailure("Cursor letterbox/Retina/hotspot mapping failed.")
    }
    try apply(["revision": 1, "sequence": 1, "x": 0, "y": 0, "visible": true])
    guard overlay.layer.frame.minX == 247 else { throw DisplayFailure("Stale cursor position applied.") }
    try apply(["revision": 1, "sequence": 2, "x": 0.25, "y": 0.75, "visible": false])
    guard overlay.layer.isHidden else { throw DisplayFailure("Cursor visibility failed.") }
    // A new shape cannot use the previous revision's position. Publishing a
    // matching position at the same coordinates restores visibility immediately.
    try apply(["revision": 2, "png": png, "width": 20, "height": 30, "hotspot_x": 3, "hotspot_y": 5])
    guard overlay.layer.isHidden else { throw DisplayFailure("Mismatched cursor revision displayed.") }
    try apply(["revision": 2, "sequence": 3, "x": 0.25, "y": 0.75, "visible": true])
    guard !overlay.layer.isHidden, overlay.layer.frame.minX == 247 else {
        throw DisplayFailure("Stationary cursor shape revision did not restore visibility.")
    }
    var rejected = false
    do { try apply(["revision": 2, "sequence": 4, "x": -1, "y": 0, "visible": true]) }
    catch { rejected = true }
    guard rejected else { throw DisplayFailure("Invalid cursor coordinates accepted.") }
    // Mirror capture letterboxes the physical source inside the encoded canvas;
    // the receiver then fits that whole canvas into its own viewport. Check the
    // composed transform, including a negative Quartz display origin.
    let mirror = CursorCaptureGeometry(source: CGRect(x: -2000, y: -100, width: 2000, height: 1000),
        canvas: CGSize(width: 1000, height: 800))!
    guard mirror.fitted == CGRect(x: 0, y: 150, width: 1000, height: 500), mirror.scale == 0.5,
        mirror.normalized(CGPoint(x: -1000, y: 400)) == CGPoint(x: 0.5, y: 0.5) else {
        throw DisplayFailure("Mirror source rectangle or negative origin mapping failed.")
    }
    let mirrorPosition = mirror.normalized(CGPoint(x: -1500, y: 650))
    let mirrorOverlay = CursorOverlay()
    let mirrorShape: [String: Any] = ["revision": 1, "png": png,
        "width": 20 * mirror.scale, "height": 30 * mirror.scale,
        "hotspot_x": 3 * mirror.scale, "hotspot_y": 5 * mirror.scale]
    try mirrorOverlay.apply(JSONSerialization.data(withJSONObject: mirrorShape), viewport: viewport, source: mirror.canvas)
    try mirrorOverlay.apply(JSONSerialization.data(withJSONObject: ["revision": 1, "sequence": 1,
        "x": mirrorPosition.x, "y": mirrorPosition.y, "visible": true]), viewport: viewport, source: mirror.canvas)
    guard !mirrorOverlay.layer.isHidden,
        mirrorOverlay.layer.frame == CGRect(x: 248.5, y: 262.5, width: 10, height: 15) else {
        throw DisplayFailure("Mirror letterbox cursor position, size or hotspot failed.")
    }
    let portrait = CursorCaptureGeometry(source: CGRect(x: 0, y: 0, width: 1000, height: 2000), canvas: mirror.canvas)!
    let resized = CursorCaptureGeometry(source: mirror.source, canvas: CGSize(width: 2000, height: 1600))!
    guard portrait.fitted == CGRect(x: 300, y: 0, width: 400, height: 800),
        portrait.normalized(.zero) == CGPoint(x: 0.3, y: 0),
        resized != mirror, resized.scale == 1,
        resized.normalized(CGPoint(x: -1500, y: 650)) == mirrorPosition,
        CursorCaptureGeometry(source: .zero, canvas: mirror.canvas) == nil,
        CursorCaptureGeometry(source: mirror.source, canvas: CGSize(width: CGFloat.infinity, height: 800)) == nil else {
        throw DisplayFailure("Mirror portrait, live canvas resize or bounds validation failed.")
    }
    print("PASS: bounded generation-scoped cursor snapshots, cursor image cache and in-place changes, hotspot, Retina logical size, letterboxing, hidden state, stationary shape revision recovery, stale sequence, mirror source/canvas geometry, negative origins, portrait fitting, live canvas resize and bounds validation.")
}
