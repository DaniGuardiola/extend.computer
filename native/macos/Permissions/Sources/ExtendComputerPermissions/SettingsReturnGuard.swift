import Foundation

struct SettingsWindowKey: Equatable {
    let pid: Int32
    let number: UInt32
}

// A failed lookup is not evidence that no Settings window existed.
struct SettingsReturnGuard {
    private(set) var eligible: Bool
    private(set) var owned: SettingsWindowKey?

    init(existing: [SettingsWindowKey]?) {
        eligible = existing?.isEmpty == true
    }

    mutating func observe(_ windows: [SettingsWindowKey]?) {
        guard eligible else { return }
        guard let windows, windows.count <= 1 else { eligible = false; return }
        if let owned {
            if windows != [owned] { eligible = false }
        } else if let first = windows.first {
            owned = first
        }
    }

    func mayClose(current: [SettingsWindowKey]?, permissionPage: Bool,
                  appEnabled: Bool, foregroundPID: Int32?) -> Bool {
        guard eligible, let owned else { return false }
        return current == [owned] && permissionPage && appEnabled && foregroundPID == owned.pid
    }
}
