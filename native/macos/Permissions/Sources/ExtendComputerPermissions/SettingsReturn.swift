import AppKit
import ApplicationServices
import CoreGraphics
import PermissionFlow

@MainActor
final class SettingsReturn {
    private struct Window {
        let key: SettingsWindowKey
        let bounds: CGRect
    }
    private var ownership: SettingsReturnGuard
    private let paneTitles: Set<String>
    private let appToggleID: String

    init(pane: PermissionFlowPane) {
        ownership = SettingsReturnGuard(existing: Self.windows()?.map(\.key))
        paneTitles = [pane.localizedTitle(localeIdentifier: nil)]
        appToggleID = Bundle.main.bundleURL.lastPathComponent + "_Toggle"
    }

    init(backgroundActivity: Void) {
        ownership = SettingsReturnGuard(existing: Self.windows()?.map(\.key))
        paneTitles = ["Login Items", "Login Items & Extensions"]
        appToggleID = "background-switch-" + Bundle.main.bundleURL.lastPathComponent
    }

    func observe() {
        ownership.observe(Self.windows()?.map(\.key))
    }

    func finish() {
        // Approval returns to our app even when Settings was already open or
        // its window cannot safely be closed. Activate after checking ownership.
        defer { NSApp.activate(ignoringOtherApps: true) }
        observe()
        guard ownership.eligible, let owned = ownership.owned,
              AXIsProcessTrusted(), let windows = Self.windows(),
              windows.count == 1, let current = windows.first, current.key == owned
        else { return }
        let app = AXUIElementCreateApplication(owned.pid)
        AXUIElementSetMessagingTimeout(app, 0.3)
        guard let window = Self.element(app, kAXFocusedWindowAttribute),
              Self.bounds(window).map({ Self.sameBounds($0, current.bounds) }) == true,
              let title = Self.attribute(window, kAXTitleAttribute) as? String,
              ownership.mayClose(current: windows.map(\.key), permissionPage: paneTitles.contains(title),
                                 appEnabled: hasEnabledAppToggle(window),
                                 foregroundPID: NSWorkspace.shared.frontmostApplication?.processIdentifier),
              let close = Self.element(window, kAXCloseButtonAttribute),
              (Self.attribute(close, kAXEnabledAttribute) as? Bool) == true
        else { return }
        // Close only the exact new window; never terminate System Settings.
        AXUIElementPerformAction(close, kAXPressAction as CFString)
    }

    private func hasEnabledAppToggle(_ window: AXUIElement) -> Bool {
        var queue = [window]
        var index = 0
        var enabled = false
        while index < queue.count && index < 600 {
            let node = queue[index]
            index += 1
            if (Self.attribute(node, kAXRoleAttribute) as? String) == kAXSheetRole as String ||
               (Self.attribute(node, kAXModalAttribute) as? Bool) == true { return false }
            if (Self.attribute(node, kAXIdentifierAttribute) as? String) == appToggleID,
               (Self.attribute(node, kAXValueAttribute) as? NSNumber)?.boolValue == true {
                enabled = true
            }
            let children = Self.attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? []
            guard queue.count + children.count <= 600 else { return false }
            queue.append(contentsOf: children)
        }
        return enabled
    }

    private static func windows() -> [Window]? {
        let pids = Set(NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.systempreferences")
            .map(\.processIdentifier))
        guard !pids.isEmpty else { return [] }
        guard let entries = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID)
                as? [[String: Any]] else { return nil }
        return entries.compactMap { entry in
            guard let pid = entry[kCGWindowOwnerPID as String] as? Int32, pids.contains(pid),
                  (entry[kCGWindowLayer as String] as? Int) == 0,
                  let number = entry[kCGWindowNumber as String] as? UInt32,
                  let dictionary = entry[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: dictionary),
                  bounds.width > 320, bounds.height > 240 else { return nil }
            return Window(key: SettingsWindowKey(pid: pid, number: number), bounds: bounds)
        }
    }

    private static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
        return value
    }

    private static func element(_ parent: AXUIElement, _ name: String) -> AXUIElement? {
        guard let value = attribute(parent, name), CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return (value as! AXUIElement)
    }

    private static func bounds(_ window: AXUIElement) -> CGRect? {
        guard let position = attribute(window, kAXPositionAttribute),
              let size = attribute(window, kAXSizeAttribute),
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID()
        else { return nil }
        let pointValue = position as! AXValue
        let sizeValue = size as! AXValue
        guard AXValueGetType(pointValue) == .cgPoint, AXValueGetType(sizeValue) == .cgSize else { return nil }
        var point = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(pointValue, .cgPoint, &point), AXValueGetValue(sizeValue, .cgSize, &dimensions)
        else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    private static func sameBounds(_ lhs: CGRect, _ rhs: CGRect) -> Bool {
        abs(lhs.minX - rhs.minX) < 2 && abs(lhs.minY - rhs.minY) < 2 &&
        abs(lhs.width - rhs.width) < 2 && abs(lhs.height - rhs.height) < 2
    }
}
