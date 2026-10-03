#if os(macOS)
import AppKit
import ApplicationServices

// Decorative targeting uses existing trust only. Permission readiness remains
// the host's responsibility, even when a matching switch is visible.
enum PermissionGuideTarget {
    static func find(identifier: String, paneTitles: Set<String>) -> CGRect? {
        guard AXIsProcessTrusted(),
              let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.systempreferences").first else { return nil }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.03)
        let windows = attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? []
        var queue = windows.filter { paneTitles.contains(attribute($0, kAXTitleAttribute) as? String ?? "") }
        var index = 0
        let deadline = ProcessInfo.processInfo.systemUptime + 0.75
        while index < queue.count && index < 600 && ProcessInfo.processInfo.systemUptime < deadline {
            let element = queue[index]; index += 1
            AXUIElementSetMessagingTimeout(element, 0.03)
            let role = attribute(element, kAXRoleAttribute) as? String
            if (role == kAXCheckBoxRole as String || role == "AXSwitch"),
               attribute(element, kAXIdentifierAttribute) as? String == identifier {
                return frame(element)
            }
            let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
            guard queue.count + children.count <= 600 else { return nil }
            queue.append(contentsOf: children)
        }
        return nil
    }

    private static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
    }

    private static func frame(_ element: AXUIElement) -> CGRect? {
        guard let position = attribute(element, kAXPositionAttribute), CFGetTypeID(position) == AXValueGetTypeID(),
              let sizeValue = attribute(element, kAXSizeAttribute), CFGetTypeID(sizeValue) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var size = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &point),
              AXValueGetValue(sizeValue as! AXValue, .cgSize, &size), size.width > 0, size.height > 0 else { return nil }
        return CGRect(origin: point, size: size)
    }

    @MainActor static func appKitFrame(_ global: CGRect) -> CGRect? {
        let screens = NSScreen.screens.compactMap { screen -> (NSScreen, CGRect)? in
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return nil }
            return (screen, CGDisplayBounds(CGDirectDisplayID(number.uint32Value)))
        }
        guard let match = screens.filter({ $0.1.intersects(global) }).max(by: {
            $0.1.intersection(global).width * $0.1.intersection(global).height <
            $1.1.intersection(global).width * $1.1.intersection(global).height
        }) else { return nil }
        return CGRect(x: match.0.frame.minX + global.minX - match.1.minX,
                      y: match.0.frame.maxY - (global.minY - match.1.minY) - global.height,
                      width: global.width, height: global.height)
    }
}
#endif
