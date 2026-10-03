import Foundation
import CoreGraphics

struct EdgePoint: Equatable { var x: Double; var y: Double }
enum EdgeSide: String { case left, right }

struct EdgeDisplay: Equatable {
    let id: UInt32
    let bounds: CGRect
}

func edgeDisplay(at point: CGPoint, in displays: [EdgeDisplay]) -> EdgeDisplay? {
    displays.first { $0.bounds.contains(point) }
}

func edgeIsExposed(display: EdgeDisplay, side: EdgeSide, y: Double, in displays: [EdgeDisplay]) -> Bool {
    let outside = CGPoint(x: side == .left ? display.bounds.minX - 1 : display.bounds.maxX,
                          y: display.bounds.minY + y)
    return !displays.contains { $0.id != display.id && $0.bounds.contains(outside) }
}
enum EdgeMotion: Equatable {
    case local
    case remote(EdgePoint)
    case returned(EdgePoint)
}

/// Coordinates are logical display points, not pixel resolutions. Remote top is
/// offsetY points below local top. Negative offsets place the remote screen above.
struct EdgeLayout {
    let localWidth: Double
    let localHeight: Double
    let remoteWidth: Double
    let remoteHeight: Double
    private(set) var side: EdgeSide
    private var pendingSide: EdgeSide?
    let offsetY: Double
    private(set) var remotePosition: EdgePoint?
    private var armed = true

    init(localWidth: Double, localHeight: Double, remoteWidth: Double,
         remoteHeight: Double, side: EdgeSide, offsetY: Double) throws {
        let sizes = [localWidth, localHeight, remoteWidth, remoteHeight]
        guard sizes.allSatisfy({ $0.isFinite && $0 >= 16 && $0 <= 32768 }),
              offsetY.isFinite,
              max(0, offsetY) < min(localHeight, offsetY + remoteHeight) else {
            throw NSError(domain: "ExtendComputerLayout", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "Displays need valid sizes and an overlapping vertical edge."])
        }
        self.localWidth = localWidth; self.localHeight = localHeight
        self.remoteWidth = remoteWidth; self.remoteHeight = remoteHeight
        self.side = side; self.offsetY = offsetY
    }

    // Preserve the return route while remote. Repeated edits replace the pending side.
    mutating func setSide(_ side: EdgeSide) {
        if remotePosition != nil { pendingSide = side }
        else { self.side = side; pendingSide = nil }
    }

    private mutating func applyPendingSide() {
        if let side = pendingSide { self.side = side; pendingSide = nil }
    }

    mutating func motion(local: EdgePoint, dx: Double, dy: Double, exposed: Bool, allowReturn: Bool = true) -> EdgeMotion {
        guard [local.x, local.y, dx, dy].allSatisfy({ $0.isFinite }) else { return .local }
        if var point = remotePosition {
            point.x += dx
            point.y = min(remoteHeight - 1, max(0, point.y + dy))
            let localY = point.y + offsetY
            let returning = side == .left ? point.x >= remoteWidth : point.x < 0
            if allowReturn && returning && localY >= 0 && localY < localHeight {
                remotePosition = nil
                armed = false
                let returned = EdgePoint(x: side == .left ? 4 : localWidth - 5, y: localY)
                applyPendingSide()
                return .returned(returned)
            }
            point.x = min(remoteWidth - 1, max(0, point.x))
            remotePosition = point
            return .remote(normalized(point))
        }
        let away = side == .left ? local.x > 6 : local.x < localWidth - 7
        if !armed {
            if away { armed = true }
            return .local
        }
        let outward = side == .left ? local.x <= 1 && dx < 0 : local.x >= localWidth - 2 && dx > 0
        guard exposed, outward, local.y >= 0, local.y < localHeight,
              local.y >= offsetY, local.y < offsetY + remoteHeight else { return .local }
        let point = EdgePoint(x: side == .left ? remoteWidth - 1 : 0, y: local.y - offsetY)
        remotePosition = point
        return .remote(normalized(point))
    }

    mutating func cancel() -> EdgePoint? {
        guard let point = remotePosition else { return nil }
        remotePosition = nil
        armed = false
        return EdgePoint(x: side == .left ? 4 : localWidth - 5,
                         y: min(localHeight - 1, max(0, point.y + offsetY)))
    }

    private func normalized(_ point: EdgePoint) -> EdgePoint {
        EdgePoint(x: point.x / (remoteWidth - 1), y: point.y / (remoteHeight - 1))
    }
}
