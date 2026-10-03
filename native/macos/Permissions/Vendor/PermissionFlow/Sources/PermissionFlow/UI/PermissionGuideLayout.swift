import Foundation

enum PermissionGuidePointerSide { case none, left, right }

struct PermissionGuidePlacement {
    let frame: CGRect
    let side: PermissionGuidePointerSide
    let pointerY: CGFloat

    private static let gap: CGFloat = 14

    static func besideWindow(_ window: CGRect, pointingAt target: CGRect?, size: CGSize, visible: CGRect) -> PermissionGuidePlacement? {
        // Overlap the body by 4pt and the 12pt arrow by 16pt.
        // Every step uses the same horizontal anchor, leaving nearby controls clear.
        guard window.intersects(visible), size.width <= visible.width, size.height <= visible.height else { return nil }
        let x: CGFloat
        let side: PermissionGuidePointerSide
        if window.maxX - 16 + size.width <= visible.maxX {
            x = window.maxX - 16; side = .left
        } else if window.minX + 16 - size.width >= visible.minX {
            x = window.minX + 16 - size.width; side = .right
        } else { return nil }
        let anchorY = target?.midY ?? window.intersection(visible).midY
        let y = max(visible.minY, min(anchorY - size.height / 2, visible.maxY - size.height))
        // Keep the arrow even before Accessibility allows a precise row lookup.
        // At screen edges, keep its tip clear of the bubble's rounded corners.
        let pointerY = max(25, min(anchorY - y, size.height - 25))
        return .init(frame: CGRect(x: x, y: y, width: size.width, height: size.height), side: side, pointerY: pointerY)
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
