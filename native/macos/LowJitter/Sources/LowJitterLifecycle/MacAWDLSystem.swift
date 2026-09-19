import Foundation
import Darwin

/// Privileged adapter. All paths, interface names, and command arguments are fixed.
public final class MacAWDLSystem: AWDLSystem {
    public enum Failure: Error { case requiresRoot, missingInterface, query(Int32), commandFailed }
    private let journal: RecoveryJournal
    private let lockDescriptor: Int32

    public init(directoryPath: String = "/var/db/computer.extend.lowjitter-development") throws {
        guard geteuid() == 0 else { throw Failure.requiresRoot }
        journal = try RecoveryJournal(directoryPath: directoryPath)
        let descriptor = open(directoryPath + "/lease.lock",
                              O_RDWR | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { throw Failure.query(errno) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == 0,
              info.st_mode & 0o077 == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_nlink == 1, flock(descriptor, LOCK_EX | LOCK_NB) == 0 else {
            close(descriptor)
            throw Failure.commandFailed
        }
        lockDescriptor = descriptor
    }

    deinit { close(lockDescriptor) }

    public func isUp() throws -> Bool {
        var first: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&first) == 0 else { throw Failure.query(errno) }
        defer { freeifaddrs(first) }
        var entry = first
        while let current = entry {
            if String(cString: current.pointee.ifa_name) == "awdl0" {
                return current.pointee.ifa_flags & UInt32(IFF_UP) != 0
            }
            entry = current.pointee.ifa_next
        }
        throw Failure.missingInterface
    }

    public func setUp(_ up: Bool) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/sbin/ifconfig")
        process.arguments = ["awdl0", up ? "up" : "down"]
        process.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin"]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        // Bound failures so a stuck command cannot block lease expiry forever.
        let deadline = ProcessInfo.processInfo.systemUptime + 2
        while process.isRunning, ProcessInfo.processInfo.systemUptime < deadline { Thread.sleep(forTimeInterval: 0.01) }
        if process.isRunning {
            process.terminate()
            Thread.sleep(forTimeInterval: 0.1)
            if process.isRunning { kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
            throw Failure.commandFailed
        }
        process.waitUntilExit()
        guard process.terminationStatus == 0, try isUp() == up else { throw Failure.commandFailed }
    }

    public func restorationPending() throws -> Bool { try journal.pending() }
    public func writeRestorationPending(_ pending: Bool) throws { try journal.write(pending: pending) }
}
