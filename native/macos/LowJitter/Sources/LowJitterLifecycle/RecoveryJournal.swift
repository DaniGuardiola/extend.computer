import Foundation
import Darwin

/// Durable single-bit restoration journal. The directory must already exist,
/// belong to the current effective user, and have no group/other permissions.
/// Production must supply a fixed root-owned directory. Tests use a private tempdir.
public final class RecoveryJournal {
    public enum Failure: Error { case system(Int32), unsafeDirectory, unsafeFile, corrupt }
    private let directory: Int32

    public init(directoryPath: String) throws {
        let descriptor = open(directoryPath, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard descriptor >= 0 else { throw Failure.system(errno) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == geteuid(),
              info.st_mode & 0o077 == 0 else {
            close(descriptor)
            throw Failure.unsafeDirectory
        }
        directory = descriptor
    }

    deinit { close(directory) }

    private func check(_ fd: Int32) throws {
        var info = stat()
        guard fstat(fd, &info) == 0 else { throw Failure.system(errno) }
        guard info.st_uid == geteuid(), info.st_mode & 0o077 == 0,
              info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1 else {
            throw Failure.unsafeFile
        }
    }

    public func pending() throws -> Bool {
        let fd = openat(directory, "restore", O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        if fd < 0 {
            if errno == ENOENT { return false }
            throw Failure.system(errno)
        }
        defer { close(fd) }
        try check(fd)
        var bytes = [UInt8](repeating: 0, count: 2)
        let count = read(fd, &bytes, bytes.count)
        guard count >= 0 else { throw Failure.system(errno) }
        guard count == 1, bytes[0] == 49 || bytes[0] == 48 else { throw Failure.corrupt }
        return bytes[0] == 49
    }

    public func write(pending: Bool) throws {
        // Unique O_EXCL temporary file makes an interrupted earlier write harmless.
        let temporary = "restore-\(UUID().uuidString)"
        let fd = openat(directory, temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { throw Failure.system(errno) }
        defer {
            close(fd)
            unlinkat(directory, temporary, 0)
        }
        var byte: UInt8 = pending ? 49 : 48
        guard Darwin.write(fd, &byte, 1) == 1 else { throw Failure.system(errno) }
        // FULLFSYNC requests a device flush on macOS, beyond ordinary fsync.
        guard fsync(fd) == 0, fcntl(fd, F_FULLFSYNC) == 0 else { throw Failure.system(errno) }
        guard renameat(directory, temporary, directory, "restore") == 0 else {
            throw Failure.system(errno)
        }
        guard fsync(directory) == 0, fcntl(fd, F_FULLFSYNC) == 0 else { throw Failure.system(errno) }
    }
}
