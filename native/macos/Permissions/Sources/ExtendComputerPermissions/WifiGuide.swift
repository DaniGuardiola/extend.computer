import AppKit
import PermissionFlow
import ServiceManagement

@MainActor
private var wifiGuide: PermissionFlowController?
@MainActor
private var wifiGuideTimer: Timer?

@MainActor
func showWifiGuide() {
    wifiGuideTimer?.invalidate()
    let guide = wifiGuide ?? PermissionFlow.makeController()
    wifiGuide = guide
    let settingsReturn = SettingsReturn(backgroundActivity: ())
    guide.guide(
        title: "Enable extend.computer under App Background Activity",
        body: "This helps keep your keyboard and mouse responsive over Wi-Fi. AirDrop pauses while you’re connected and resumes when you disconnect.",
        openSettings: { SMAppService.openSystemSettingsLoginItems() }
    )
    let deadline = Date().addingTimeInterval(300)
    wifiGuideTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in
        MainActor.assumeIsolated {
            settingsReturn.observe()
            let status = wifiStatus()
            let visible = NSApp.windows.contains { $0 is NSPanel && $0.isVisible }
            if status != 2 || !visible || Date() >= deadline {
                wifiGuideTimer?.invalidate()
                wifiGuideTimer = nil
                guide.closePanel()
                if status == 1 { settingsReturn.finish() }
            }
        }
    }
}
