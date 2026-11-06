import UIKit
import CoreVideo

/// Remote display surface: black letterbox, aspect-fitted Metal video layer and
/// the separate-cursor overlay clipped to the video rectangle. Mirrors
/// Android's ExtendDisplayView layout (SurfaceView at the fitted rect, cursor
/// drawn over it). Configure/cursor/layout calls are main-thread; frame
/// delivery goes through `presenter`, which is thread-safe.
final class DisplayRenderer: UIView {
    let presenter: MetalFramePresenter
    let cursor = DisplayCursorOverlay()
    /// Decoded raster (pixels) and logical desktop size (points) of the stream.
    private(set) var raster = CGSize(width: 1280, height: 800)
    private(set) var logical = CGSize(width: 1280, height: 800)
    private var expiry: Timer?

    /// Throws when Metal is unavailable.
    static func make() throws -> DisplayRenderer { DisplayRenderer(presenter: try MetalFramePresenter()) }

    init(presenter: MetalFramePresenter) {
        self.presenter = presenter
        super.init(frame: .zero)
        backgroundColor = .black
        isOpaque = true
        layer.addSublayer(presenter.layer)
        layer.addSublayer(cursor.container)
        isAccessibilityElement = true
        accessibilityLabel = "Remote display"
    }
    required init?(coder: NSCoder) { fatalError("Not supported") }

    /// Applies a negotiated stream size. Frames of other revisions are dropped.
    func configure(_ config: DisplayStreamConfig, revision: Int64) {
        presenter.configure(revision: revision, rasterWidth: config.rasterWidth, rasterHeight: config.rasterHeight)
        updateLayout(for: config)
    }

    /// Main thread: letterbox geometry only (DisplayReceiver configures the
    /// presenter itself on its frame thread so no new-revision frame is dropped).
    func updateLayout(for config: DisplayStreamConfig) {
        raster = CGSize(width: config.rasterWidth, height: config.rasterHeight)
        logical = CGSize(width: config.width, height: config.height)
        setNeedsLayout()
    }

    /// Stops presenting and hides the cursor (session ended or replaced).
    func reset() {
        presenter.reset()
        cursor.reset()
    }

    /// Aspect-fitted video rectangle in view coordinates (pixel-aligned).
    var videoRect: CGRect {
        guard let fitted = DisplayFit.fitted(source: raster, in: bounds) else { return .zero }
        let scale = window?.screen.scale ?? traitCollection.displayScale
        func align(_ value: CGFloat) -> CGFloat { (value * scale).rounded() / scale }
        return CGRect(x: align(fitted.minX), y: align(fitted.minY),
                      width: align(fitted.width), height: align(fitted.height))
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let rect = videoRect
        CATransaction.begin(); CATransaction.setDisableActions(true)
        presenter.layer.frame = rect
        presenter.layer.contentsScale = window?.screen.scale ?? traitCollection.displayScale
        cursor.container.frame = rect
        CATransaction.commit()
        cursor.layout(source: logical)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        expiry?.invalidate(); expiry = nil
        guard window != nil else { return }
        // Hide a cursor whose position stream went quiet (2 s limit), as Android's expiry tick.
        expiry = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.cursor.layout(source: self.logical)
        }
    }

    /// Main thread: applies one packet from `next_display_cursor`.
    func applyCursor(_ packet: Data) throws {
        try cursor.apply(packet, source: logical)
    }

    /// Geometry for input mapping, from this view's current layout.
    var touchMapping: DisplayTouchMapping {
        DisplayTouchMapping(video: videoRect, raster: raster, logical: logical)
    }

    deinit { expiry?.invalidate() }
}

/// CALayer cursor over the video, positioned from the shared CursorState
/// (shape bitmap + hotspot in logical points, normalized top-left position).
final class DisplayCursorOverlay {
    /// Same frame as the video layer; clips the cursor like Android's clipRect.
    let container = CALayer()
    let layer = CALayer()
    private var state = CursorState()

    init() {
        container.masksToBounds = true
        container.actions = ["position": NSNull(), "bounds": NSNull()]
        layer.isHidden = true
        layer.magnificationFilter = .linear
        layer.actions = ["position": NSNull(), "bounds": NSNull(), "contents": NSNull(), "hidden": NSNull()]
        container.addSublayer(layer)
    }

    func apply(_ bytes: Data, source: CGSize) throws {
        let packet: CursorPacket
        do { packet = try CursorPacket.parse(bytes) }
        catch { layer.isHidden = true; throw error }
        if let image = state.apply(packet) { layer.contents = image }
        layout(source: source)
    }

    func layout(source: CGSize) {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        guard let frame = state.frame(viewport: container.bounds, source: source, yUp: false) else {
            layer.isHidden = true; return
        }
        layer.frame = frame
        layer.isHidden = false
    }

    func reset() {
        state = CursorState()
        layer.contents = nil
        layer.isHidden = true
    }
}
