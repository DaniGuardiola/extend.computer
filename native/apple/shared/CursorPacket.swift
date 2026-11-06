import Foundation
import CoreGraphics
import ImageIO

// Receiver side of the separate-cursor stream, shared by the macOS viewer and
// the iOS receiver. Packets are the JSON objects the source sends (and the
// Android/iOS engines return from nextDisplayCursor):
//   shape:    {"revision", "png" (base64), "width", "height", "hotspot_x", "hotspot_y"}
//             sizes and hotspot in source logical points (<= 512)
//   position: {"revision", "sequence", "x", "y" (normalized 0...1, top-left origin), "visible"}

struct CursorPosition: Decodable {
    let revision: UInt64, sequence: UInt64
    let x: Double, y: Double, visible: Bool
}
struct CursorShape: Decodable {
    let revision: UInt64, png: String
    let width: Double, height: Double, hotspot_x: Double, hotspot_y: Double
}

enum CursorPacket {
    case shape(CursorShape, CGImage)
    case position(CursorPosition)

    static let maxBytes = 128 * 1024
    static let maxPNG = 90 * 1024

    static func parse(_ bytes: Data) throws -> CursorPacket {
        guard bytes.count <= maxBytes else { throw DisplayFailure("Cursor packet too large.") }
        let object = try JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        if object?["png"] != nil {
            let next = try JSONDecoder().decode(CursorShape.self, from: bytes)
            guard next.width.isFinite, next.height.isFinite, next.width > 0, next.height > 0,
                next.width <= 512, next.height <= 512,
                next.hotspot_x.isFinite, next.hotspot_y.isFinite,
                (0...next.width).contains(next.hotspot_x), (0...next.height).contains(next.hotspot_y),
                let data = Data(base64Encoded: next.png), data.count <= maxPNG,
                let imageSource = CGImageSourceCreateWithData(data as CFData, nil),
                let properties = CGImageSourceCopyPropertiesAtIndex(imageSource, 0, nil) as? [CFString: Any],
                let width = properties[kCGImagePropertyPixelWidth] as? Int,
                let height = properties[kCGImagePropertyPixelHeight] as? Int,
                width > 0, height > 0, width <= 512, height <= 512,
                let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
                throw DisplayFailure("Invalid cursor image.")
            }
            return .shape(next, image)
        }
        let next = try JSONDecoder().decode(CursorPosition.self, from: bytes)
        guard next.x.isFinite, next.y.isFinite, (0...1).contains(next.x), (0...1).contains(next.y) else {
            throw DisplayFailure("Invalid cursor position.")
        }
        return .position(next)
    }
}

/// Latest-wins cursor state: newer-or-equal shape revisions, strictly newer
/// position sequences, and a two-second staleness limit (as on Android).
struct CursorState {
    private(set) var shape: CursorShape?
    private(set) var image: CGImage?
    private(set) var position: CursorPosition?
    private var lastSequence: UInt64 = 0
    private var lastPosition = Date.distantPast

    /// Applies a packet; returns the new image when the shape was replaced.
    @discardableResult
    mutating func apply(_ packet: CursorPacket, now: Date = Date()) -> CGImage? {
        switch packet {
        case let .shape(next, nextImage):
            guard next.revision >= (shape?.revision ?? 0) else { return nil }
            shape = next; image = nextImage; return nextImage
        case let .position(next):
            if next.sequence > lastSequence { position = next; lastSequence = next.sequence; lastPosition = now }
            return nil
        }
    }

    /// Cursor layer frame inside `viewport`, where the `source` logical canvas
    /// is aspect-fitted (letterboxed). `yUp` selects AppKit-style coordinates;
    /// UIKit layers use top-left origin. Nil when the cursor must be hidden.
    func frame(viewport: CGRect, source: CGSize, yUp: Bool, now: Date = Date()) -> CGRect? {
        guard let shape, let position, shape.revision == position.revision, position.visible,
              now.timeIntervalSince(lastPosition) < 2,
              let fitted = DisplayFit.fitted(source: source, in: viewport) else { return nil }
        let scale = fitted.width / source.width
        let x = fitted.minX + position.x * fitted.width - shape.hotspot_x * scale
        let y = yUp ? fitted.maxY - position.y * fitted.height - (shape.height - shape.hotspot_y) * scale
                    : fitted.minY + position.y * fitted.height - shape.hotspot_y * scale
        return CGRect(x: x, y: y, width: shape.width * scale, height: shape.height * scale)
    }
}

enum DisplayFit {
    /// Aspect-fit rectangle of `source` centered in `viewport`.
    static func fitted(source: CGSize, in viewport: CGRect) -> CGRect? {
        guard source.width > 0, source.height > 0, source.width.isFinite, source.height.isFinite,
              viewport.width > 0, viewport.height > 0 else { return nil }
        let scale = min(viewport.width / source.width, viewport.height / source.height)
        return CGRect(x: viewport.midX - source.width * scale / 2,
                      y: viewport.midY - source.height * scale / 2,
                      width: source.width * scale, height: source.height * scale)
    }
}
