#if os(macOS)
import AppKit
import SwiftUI

@available(macOS 13.0, *)
@MainActor
final class FloatingDropPanel: NSPanel {
    private weak var panelController: PermissionFlowController?
    private let hostingView: NSHostingView<AnyView>
    private let sizingController: NSHostingController<AnyView>
    private var localeIdentifier: String?

    init(controller: PermissionFlowController) {
        panelController = controller
        localeIdentifier = controller.localeIdentifier
        let view = Self.makePanelView(controller: controller, localeIdentifier: controller.localeIdentifier)
        hostingView = NSHostingView(rootView: view)
        sizingController = NSHostingController(rootView: view)
        super.init(contentRect: CGRect(x: 0, y: 0, width: 330, height: 132),
                   styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .floating
        isReleasedWhenClosed = false
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
        hidesOnDeactivate = false
        animationBehavior = .none
        if #available(macOS 13.3, *) { hostingView.sizingOptions = [] }
        contentView = hostingView
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    override func sendEvent(_ event: NSEvent) {
        if event.type == .leftMouseDown || event.type == .rightMouseDown {
            panelController?.keepSettingsVisible()
        }
        super.sendEvent(event)
    }

    func updateLocaleIdentifier(_ localeIdentifier: String?) {
        guard self.localeIdentifier != localeIdentifier, let panelController else { return }
        self.localeIdentifier = localeIdentifier
        let view = Self.makePanelView(controller: panelController, localeIdentifier: localeIdentifier)
        hostingView.rootView = view
        sizingController.rootView = view
    }

    func setDraggingPassthrough(_ dragging: Bool) {
        ignoresMouseEvents = dragging
        alphaValue = dragging ? 0.72 : 1
        if dragging { orderBack(nil) }
        // Visibility is restored by the controller, respecting the foreground app.
    }

    func setVisible(_ visible: Bool) {
        guard visible else { orderOut(nil); return }
        let entering = !isVisible
        orderFrontRegardless()
        if entering && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            alphaValue = 0
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.2
                animator().alphaValue = panelController?.isDraggingApp == true ? 0.72 : 1
            }
        } else { alphaValue = panelController?.isDraggingApp == true ? 0.72 : 1 }
    }

    func snap(to settingsFrame: CGRect) {
        guard let controller = panelController,
              let screen = NSScreen.screens.max(by: {
                  $0.frame.intersection(settingsFrame).width * $0.frame.intersection(settingsFrame).height <
                  $1.frame.intersection(settingsFrame).width * $1.frame.intersection(settingsFrame).height
              }) else { return }
        let visible = screen.visibleFrame.insetBy(dx: 12, dy: 12)
        let width = min(330, visible.width)
        // Reserve pointer padding while measuring so changing sides cannot clip text.
        let height = measuredPanelHeight(for: width)
        let size = CGSize(width: width, height: height)
        let target = controller.targetFrame.flatMap { settingsFrame.insetBy(dx: -5, dy: -5).contains($0) ? $0 : nil }
        let placement = target.flatMap {
            PermissionGuidePlacement.besideWindow(settingsFrame, pointingAt: $0, size: size, visible: visible)
        } ?? PermissionGuidePlacement.nextTo(settingsFrame, size: size, visible: visible)
        let destination: CGRect
        if let placement {
            if controller.pointerSide != placement.side { controller.pointerSide = placement.side }
            if controller.pointerY != placement.pointerY { controller.pointerY = placement.pointerY }
            destination = placement.frame
        } else {
            if controller.pointerSide != .none { controller.pointerSide = .none }
            let height = measuredPanelHeight(for: width)
            let x = max(visible.minX, min(settingsFrame.maxX - width, visible.maxX - width))
            let y = max(visible.minY, min(settingsFrame.minY - height - 8, visible.maxY - height))
            destination = CGRect(x: x, y: y, width: width, height: height)
        }
        // Follow window drags directly; avoid chasing them with an animation.
        if frame != destination { setFrame(destination, display: true) }
    }

    private func measuredPanelHeight(for width: CGFloat) -> CGFloat {
        return max(96, sizingController.sizeThatFits(in: CGSize(width: width, height: 4096)).height)
    }

    private static func makePanelView(controller: PermissionFlowController, localeIdentifier: String?) -> AnyView {
        let view = PermissionFlowPanelView(controller: controller)
        guard let localeIdentifier else { return AnyView(view) }
        return AnyView(view.environment(\.locale, .init(identifier: localeIdentifier)))
    }
}
#endif
