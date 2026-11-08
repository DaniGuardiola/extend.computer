import Foundation
import IOKit.pwr_mgt

/// A named IOKit power assertion. It shows in `pmset -g assertions` until it
/// is released; macOS also ends it with the process.
final class PowerAssertion {
    /// `kIOPMAssertionTypePreventUserIdleDisplaySleep`: the display stays on
    /// while the user is idle, and so does the Mac.
    static let preventIdleDisplaySleep = "PreventUserIdleDisplaySleep"
    private let lock = NSLock()
    private var id = IOPMAssertionID(kIOPMNullAssertionID)

    init?(_ type: String, name: String) {
        guard IOPMAssertionCreateWithName(type as CFString, IOPMAssertionLevel(kIOPMAssertionLevelOn),
                                          name as CFString, &id) == kIOReturnSuccess else { return nil }
    }

    func release() {
        lock.lock(); defer { lock.unlock() }
        guard id != IOPMAssertionID(kIOPMNullAssertionID) else { return }
        IOPMAssertionRelease(id)
        id = IOPMAssertionID(kIOPMNullAssertionID)
    }

    deinit { release() }
}
