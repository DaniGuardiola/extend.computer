import Foundation

enum PermissionGuidePointerSide { case none, left, right }

struct PermissionGuidePlacement {
    let frame: CGRect
    let side: PermissionGuidePointerSide
    let pointerY: CGFloat

    private static let gap: CGFloat = 14

    static func besideWindow(_ window: CGRect, pointingAt target: CGRect, size: CGSize, visible: CGRect) -> PermissionGuidePlacement? {
        // Let 8pt of the 12pt arrow overlap the edge; the body stays 4pt outside.
        // Every step uses the same horizontal anchor, leaving nearby controls clear.
        let inset = gap + 8
        let row = CGRect(x: window.minX + inset, y: target.minY, width: window.width - 2 * inset, height: target.height)
        return nextTo(row, size: size, visible: visible)
    }

    static func nextTo(_ target: CGRect, size: CGSize, visible: CGRect) -> PermissionGuidePlacement? {
        guard visible.contains(target), size.width <= visible.width, size.height <= visible.height else { return nil }
        let x: CGFloat
        let side: PermissionGuidePointerSide
        if target.maxX + gap + size.width <= visible.maxX {
            x = target.maxX + gap; side = .left
        } else if target.minX - gap - size.width >= visible.minX {
            x = target.minX - gap - size.width; side = .right
        } else { return nil }
        let y = max(visible.minY, min(target.midY - size.height / 2, visible.maxY - size.height))
        let pointerY = target.midY - y
        guard pointerY >= 25, pointerY <= size.height - 25 else { return nil }
        return .init(frame: CGRect(origin: CGPoint(x: x, y: y), size: size), side: side, pointerY: pointerY)
    }
}

struct PermissionGuidePresentation {
    private(set) var started: TimeInterval
    private(set) var stableSince: TimeInterval
    private(set) var frame: CGRect?

    init(now: TimeInterval) { started = now; stableSince = now }

    mutating func observe(_ frame: CGRect, now: TimeInterval) {
        if self.frame != frame { self.frame = frame; stableSince = now }
    }

    func canReveal(settingsForeground: Bool, now: TimeInterval) -> Bool {
        settingsForeground && frame != nil && now - started >= 0.2 && now - stableSince >= 0.12
    }
}
