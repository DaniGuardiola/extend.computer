#if os(macOS)
import SwiftUI

// A continuous outline keeps the material, border, and pointer seamless.
struct PermissionGuideBubble: Shape {
    var side: PermissionGuidePointerSide
    var pointerY: CGFloat

    func path(in rect: CGRect) -> Path {
        let left: CGFloat = side == .left ? 12 : 0
        let right = rect.width - (side == .right ? 12 : 0)
        let bottom = rect.height
        let radius: CGFloat = 18
        let y = min(bottom - 25, max(25, bottom - pointerY))
        var p = Path()
        p.move(to: CGPoint(x: left + radius, y: 0))
        p.addLine(to: CGPoint(x: right - radius, y: 0))
        p.addQuadCurve(to: CGPoint(x: right, y: radius), control: CGPoint(x: right, y: 0))
        if side == .right {
            p.addLine(to: CGPoint(x: right, y: y - 9))
            p.addLine(to: CGPoint(x: rect.width, y: y))
            p.addLine(to: CGPoint(x: right, y: y + 9))
        }
        p.addLine(to: CGPoint(x: right, y: bottom - radius))
        p.addQuadCurve(to: CGPoint(x: right - radius, y: bottom), control: CGPoint(x: right, y: bottom))
        p.addLine(to: CGPoint(x: left + radius, y: bottom))
        p.addQuadCurve(to: CGPoint(x: left, y: bottom - radius), control: CGPoint(x: left, y: bottom))
        if side == .left {
            p.addLine(to: CGPoint(x: left, y: y + 9))
            p.addLine(to: CGPoint(x: 0, y: y))
            p.addLine(to: CGPoint(x: left, y: y - 9))
        }
        p.addLine(to: CGPoint(x: left, y: radius))
        p.addQuadCurve(to: CGPoint(x: left + radius, y: 0), control: CGPoint(x: left, y: 0))
        p.closeSubpath()
        return p
    }
}

@available(macOS 13.0, *)
struct PermissionFlowPanelView: View {
    @ObservedObject var controller: PermissionFlowController

    var body: some View {
        let shape = PermissionGuideBubble(side: controller.pointerSide, pointerY: controller.pointerY)
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                Image(systemName: controller.currentPane == nil ? "wifi" : "hand.raised.fill")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 32, height: 32)
                    .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
                VStack(alignment: .leading, spacing: 3) {
                    Text(controller.currentPane?.localizedTitle(localeIdentifier: controller.localeIdentifier) ?? "Wi-Fi optimization")
                        .font(.system(size: 14, weight: .semibold))
                    Text(appDisplayName).font(.system(size: 11)).foregroundStyle(.secondary)
                }
                Spacer(minLength: 4)
                Button { controller.closePanel(returnToPreviousApp: true) } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 18))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Close permission guide")
            }
            Text(instruction)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .lineSpacing(2)
                .fixedSize(horizontal: false, vertical: true)
            if controller.guidanceBody == nil, controller.targetFrame == nil,
               let app = controller.preferredAppURL {
                AppDragItemView(url: app, localeIdentifier: controller.localeIdentifier) {
                    controller.setPanelDragging($0)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(16)
        .padding(.leading, controller.pointerSide == .left ? 12 : 0)
        .padding(.trailing, controller.pointerSide == .left ? 0 : 12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .fixedSize(horizontal: false, vertical: true)
        .background(.ultraThinMaterial, in: shape)
        .overlay(shape.stroke(.primary.opacity(0.14), lineWidth: 1))
    }

    private var instruction: String {
        if let body = controller.guidanceBody {
            return [controller.guidanceTitle, body].compactMap { $0 }.joined(separator: ". ")
        }
        if controller.targetFrame != nil { return "Turn on \(appDisplayName) to allow access." }
        let template = PermissionFlowLocalizer.string(
            "permission_flow.panel.add_app", defaultValue: "Drag %@ into the list in System Settings, then turn it on.",
            localeIdentifier: controller.localeIdentifier
        )
        return String(format: template, appDisplayName)
    }

    private var appDisplayName: String {
        guard let app = controller.preferredAppURL else { return "This App" }
        return FileManager.default.displayName(atPath: app.path)
    }
}
#endif
