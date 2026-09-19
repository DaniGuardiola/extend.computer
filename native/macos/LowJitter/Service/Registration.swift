import Foundation
import ServiceManagement

private let plistName = "computer.extend.lowjitter.plist"

private final class RegistrationProgress: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Int32? = nil
    func get() -> Int32? { lock.lock(); defer { lock.unlock() }; return value }
    func set(_ value: Int32?) { lock.lock(); defer { lock.unlock() }; self.value = value }
}
private let progress = RegistrationProgress()

/// Called inside the GUI process so macOS attributes approval to the app.
@_cdecl("extend_computer_wifi_status")
public func wifiStatus() -> Int32 {
    guard Bundle.main.bundleURL.pathExtension == "app" else { return -2 }
    if let value = progress.get() { return value }
    return Int32(SMAppService.daemon(plistName: plistName).status.rawValue)
}

private func registerService() -> Int32 {
    let service = SMAppService.daemon(plistName: plistName)
    do { try service.register() }
    catch {
        // macOS may return LaunchDeniedByUser while approval is still pending.
        guard service.status == .requiresApproval else {
            NSLog("Wi-Fi optimization registration failed: %@", String(describing: error))
            progress.set(-1)
            return -1
        }
    }
    progress.set(nil)
    if service.status == .requiresApproval { MainActor.assumeIsolated { showWifiGuide() } }
    return Int32(service.status.rawValue)
}

@_cdecl("extend_computer_wifi_allow")
public func wifiAllow() -> Int32 {
    if progress.get() == 4 { return 4 }
    do {
        guard Bundle.main.bundleURL.pathExtension == "app" else { return -2 }
        let identity = try ServiceIdentity.current(application: true)
        let broker = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/LaunchServices/ExtendComputerLowJitter")
        try identity.verifyBroker(at: broker)
        let service = SMAppService.daemon(plistName: plistName)
        switch service.status {
        case .notRegistered, .notFound:
            return registerService()
        case .enabled:
            SMAppService.openSystemSettingsLoginItems()
            return 1
        case .requiresApproval:
            progress.set(nil)
            MainActor.assumeIsolated { showWifiGuide() }
            return 2
        @unknown default:
            return -1
        }
    } catch {
        NSLog("Wi-Fi optimization identity check failed: %@", String(describing: error))
        return -2
    }
}
