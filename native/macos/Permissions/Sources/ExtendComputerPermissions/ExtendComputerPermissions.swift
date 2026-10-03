import AppKit
import PermissionFlow

@MainActor private var permissionTimer: Timer?
@MainActor private var controller: PermissionFlowController?
@MainActor private var requestID = UUID()
@MainActor private var checking = false

// Called only on Tauri's main thread. Keep the controller alive for panel tracking.
@_cdecl("extend_computer_permissions_open")
public func extendComputerPermissionsOpen(_ permission: Int32) {
    MainActor.assumeIsolated {
        permissionTimer?.invalidate()
        permissionTimer = nil
        controller?.closePanel()
        let id = UUID()
        requestID = id
        checking = false
        let pane: PermissionFlowPane = permission == 0 ? .inputMonitoring : .accessibility
        PermissionProbe.check(permission) { granted in
            guard requestID == id else { return }
            if granted == true {
                NSWorkspace.shared.open(pane.settingsURL)
                return
            }
            if controller == nil {
                controller = PermissionFlow.makeController(configuration: .init(promptForAccessibilityTrust: false))
            }
            let settingsReturn = SettingsReturn(pane: pane)
            controller?.authorize(pane: pane, suggestedAppURLs: [Bundle.main.bundleURL])
            let deadline = Date().addingTimeInterval(300)
            permissionTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in
                MainActor.assumeIsolated {
                    settingsReturn.observe()
                    let guideActive = controller?.isActive == true
                    guard guideActive, Date() < deadline else {
                        controller?.closePanel()
                        permissionTimer?.invalidate()
                        permissionTimer = nil
                        requestID = UUID()
                        return
                    }
                    guard !checking else { return }
                    checking = true
                    PermissionProbe.check(permission) { granted in
                        guard requestID == id else { return }
                        checking = false
                        guard granted == true else { return }
                        permissionTimer?.invalidate()
                        permissionTimer = nil
                        controller?.closePanel()
                        settingsReturn.finish()
                    }
                }
            }
        }
    }
}
