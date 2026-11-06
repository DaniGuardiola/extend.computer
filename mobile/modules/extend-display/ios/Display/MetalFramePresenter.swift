import Foundation
import CoreImage
import CoreVideo
import Metal
import QuartzCore

/// Renders decoded frames into a CAMetalLayer as soon as they arrive, latest
/// frame wins. Thread-safe: `present` may be called from the decoder thread.
///
/// Why Metal and not AVSampleBufferDisplayLayer: both reach the screen on the
/// next compositor vsync after enqueue, so latency is equal, but only a
/// CAMetalDrawable reports the actual presentation time per frame
/// (addPresentedHandler). The engine needs that for displayRendered (device
/// clock + render progress gates its decoder budget) and for the first-frame
/// "streaming" signal. AVSampleBufferDisplayLayer also fails its renderer
/// after backgrounding and needs flush/recovery. This is the same pipeline as
/// the macOS viewer (native/macos/display/Viewer.swift): CIContext converts the
/// BT.709/sRGB-tagged YCbCr buffer into a BGRA drawable on the GPU.
final class MetalFramePresenter {
    struct Presented {
        /// Revision the frame was decoded for.
        let revision: Int64
        /// Source timestamp from the frame packet (ns).
        let timestamp: UInt64
        /// Compositor presentation time on the mach host clock (ns), same clock
        /// as DispatchTime.now().uptimeNanoseconds.
        let presentedNs: UInt64
        /// When the presented handler ran (ns, same clock).
        let callbackNs: UInt64
    }

    let layer = CAMetalLayer()
    /// Called once per newly presented source frame, on a Metal/CA thread.
    var presented: ((Presented) -> Void)? {
        get { lock.lock(); defer { lock.unlock() }; return presentedHandler }
        set { lock.lock(); presentedHandler = newValue; lock.unlock() }
    }

    private let device: MTLDevice
    private let commands: MTLCommandQueue
    private let context: CIContext
    private let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    private let queue = DispatchQueue(label: "computer.extend.display.render", qos: .userInteractive)
    private let lock = NSLock()
    private var presentedHandler: ((Presented) -> Void)?
    private var pending: (CVPixelBuffer, UInt64, Int64)?
    private var scheduled = false
    private var acceptedRevision: Int64 = -1
    private var acceptedRaster = (width: 0, height: 0)
    private var lastPresented: (revision: Int64, timestamp: UInt64)?

    init() throws {
        guard let device = MTLCreateSystemDefaultDevice(), let commands = device.makeCommandQueue() else {
            throw DisplayFailure("Metal rendering unavailable.")
        }
        self.device = device; self.commands = commands
        context = CIContext(mtlDevice: device, options: [.cacheIntermediates: false, .workingColorSpace: colorSpace])
        layer.device = device
        layer.pixelFormat = .bgra8Unorm
        layer.framebufferOnly = false
        layer.isOpaque = true
        layer.backgroundColor = CGColor(gray: 0, alpha: 1)
        layer.presentsWithTransaction = false
        // The encoder tags sRGB transfer / 709 primaries; tell Core Animation
        // so wide-gamut panels get the right conversion.
        layer.colorspace = colorSpace
        layer.actions = ["bounds": NSNull(), "position": NSNull(), "contents": NSNull()]
    }

    /// Accept frames of this revision and decoded raster only. Older in-flight
    /// frames (previous size or revision) are dropped, like Android's DecodedFrameGate.
    func configure(revision: Int64, rasterWidth: Int, rasterHeight: Int) {
        lock.lock()
        acceptedRevision = revision; acceptedRaster = (rasterWidth, rasterHeight)
        pending = nil; lastPresented = nil
        lock.unlock()
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer.drawableSize = CGSize(width: rasterWidth, height: rasterHeight)
        CATransaction.commit()
    }

    /// Stops accepting frames until the next configure.
    func reset() {
        lock.lock(); acceptedRevision = -1; pending = nil; lastPresented = nil; lock.unlock()
    }

    /// Queues a decoded frame for immediate rendering. Returns false when the
    /// frame is stale (revision or raster mismatch) and was dropped.
    @discardableResult
    func present(_ image: CVPixelBuffer, timestamp: UInt64, revision: Int64) -> Bool {
        lock.lock()
        guard revision == acceptedRevision,
              CVPixelBufferGetWidth(image) == acceptedRaster.width,
              CVPixelBufferGetHeight(image) == acceptedRaster.height else { lock.unlock(); return false }
        pending = (image, timestamp, revision)
        if scheduled { lock.unlock(); return true }
        scheduled = true; lock.unlock()
        queue.async { [weak self] in self?.drain() }
        return true
    }

    private func drain() {
        lock.lock(); let next = pending; pending = nil; scheduled = false; lock.unlock()
        guard let (image, timestamp, revision) = next else { return }
        render(image, timestamp: timestamp, revision: revision)
    }

    private func render(_ image: CVPixelBuffer, timestamp: UInt64, revision: Int64) {
        // nextDrawable blocks up to a second when all drawables are in flight
        // or returns nil while the layer is offscreen; drop the frame then.
        guard let drawable = layer.nextDrawable(), let buffer = commands.makeCommandBuffer() else { return }
        let source = CIImage(cvPixelBuffer: image, options: [.colorSpace: colorSpace])
        let size = CGSize(width: drawable.texture.width, height: drawable.texture.height)
        let bounds = CGRect(origin: .zero, size: size)
        var output = source
        if source.extent.size != size {
            // Only during a drawable resize race: aspect-fit instead of stretching.
            let scale = min(size.width / source.extent.width, size.height / source.extent.height)
            output = source.transformed(by: CGAffineTransform(translationX: (size.width - source.extent.width * scale) / 2,
                y: (size.height - source.extent.height * scale) / 2).scaledBy(x: scale, y: scale))
                .composited(over: CIImage(color: .black).cropped(to: bounds))
        }
        context.render(output, to: drawable.texture, commandBuffer: buffer, bounds: bounds, colorSpace: colorSpace)
        #if targetEnvironment(simulator)
        // The simulator has no presented handler; GPU completion stands in.
        buffer.addCompletedHandler { [weak self] command in
            guard command.status == .completed else { return }
            let now = DispatchTime.now().uptimeNanoseconds
            self?.report(revision: revision, timestamp: timestamp, presentedNs: now, callbackNs: now)
        }
        #else
        drawable.addPresentedHandler { [weak self] drawable in
            guard let self, drawable.presentedTime.isFinite, drawable.presentedTime > 0 else { return }
            self.report(revision: revision, timestamp: timestamp, presentedNs: UInt64(drawable.presentedTime * 1e9),
                        callbackNs: DispatchTime.now().uptimeNanoseconds)
        }
        #endif
        buffer.present(drawable)
        buffer.commit()
    }

    private func report(revision: Int64, timestamp: UInt64, presentedNs: UInt64, callbackNs: UInt64) {
        lock.lock()
        let fresh = revision == acceptedRevision
            && (lastPresented.map { $0.revision != revision || timestamp > $0.timestamp } ?? true)
        if fresh { lastPresented = (revision, timestamp) }
        let handler = presentedHandler
        lock.unlock()
        guard fresh else { return }
        handler?(Presented(revision: revision, timestamp: timestamp, presentedNs: presentedNs, callbackNs: callbackNs))
    }
}
