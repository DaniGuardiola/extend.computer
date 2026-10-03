import AppKit
import SwiftUI

@main struct GuideTests {
    @MainActor static func main() {
        let screen = CGRect(x: 0, y: 0, width: 1600, height: 900)
        let size = CGSize(width: 330, height: 180)
        for window in [CGRect(x: 100, y: 100, width: 740, height: 625),
                       CGRect(x: 800, y: 100, width: 740, height: 625)] {
            var anchor: CGFloat?
            for y: CGFloat in [250, 400, 600] {
                let toggle = CGRect(x: window.maxX - 80, y: y, width: 44, height: 20)
                let placement = PermissionGuidePlacement.besideWindow(window, pointingAt: toggle, size: size, visible: screen)!
                let body = placement.frame.insetBy(dx: 12, dy: 0)
                assert(!body.intersects(window))
                assert(!placement.frame.intersects(toggle))
                assert(screen.contains(placement.frame))
                assert(abs(placement.frame.minY + placement.pointerY - toggle.midY) < 1)
                if let anchor { assert(placement.frame.minX == anchor) }
                anchor = placement.frame.minX
            }
        }
        let tight = CGRect(x: 0, y: 0, width: 740, height: 700)
        assert(PermissionGuidePlacement.nextTo(tight, size: size, visible: tight) == nil)
        let secondary = CGRect(x: -1600, y: -200, width: 1600, height: 900)
        let secondaryWindow = CGRect(x: -1500, y: -100, width: 740, height: 625)
        assert(secondary.contains(PermissionGuidePlacement.nextTo(secondaryWindow, size: size, visible: secondary)!.frame))

        var presentation = PermissionGuidePresentation(now: 10)
        assert(!presentation.canReveal(settingsForeground: true, now: 11))
        presentation.observe(CGRect(x: 100, y: 100, width: 740, height: 600), now: 11)
        assert(!presentation.canReveal(settingsForeground: true, now: 11.05))
        assert(presentation.canReveal(settingsForeground: true, now: 11.2))
        assert(!presentation.canReveal(settingsForeground: false, now: 12))
        presentation.observe(CGRect(x: 200, y: 100, width: 740, height: 600), now: 12)
        assert(!presentation.canReveal(settingsForeground: true, now: 12.05))
        assert(presentation.canReveal(settingsForeground: true, now: 12.2))
        print("PASS: guide placement, screen edges, secondary displays, and settled presentation")

        _ = NSApplication.shared
        let controller = PermissionFlowController(configuration: .init(requiredAppURLs: [URL(fileURLWithPath: "/Applications/extend.computer.app")]))
        controller.guide(title: "Enable extend.computer under App Background Activity",
                         body: "This helps keep your keyboard and mouse responsive over Wi-Fi. AirDrop pauses while you’re connected and resumes when you disconnect.",
                         openSettings: {})
        assert(controller.isActive)
        assert(!NSApp.windows.contains { $0 is FloatingDropPanel && $0.isVisible })
        let replacement = PermissionFlowController()
        replacement.guide(title: "Replacement", body: "Replacement guidance", openSettings: {})
        assert(!controller.isActive)
        assert(replacement.isActive)
        replacement.closePanel()
        assert(!replacement.isActive)
        print("PASS: hidden guide stays active; replacement and dismissal end its session")

        if let preview = CommandLine.arguments.dropFirst().first {
            func render<V: View>(_ content: V, to path: String) {
                let measured = NSHostingController(rootView: content).sizeThatFits(in: CGSize(width: 330, height: 4096)).height
                assert(measured > 40 && measured < 400)
                let view = NSHostingView(rootView: content)
                if #available(macOS 13.3, *) { view.sizingOptions = [] }
                view.frame = CGRect(x: 0, y: 0, width: 330, height: measured)
                let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
                window.contentView = view
                window.display()
                RunLoop.current.run(until: Date().addingTimeInterval(0.2))
                view.layoutSubtreeIfNeeded()
                let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
                view.cacheDisplay(in: view.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
                assert(view.frame.height == measured)
                print("PASS: guide renders at 330 × \(measured) without fixed-height clipping")
            }
            controller.pointerSide = .left
            controller.pointerY = 80
            render(PermissionFlowPanelView(controller: controller), to: preview)
            controller.pointerSide = .right
            render(PermissionFlowPanelView(controller: controller).environment(\.colorScheme, .dark), to: preview + ".dark.png")
            controller.guidanceTitle = nil
            controller.guidanceBody = nil
            render(PermissionFlowPanelView(controller: controller), to: preview + ".drag.png")

            let panel = FloatingDropPanel(controller: controller)
            panel.snap(to: CGRect(x: 100, y: 100, width: 740, height: 625))
            assert(panel.frame.height > 96 && panel.frame.height < 400)
            assert(!panel.canBecomeKey && !panel.canBecomeMain)
            panel.setDraggingPassthrough(true)
            assert(panel.ignoresMouseEvents)
            panel.setDraggingPassthrough(false)
            assert(!panel.ignoresMouseEvents)
            panel.close()
            print("PASS: native panel sizing and drag passthrough")
        }
    }
}
