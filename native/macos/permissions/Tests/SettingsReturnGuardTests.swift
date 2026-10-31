import Foundation

@main struct SettingsReturnGuardTests {
    static func main() {
        let original = SettingsWindowKey(pid: 10, number: 20)
        let other = SettingsWindowKey(pid: 10, number: 21)
        func allowed(_ guardState: SettingsReturnGuard, _ windows: [SettingsWindowKey]? = nil,
                     page: Bool = true, enabled: Bool = true, foreground: Int32? = 10) -> Bool {
            guardState.mayClose(current: windows ?? [original], permissionPage: page,
                                appEnabled: enabled, foregroundPID: foreground)
        }
        var fresh = SettingsReturnGuard(existing: [])
        assert(!allowed(fresh))
        fresh.observe([])
        fresh.observe([original])
        assert(allowed(fresh))
        assert(!allowed(fresh, [other]))
        assert(!allowed(fresh, page: false))
        assert(!allowed(fresh, enabled: false))
        assert(!allowed(fresh, foreground: 30))
        assert(!allowed(fresh, foreground: nil))
        assert(!fresh.mayClose(current: nil, permissionPage: true, appEnabled: true, foregroundPID: 10))
        var existing = SettingsReturnGuard(existing: [original])
        existing.observe([other])
        assert(!allowed(existing, [other]))
        var unknown = SettingsReturnGuard(existing: nil)
        unknown.observe([original])
        assert(!allowed(unknown))
        var replaced = fresh
        replaced.observe([other])
        replaced.observe([original])
        assert(!allowed(replaced))
        var closed = fresh
        closed.observe([])
        closed.observe([original])
        assert(!allowed(closed))
        var multiple = fresh
        multiple.observe([original, other])
        multiple.observe([original])
        assert(!allowed(multiple))
        var lost = fresh
        lost.observe(nil)
        lost.observe([original])
        assert(!allowed(lost))
        print("Settings auto-close ownership guards passed")
    }
}
