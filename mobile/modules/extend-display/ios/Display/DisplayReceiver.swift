import Foundation
import CoreVideo

/// Engine calls the receiver needs, bound to one display session id by the
/// engine wiring (ext_* C functions on iOS). Same calls, same order as
/// Android's MediaSession.receive():
///   attach → loop { state → (configured on revision change) → nextFrame → decode }
///   → rendered per presented frame → cancel when finished.
struct DisplayEngineCalls {
    /// attach_display(id)
    var attach: () throws -> Void
    /// display_state() JSON
    var state: () throws -> Data
    /// display_configured(id, revision)
    var configured: (_ revision: Int64) throws -> Void
    /// display_rendered(id, revision, presentation_us, device_ns, callback_device_ns)
    var rendered: (_ revision: Int64, _ presentationUs: Int64, _ deviceNs: Int64, _ callbackNs: Int64) throws -> Void
    /// next_display_frame(id): 17-byte header + Annex-B, or nil after the
    /// engine's ~20 ms wait. Throws once the session ended.
    var nextFrame: () throws -> Data?
    /// next_display_cursor(id): cursor JSON packet or nil. Optional; only used
    /// when the negotiated config has separate_cursor.
    var nextCursor: (() throws -> Data?)?
    /// cancel_display(id)
    var cancel: () -> Void
}

/// Drives one engine display session into a DisplayRenderer: a dedicated
/// frame thread decodes with VideoToolbox, the presenter renders immediately,
/// and a cursor thread feeds the overlay. Callbacks arrive on the main thread.
final class DisplayReceiver {
    /// Stream (re)configured: new raster/logical size; input must re-bind to `revision`.
    var onConfigured: ((DisplayStreamConfig, Int64) -> Void)?
    /// First frame of `revision` reached the screen ("streaming").
    var onFirstFrame: ((Int64) -> Void)?
    /// Session finished; nil error after stop().
    var onEnded: ((Error?) -> Void)?

    private let id: String
    private let engine: DisplayEngineCalls
    private let renderer: DisplayRenderer
    private let presenter: MetalFramePresenter
    private let lock = NSLock()
    private var running = false
    private var stopped = false
    private var outputError: Error?
    private var firstRendered: Int64 = -1
    private var cursorMailbox: [Data?] = [nil, nil]  // [position, shape]
    private var cursorScheduled = false

    init(id: String, engine: DisplayEngineCalls, renderer: DisplayRenderer) {
        self.id = id; self.engine = engine; self.renderer = renderer; presenter = renderer.presenter
    }

    /// Main thread. Starts the frame (and cursor) threads. `config` is
    /// display_state().config at start (revision 0), as Android passes it.
    func start(config: DisplayStreamConfig) {
        lock.lock(); guard !running, !stopped else { lock.unlock(); return }; running = true; lock.unlock()
        presenter.presented = { [weak self] in self?.presented($0) }
        let frames = Thread { [weak self] in self?.receive(initial: config) }
        frames.name = "extend-display-frames"; frames.qualityOfService = .userInteractive
        frames.start()
        if config.separateCursor, let next = engine.nextCursor {
            let cursor = Thread { [weak self] in self?.receiveCursor(next) }
            cursor.name = "extend-display-cursor"; cursor.qualityOfService = .userInteractive
            cursor.start()
        }
    }

    /// Any thread. Ends the session; the frame thread exits within ~20 ms.
    func stop() {
        lock.lock(); let wasStopped = stopped; stopped = true; lock.unlock()
        if !wasStopped { engine.cancel() }
    }

    private var isStopped: Bool { lock.lock(); defer { lock.unlock() }; return stopped }

    private func fail(_ error: Error) {
        lock.lock(); if outputError == nil { outputError = error }; lock.unlock()
    }

    private func configure(_ config: DisplayStreamConfig, revision: Int64) {
        // Accept frames for the new revision before any can be decoded; the
        // view layout follows on the main thread.
        presenter.configure(revision: revision, rasterWidth: config.rasterWidth, rasterHeight: config.rasterHeight)
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.isStopped else { return }
            self.renderer.updateLayout(for: config)
            self.onConfigured?(config, revision)
        }
    }

    private func presented(_ frame: MetalFramePresenter.Presented) {
        guard !isStopped else { return }
        do {
            try engine.rendered(frame.revision, Int64(frame.timestamp / 1000),
                                Int64(clamping: frame.presentedNs), Int64(clamping: frame.callbackNs))
        } catch { fail(error); return }
        lock.lock(); let first = firstRendered != frame.revision; firstRendered = frame.revision; lock.unlock()
        if first {
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.isStopped else { return }
                self.onFirstFrame?(frame.revision)
            }
        }
    }

    private func receive(initial: DisplayStreamConfig) {
        var config = initial
        var revision: Int64 = 0
        var decoder: VideoDecoder?
        var previous: Int64 = -1
        var failures = 0
        let decodeFailed = LockedValue(false)
        var ending: Error?
        configure(config, revision: revision)
        do {
            try engine.attach()
            while !isStopped {
                lock.lock(); let failed = outputError; lock.unlock()
                if let failed { throw failed }
                let state = try DisplaySessionSnapshot(json: engine.state())
                guard state.id == id else { throw DisplayFailure("Display session replaced") }
                if state.revision != revision {
                    decoder?.drain(); decoder = nil
                    guard let next = state.config else { throw DisplayFailure("Invalid display configuration") }
                    guard next.codec == config.codec else { throw DisplayFailure("Live codec changes require reconnect") }
                    config = next; revision = state.revision
                    configure(config, revision: revision)
                    try engine.configured(revision)
                }
                guard let payload = try engine.nextFrame() else { continue }
                let packet = try DisplayFramePacket(payload)
                if packet.revision != revision { continue }
                guard Int64(packet.timestamp) > previous else { throw DisplayFailure("Invalid video timestamp") }
                previous = Int64(packet.timestamp)
                if decodeFailed.exchange(false) { decoder = nil }
                if decoder == nil {
                    // Start (or recover, e.g. after iOS invalidated the session
                    // in the background) only at a keyframe with parameter sets.
                    guard packet.keyframe else { continue }
                    let frameRevision = revision
                    decoder = VideoDecoder(codec: config.codec,
                        output: { [presenter] image, timestamp in
                            presenter.present(image, timestamp: timestamp, revision: frameRevision)
                        },
                        failure: { decodeFailed.store(true) })
                }
                do {
                    try decoder?.decode(packet.videoFrame, wait: .now() + 1)
                    failures = 0
                } catch let error as DisplayFailure where error.description.hasPrefix("Decoder is busy") {
                    throw DisplayFailure("Hardware decoder stalled")
                } catch {
                    // Bad data or an invalidated session: wait for the next keyframe.
                    decoder = nil; failures += 1
                    if failures >= 8 { throw error }
                }
            }
        } catch { if !isStopped { ending = error } }
        engine.cancel()
        presenter.presented = nil
        decoder?.drain(); decoder = nil
        presenter.reset()
        lock.lock(); stopped = true; lock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.renderer.reset()
            self.onEnded?(ending)
        }
    }

    private func receiveCursor(_ next: () throws -> Data?) {
        do {
            while !isStopped {
                guard let bytes = try next() else { continue }
                guard bytes.count <= CursorPacket.maxBytes else { throw DisplayFailure("Cursor packet too large.") }
                // Latest shape and latest position win; one main-thread hop at a time.
                let isShape = bytes.range(of: Data("\"png\"".utf8)) != nil
                lock.lock()
                cursorMailbox[isShape ? 1 : 0] = bytes
                let schedule = !cursorScheduled; cursorScheduled = true
                lock.unlock()
                if schedule { DispatchQueue.main.async { [weak self] in self?.flushCursor() } }
            }
        } catch {
            // Like Android, a cursor failure hides the cursor but keeps video.
            DispatchQueue.main.async { [weak self] in self?.renderer.cursor.reset() }
        }
    }

    private func flushCursor() {
        lock.lock(); let pending = cursorMailbox; cursorMailbox = [nil, nil]; cursorScheduled = false
        let stopped = stopped; lock.unlock()
        guard !stopped else { return }
        // Shape first so a position for the new revision can show immediately.
        for bytes in pending.reversed().compactMap({ $0 }) {
            do { try renderer.applyCursor(bytes) } catch { renderer.cursor.reset() }
        }
    }
}

/// Minimal lock-based flag shared with decoder callbacks.
final class LockedValue<Value> {
    private let lock = NSLock()
    private var value: Value
    init(_ value: Value) { self.value = value }
    func load() -> Value { lock.lock(); defer { lock.unlock() }; return value }
    func store(_ next: Value) { lock.lock(); value = next; lock.unlock() }
    func exchange(_ next: Value) -> Value { lock.lock(); defer { lock.unlock() }; let old = value; value = next; return old }
}
