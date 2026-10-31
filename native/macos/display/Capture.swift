import Foundation
import ScreenCaptureKit
import CoreGraphics
import CoreMedia
import CoreVideo

final class DisplayCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    private var stream: SCStream?
    private let cursor = CursorCapture()
    private var virtual: UnsafeMutableRawPointer?
    private let queue = DispatchQueue(label: "computer.extend.display.capture", qos: .userInteractive)
    private var encoder: VideoEncoder
    private var configuring = false
    private var profile: (width: Int, height: Int, retina: Bool, fps: Int)?
    private var mode = "extend"
    private var capturedDisplay: UInt32?
    private var separateCursor = false
    private var benchmarkMarker: BenchmarkMarker?
    private var reportedColor = false
    private var diagnostics = CaptureDiagnostics(enabled: DisplayMetrics.shared.enabled)
    private let startupDeadline = CaptureStartupDeadline()
    private var firstImageObserved = false
    private let failure: @Sendable (Error) -> Void
    private weak var lifecycle: CaptureLifecycle?
    init(encoder: VideoEncoder, lifecycle: CaptureLifecycle, failure: @escaping @Sendable (Error) -> Void) {
        self.encoder = encoder; self.lifecycle = lifecycle; self.failure = failure
    }
    private func checkLifetime() throws {
        if lifecycle?.isClosing == true { throw CancellationError() }
    }
    func start(mode: String, width: Int, height: Int, retina: Bool, displayID: UInt32?, fps: Int, separateCursor: Bool = false) async throws {
        try checkLifetime()
        self.mode = mode; self.separateCursor = separateCursor
        profile = (width, height, retina, fps)
        benchmarkMarker = BenchmarkMarker(logicalWidth: width, logicalHeight: height)
        guard CGPreflightScreenCaptureAccess() else {
            _ = CGRequestScreenCaptureAccess()
            throw DisplayFailure("Allow Screen Recording for extend.computer Display in macOS Settings, then reconnect.")
        }
        var selected = displayID
        if mode == "extend" {
            let lifetime = lifecycle
            let created = await MainActor.run { () -> (UInt, UInt32, String) in
                guard lifetime?.isClosing != true else { return (0, 0, "closing") }
                var identifier: UInt32 = 0
                let pointer = ECDisplayCreate(UInt32(width), UInt32(height), retina, UInt32(fps), &identifier)
                return (UInt(bitPattern: pointer), identifier, pointer == nil ? String(cString: ECDisplayLastFailure()) : "")
            }
            virtual = UnsafeMutableRawPointer(bitPattern: created.0)
            try checkLifetime()
            guard virtual != nil else { throw DisplayFailure("macOS could not create the extended display (\(created.2)).") }
            selected = created.1
        }
        var display: SCDisplay?
        // Virtual displays become visible to ScreenCaptureKit asynchronously.
        for _ in 0..<20 {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            try checkLifetime()
            display = content.displays.first { $0.displayID == (selected ?? CGMainDisplayID()) }
            if display != nil { break }
            try await Task.sleep(nanoseconds: 100_000_000)
            try checkLifetime()
        }
        guard let display else { throw DisplayFailure("Selected display is no longer available.") }
        capturedDisplay = display.displayID
        if mode == "extend" {
            let bounds = CGDisplayBounds(display.displayID)
            guard Int(bounds.width) == width, Int(bounds.height) == height else {
                throw DisplayFailure("macOS applied an unexpected virtual display size.")
            }
        }
        let configuration = SCStreamConfiguration()
        configuration.width = width * (retina ? 2 : 1)
        configuration.height = height * (retina ? 2 : 1)
        if mode == "mirror" { configuration.destinationRect = try mirrorDestination(width: width, height: height, retina: retina) }
        let rgbCapture = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_CAPTURE_BGRA"] == "1"
        configuration.pixelFormat = rgbCapture ? kCVPixelFormatType_32BGRA : kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        // Transport SDR in a defined space; the source panel may use Display P3.
        configuration.colorSpaceName = CGColorSpace.sRGB
        if !rgbCapture { configuration.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2 }
        // The virtual display already refreshes at 60 Hz. Capture its native
        // cadence so interval rounding does not skip refreshes. Other profiles
        // retain their requested capture limit.
        configuration.minimumFrameInterval = mode == "extend" && fps == 60
            ? .zero : CMTime(value: 1, timescale: Int32(fps))
        configuration.queueDepth = 3
        configuration.showsCursor = !separateCursor
        configuration.capturesAudio = false
        configuration.scalesToFit = true
        if #available(macOS 14.0, *) { configuration.preservesAspectRatio = true }
        let filter = SCContentFilter(display: display, excludingWindows: [])
        let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
        self.stream = stream
        try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
        try await stream.startCapture()
        try checkLifetime()
        if let deadline = startupDeadline.started(at: DispatchTime.now().uptimeNanoseconds) {
            let gate = startupDeadline, reportFailure = failure
            DispatchQueue.global(qos: .utility).asyncAfter(deadline: DispatchTime(uptimeNanoseconds: deadline)) { [weak gate] in
                guard let gate, gate.timeout(at: DispatchTime.now().uptimeNanoseconds) else { return }
                reportFailure(DisplayFailure(CaptureStartupDeadline.failureMessage))
            }
        }
        if separateCursor {
            let id = display.displayID, canvas = CGSize(width: width, height: height)
            await MainActor.run { [cursor] in if self.lifecycle?.isClosing != true { cursor.start(display: id, canvas: canvas) } }
        }
    }
    func inputTarget(configurationID: Int) throws -> [String: Any] {
        guard let capturedDisplay, let profile, CGDisplayIsActive(capturedDisplay) != 0 else {
            throw DisplayFailure("Captured display is no longer active.")
        }
        let bounds = CGDisplayBounds(capturedDisplay)
        let canvas = CGSize(width: profile.width, height: profile.height)
        guard let geometry = CursorCaptureGeometry(source: bounds, canvas: canvas) else {
            throw DisplayFailure("Captured display geometry is invalid.")
        }
        let content = mode == "mirror" ? geometry.fitted : CGRect(origin: .zero, size: canvas)
        let rect: (CGRect) -> [String: Double] = { ["x": $0.minX, "y": $0.minY, "width": $0.width, "height": $0.height] }
        let scale = profile.retina ? 2 : 1
        return ["configuration_id": configurationID, "display_id": capturedDisplay,
                "bounds": rect(bounds), "content": rect(content),
                "canvas": ["width": profile.width, "height": profile.height],
                "raster": ["width": profile.width * scale, "height": profile.height * scale]]
    }
    private func mirrorDestination(width: Int, height: Int, retina: Bool) throws -> CGRect {
        guard let capturedDisplay,
            let geometry = CursorCaptureGeometry(source: CGDisplayBounds(capturedDisplay), canvas: CGSize(width: width, height: height)) else {
            throw DisplayFailure("Selected display is no longer available.")
        }
        let rectangle = geometry.fitted, scale = retina ? 2.0 : 1.0
        return CGRect(x: rectangle.minX * scale, y: rectangle.minY * scale,
            width: rectangle.width * scale, height: rectangle.height * scale)
    }
    private func diagnostic(_ stage: CaptureDiagnostics.Stage) {
        guard let count = diagnostics.record(stage) else { return }
        DisplayMetrics.shared.event(stage.metric, bytes: Int(clamping: count))
    }
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        diagnostic(.callback)
        guard lifecycle?.isClosing != true else { return }
        guard !configuring else { diagnostic(.configuring); return }
        guard type == .screen else { diagnostic(.wrongType); return }
        guard sampleBuffer.isValid else { diagnostic(.invalidSample); return }
        diagnostic(.sampleValid)
        guard let rawAttachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) else {
            diagnostic(.noAttachments); return
        }
        guard let attachments = rawAttachments as? [[SCStreamFrameInfo: Any]] else {
            diagnostic(.attachmentCast); return
        }
        guard let first = attachments.first else { diagnostic(.emptyAttachments); return }
        guard let rawStatus = first[.status] else { diagnostic(.noStatus); return }
        guard let status = rawStatus as? Int else { diagnostic(.statusCast); return }
        guard status == SCFrameStatus.complete.rawValue else {
            DisplayMetrics.shared.event("capture_incomplete", bytes: status); return
        }
        guard let image = CMSampleBufferGetImageBuffer(sampleBuffer) else {
            diagnostic(.noImage)
            DisplayMetrics.shared.event("capture_incomplete", bytes: status); return
        }
        // Only the first complete, valid image can satisfy initial capture
        // readiness. Reconfiguration and idle callbacks cannot extend its bound.
        if !firstImageObserved {
            firstImageObserved = true
            if startupDeadline.acceptedImage(at: DispatchTime.now().uptimeNanoseconds) {
                failure(DisplayFailure(CaptureStartupDeadline.failureMessage)); return
            }
        }
        let timestamp = UInt64(max(0, CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sampleBuffer))) * 1e9)
        DisplayMetrics.shared.event("capture_complete", timestamp: timestamp)
        if !reportedColor { reportPixelColor(image, stage: "captured") }
        // ScreenCaptureKit's explicitly requested sRGB output has an NV12
        // Rec.709 transfer attachment. Passing it unchanged to an sRGB encoder
        // adds a transfer conversion (e.g. middle gray 128 becomes 140).
        // Exact registered fixtures validate the sRGB interpretation without
        // changing pixel data. Retain the original attachment as a diagnostic
        // control; leave RGB and already-sRGB/unknown formats untouched.
        if ProcessInfo.processInfo.environment["EXTEND_DISPLAY_NV12_SRGB_INPUT"] != "0",
           CVPixelBufferGetPixelFormatType(image) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
           (CVBufferCopyAttachment(image, kCVImageBufferTransferFunctionKey, nil) as? String) == kCVImageBufferTransferFunction_ITU_R_709_2 as String {
            CVBufferSetAttachment(image, kCVImageBufferTransferFunctionKey,
                                  kCVImageBufferTransferFunction_sRGB, .shouldPropagate)
            if !reportedColor { reportPixelColor(image, stage: "encoder_input") }
        }
        reportedColor = true
        benchmarkMarker?.event("capture_marker", image: image, timestamp: timestamp)
        do { try encoder.encode(image, timestamp: CMSampleBufferGetPresentationTimeStamp(sampleBuffer)) }
        catch { failure(error) }
    }
    func requestKeyframe() { guard lifecycle?.isClosing != true else { return }; queue.sync { encoder.requestKeyframe() } }
    func pauseCapture(_ paused: Bool) { guard lifecycle?.isClosing != true else { return }; queue.sync { encoder.pauseCapture(paused) } }
    func setBitrate(_ bitrate: Int) throws { try checkLifetime(); try queue.sync { try encoder.setBitrate(bitrate) } }
    func configure(width: Int, height: Int, retina: Bool, fps: Int, bitrate: Int) async throws {
        try checkLifetime()
        let scale = retina ? 2 : 1
        guard width >= 640, height >= 480, width * scale <= 4096, height * scale <= 4096,
              (30...120).contains(fps), (1_000_000...80_000_000).contains(bitrate),
              let stream, let previous = profile else { throw DisplayFailure("Invalid display settings.") }
        let resize = previous.width != width || previous.height != height || previous.retina != retina
        let old = queue.sync { encoder }
        let next = resize ? try old.replacement(width: width * scale, height: height * scale, fps: fps, bitrate: bitrate) : old
        queue.sync { configuring = true }
        defer { queue.sync { configuring = false } }
        old.drain()
        try checkLifetime()
        if let virtual, resize || previous.fps != fps {
            let lease = Unmanaged<AnyObject>.fromOpaque(virtual).takeUnretainedValue()
            defer { withExtendedLifetime(lease) {} }
            let address = UInt(bitPattern: virtual)
            let lifetime = lifecycle
            // A size the display already offers only needs a mode switch,
            // which keeps macOS's saved display setup valid.
            let preset = await MainActor.run {
                lifetime?.isClosing != true && ECDisplaySelectMode(UnsafeMutableRawPointer(bitPattern: address), UInt32(width), UInt32(height), retina, UInt32(fps))
            }
            let applied = preset ? true : await MainActor.run {
                lifetime?.isClosing != true && ECDisplayConfigure(UnsafeMutableRawPointer(bitPattern: address), UInt32(width), UInt32(height), retina, UInt32(fps))
            }
            try checkLifetime()
            guard applied else { throw DisplayFailure("macOS could not update the extended display.") }
            var selected = false
            // Mode inventories refresh asynchronously after applySettings.
            // Do not acknowledge until macOS confirms logical size and density.
            for _ in 0..<40 {
                selected = await MainActor.run {
                    lifetime?.isClosing != true && ECDisplaySelectMode(UnsafeMutableRawPointer(bitPattern: address), UInt32(width), UInt32(height), retina, UInt32(fps))
                }
                try checkLifetime()
                if selected { break }
                try await Task.sleep(nanoseconds: 50_000_000)
                try checkLifetime()
            }
            guard selected else { throw DisplayFailure("macOS could not select the requested display resolution and density.") }
        }
        let configuration = SCStreamConfiguration()
        configuration.width = width * scale; configuration.height = height * scale
        if mode == "mirror" { configuration.destinationRect = try mirrorDestination(width: width, height: height, retina: retina) }
        let rgbCapture = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_CAPTURE_BGRA"] == "1"
        configuration.pixelFormat = rgbCapture ? kCVPixelFormatType_32BGRA : kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        configuration.colorSpaceName = CGColorSpace.sRGB
        if !rgbCapture { configuration.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2 }
        configuration.minimumFrameInterval = mode == "extend" && fps == 60 ? .zero : CMTime(value: 1, timescale: Int32(fps))
        configuration.queueDepth = 3; configuration.showsCursor = !separateCursor
        configuration.capturesAudio = false; configuration.scalesToFit = true
        if #available(macOS 14.0, *) { configuration.preservesAspectRatio = true }
        try await stream.updateConfiguration(configuration)
        try checkLifetime()
        try next.setFrameRate(fps); try next.setBitrate(bitrate)
        queue.sync {
            encoder = next
            if resize { benchmarkMarker = BenchmarkMarker(logicalWidth: width, logicalHeight: height) }
            profile = (width, height, retina, fps)
        }
        if resize {
            let canvas = CGSize(width: width, height: height)
            await MainActor.run { [cursor] in cursor.setCanvas(canvas) }
        }
        if resize { next.requestKeyframe() }
    }
    func stream(_ stream: SCStream, didStopWithError error: Error) { failure(error) }
    func stop(record: @escaping @Sendable (CaptureLifecycle.Phase) -> Void) async throws {
        startupDeadline.cancel()
        record(.stoppingCursor)
        await MainActor.run { [cursor] in cursor.stop() }
        record(.stoppingStream)
        var stopError: Error?
        if let stream {
            do { try await stream.stopCapture() } catch { stopError = error }
            self.stream = nil
        }
        if let virtual {
            record(.releasingVirtual)
            let address = UInt(bitPattern: virtual)
            await MainActor.run { ECDisplayDestroy(UnsafeMutableRawPointer(bitPattern: address)) }
            self.virtual = nil
            record(.virtualReleased)
        } else { record(.noVirtual) }
        // Final encoder callbacks no longer need the display or a live output
        // consumer. Release our topology ownership before a possible VT stall.
        record(.drainingEncoder)
        encoder.drain()
        if let stopError { throw stopError }
    }
    deinit { if let virtual { ECDisplayDestroy(virtual) } }
}
