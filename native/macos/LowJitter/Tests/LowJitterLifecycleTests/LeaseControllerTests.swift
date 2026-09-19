import Foundation
#if canImport(LowJitterLifecycle)
import LowJitterLifecycle
#endif

private func expectTrue(_ condition: @autoclosure () -> Bool) { precondition(condition()) }
private func expectFalse(_ condition: @autoclosure () -> Bool) { precondition(!condition()) }
private func expectEqual<T: Equatable>(_ actual: T, _ expected: T) { precondition(actual == expected) }
private func expectThrows<T>(_ expression: @autoclosure () throws -> T) {
    do { _ = try expression() } catch { return }
    preconditionFailure("Expected an error")
}

private enum Fault: Error { case injected }
private final class System: AWDLSystem {
    var up = true
    var pending = false
    var operations: [String] = []
    var failDown = false
    var failUp = false
    var failWrite = false
    var failClear = false
    func isUp() throws -> Bool { up }
    func restorationPending() throws -> Bool { pending }
    func setUp(_ value: Bool) throws {
        operations.append(value ? "up" : "down")
        if value && failUp { throw Fault.injected }
        up = value
        if !value && failDown { throw Fault.injected }
    }
    func writeRestorationPending(_ value: Bool) throws {
        operations.append(value ? "journal-set" : "journal-clear")
        if (value && failWrite) || (!value && failClear) { throw Fault.injected }
        pending = value
    }
}

final class LeaseControllerTests {
    private var time: TimeInterval = 100
    private func controller(_ system: System) throws -> LeaseController {
        try LeaseController(system: system, clock: { self.time })
    }

    func testOverlappingSessionsRestoreOnlyAfterLastRelease() throws {
        let system = System(), owner = UUID()
        let service = try controller(system)
        let first = try service.begin(owner: owner)
        let second = try service.begin(owner: owner)
        try service.end(first, owner: owner)
        expectFalse(system.up)
        try service.end(second, owner: owner)
        expectEqual(system.operations, ["journal-set", "down", "up", "journal-clear"])
    }

    func testExternalReactivationRejectsRenewalAndNewLease() throws {
        let system = System(), owner = UUID()
        let service = try controller(system)
        let lease = try service.begin(owner: owner)
        system.up = true
        expectThrows(try service.renew(lease, owner: owner))
        expectThrows(try service.begin(owner: UUID()))
        try service.disconnect(owner: owner)
        expectTrue(system.up)
        expectFalse(system.pending)
    }

    func testInitiallyDownRemainsDown() throws {
        let system = System(); system.up = false
        let service = try controller(system), owner = UUID()
        _ = try service.begin(owner: owner)
        try service.disconnect(owner: owner)
        expectFalse(system.up)
        expectTrue(system.operations.isEmpty)
    }

    func testRenewalExpiryAndLateRenewal() throws {
        let system = System(), owner = UUID()
        let service = try controller(system)
        let lease = try service.begin(owner: owner)
        time += 9
        try service.renew(lease, owner: owner)
        time += 9
        try service.tick()
        expectFalse(system.up)
        time += 1
        expectThrows(try service.renew(lease, owner: owner))
        expectTrue(system.up)
        expectEqual(service.activeLeaseCount, 0)
    }

    func testDisconnectedOwnerCannotAffectOtherOwner() throws {
        let system = System(), first = UUID(), second = UUID()
        let service = try controller(system)
        _ = try service.begin(owner: first)
        let remaining = try service.begin(owner: second)
        expectThrows(try service.end(remaining, owner: first))
        expectThrows(try service.renew(remaining, owner: first))
        try service.disconnect(owner: first)
        expectFalse(system.up)
        try service.disconnect(owner: second)
        expectTrue(system.up)
    }

    func testRestartRestoresJournaledChange() throws {
        let system = System()
        var service: LeaseController? = try controller(system)
        _ = try service!.begin(owner: UUID())
        service = nil // Simulate process loss; no graceful shutdown.
        expectFalse(system.up)
        let restarted = try controller(system)
        expectTrue(system.up)
        expectFalse(system.pending)
        expectEqual(restarted.activeLeaseCount, 0)
    }

    func testFailedRestoreRetainsJournalAndRetries() throws {
        let system = System(), owner = UUID()
        let service = try controller(system)
        _ = try service.begin(owner: owner)
        system.failUp = true
        expectThrows(try service.disconnect(owner: owner))
        expectTrue(system.pending)
        expectTrue(service.needsRestoration)
        expectThrows(try service.begin(owner: owner))
        system.failUp = false
        try service.tick()
        expectTrue(system.up)
        expectFalse(system.pending)
    }

    func testFailedJournalNeverLowersInterface() throws {
        let system = System(); system.failWrite = true
        let service = try controller(system)
        expectThrows(try service.begin(owner: UUID()))
        expectTrue(system.up)
        expectFalse(system.operations.contains("down"))
        expectEqual(service.activeLeaseCount, 0)
    }

    func testPartialLowerFailureRestores() throws {
        let system = System(); system.failDown = true
        let service = try controller(system)
        expectThrows(try service.begin(owner: UUID()))
        expectTrue(system.up)
        expectFalse(system.pending)
        expectEqual(service.activeLeaseCount, 0)
    }

    func testFailedJournalClearRetainsRecoveryObligation() throws {
        let system = System(), owner = UUID()
        let service = try controller(system)
        _ = try service.begin(owner: owner)
        system.failClear = true
        expectThrows(try service.shutdown())
        expectTrue(system.up)
        expectTrue(system.pending)
        system.failClear = false
        try service.tick()
        expectFalse(system.pending)
    }

    func testFailedStartupRecoveryRejectsController() throws {
        let system = System(); system.pending = true; system.up = false; system.failUp = true
        expectThrows(try controller(system))
        expectTrue(system.pending)
    }

    func testCapacityAndClockFailures() throws {
        let system = System()
        let service = try controller(system)
        for _ in 0..<LeaseController.maximumLeases { _ = try service.begin(owner: UUID()) }
        expectThrows(try service.begin(owner: UUID()))
        time -= 1
        expectThrows(try service.tick())
        time = .nan
        expectThrows(try service.tick())
        try service.shutdown()
        expectTrue(system.up)
    }

    func testDurableJournalRoundTrip() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: folder) }
        let journal = try RecoveryJournal(directoryPath: folder.path)
        expectEqual(try journal.pending(), false)
        try journal.write(pending: true)
        expectEqual(try RecoveryJournal(directoryPath: folder.path).pending(), true)
        try journal.write(pending: false)
        expectEqual(try RecoveryJournal(directoryPath: folder.path).pending(), false)
    }

    func testJournalRejectsSymlinkAndCorruption() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: folder) }
        let journal = try RecoveryJournal(directoryPath: folder.path)
        let file = folder.appendingPathComponent("restore")
        try FileManager.default.createSymbolicLink(at: file, withDestinationURL: folder.appendingPathComponent("missing"))
        expectThrows(try journal.pending())
        try FileManager.default.removeItem(at: file)
        try Data("invalid".utf8).write(to: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        expectThrows(try journal.pending())
    }

    func testSleepLikeTimeJumpExpiresAllLeases() throws {
        let system = System()
        let service = try controller(system)
        _ = try service.begin(owner: UUID())
        time += 3600
        try service.tick()
        expectTrue(system.up)
        expectFalse(system.pending)
    }
}

@main
struct LifecycleTestRunner {
    static func main() throws {
        try LeaseControllerTests().testOverlappingSessionsRestoreOnlyAfterLastRelease()
        print("PASS testOverlappingSessionsRestoreOnlyAfterLastRelease")
        try LeaseControllerTests().testInitiallyDownRemainsDown()
        print("PASS testInitiallyDownRemainsDown")
        try LeaseControllerTests().testRenewalExpiryAndLateRenewal()
        print("PASS testRenewalExpiryAndLateRenewal")
        try LeaseControllerTests().testDisconnectedOwnerCannotAffectOtherOwner()
        print("PASS testDisconnectedOwnerCannotAffectOtherOwner")
        try LeaseControllerTests().testRestartRestoresJournaledChange()
        print("PASS testRestartRestoresJournaledChange")
        try LeaseControllerTests().testFailedRestoreRetainsJournalAndRetries()
        print("PASS testFailedRestoreRetainsJournalAndRetries")
        try LeaseControllerTests().testFailedJournalNeverLowersInterface()
        print("PASS testFailedJournalNeverLowersInterface")
        try LeaseControllerTests().testPartialLowerFailureRestores()
        print("PASS testPartialLowerFailureRestores")
        try LeaseControllerTests().testFailedJournalClearRetainsRecoveryObligation()
        print("PASS testFailedJournalClearRetainsRecoveryObligation")
        try LeaseControllerTests().testFailedStartupRecoveryRejectsController()
        print("PASS testFailedStartupRecoveryRejectsController")
        try LeaseControllerTests().testCapacityAndClockFailures()
        print("PASS testCapacityAndClockFailures")
        try LeaseControllerTests().testSleepLikeTimeJumpExpiresAllLeases()
        print("PASS testSleepLikeTimeJumpExpiresAllLeases")
        try LeaseControllerTests().testDurableJournalRoundTrip()
        print("PASS testDurableJournalRoundTrip")
        try LeaseControllerTests().testJournalRejectsSymlinkAndCorruption()
        print("PASS testJournalRejectsSymlinkAndCorruption")
        try LeaseControllerTests().testExternalReactivationRejectsRenewalAndNewLease()
        print("PASS testExternalReactivationRejectsRenewalAndNewLease")
        print("15 lifecycle and journal tests passed")
    }
}
