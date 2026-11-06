import Foundation
import CoreGraphics

/// View point → stream coordinates for input. `video` is the aspect-fitted
/// video rectangle in view coordinates (DisplayRenderer.videoRect).
struct DisplayTouchMapping: Equatable {
    let video: CGRect
    let raster: CGSize
    let logical: CGSize

    /// Normalized 0...1 point (top-left origin) inside the video, or nil in the letterbox.
    func normalized(_ point: CGPoint) -> CGPoint? {
        guard video.width > 0, video.height > 0 else { return nil }
        let x = (point.x - video.minX) / video.width, y = (point.y - video.minY) / video.height
        guard x.isFinite, y.isFinite, (0...1).contains(x), (0...1).contains(y) else { return nil }
        return CGPoint(x: x, y: y)
    }
    /// Decoded-raster pixel under the point (e.g. 2560x1600 for a Retina 1280x800 desktop).
    func sourcePixel(_ point: CGPoint) -> CGPoint? {
        normalized(point).map { CGPoint(x: min($0.x * raster.width, raster.width - 1).rounded(.down),
                                        y: min($0.y * raster.height, raster.height - 1).rounded(.down)) }
    }
    /// Desktop logical point under the touch.
    func logicalPoint(_ point: CGPoint) -> CGPoint? {
        normalized(point).map { CGPoint(x: $0.x * logical.width, y: $0.y * logical.height) }
    }
}

// Engine input-grant geometry (content rect + source_size) is handled by
// Input/DisplayInputGrant.swift + TouchGeometry; feed it `videoRect`.
