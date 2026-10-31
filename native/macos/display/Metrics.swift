import Foundation
import CoreMedia
import Darwin

// Optional test telemetry. No image bytes, device identities, or input events.
final class DisplayMetrics {
    static let shared = DisplayMetrics()
    private let lock = NSLock()
    private var file: FileHandle?
    private var pending = Data()
    var enabled: Bool { file != nil }
    private init() {
        guard let path = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_METRICS"] else { return }
        // Helpers restart during reconnects within one prepared app session.
        // Append atomically instead of silently disabling telemetry on reconnect.
        let descriptor = open(path, O_WRONLY | O_APPEND | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { return }
        var info = stat()
        guard fstat(descriptor, &info) == 0,
            info.st_mode & S_IFMT == S_IFREG,
            info.st_uid == getuid(), info.st_nlink == 1,
            info.st_mode & 0o777 == 0o600 else {
            close(descriptor); return
        }
        file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    }
    func event(_ stage: String, timestamp: UInt64 = 0, bytes: Int = 0, atNanoseconds: UInt64? = nil) {
        guard file != nil else { return }
        let now = UInt64(CMTimeGetSeconds(CMClockGetTime(CMClockGetHostTimeClock())) * 1e9)
        let recorded = atNanoseconds ?? now
        let wall = UInt64(Date().timeIntervalSince1970 * 1e9 + Double(recorded) - Double(now))
        let row = "{\"stage\":\"\(stage)\",\"at_ns\":\(recorded),\"wall_ns\":\(wall),\"frame_ns\":\(timestamp),\"bytes\":\(bytes)}\n"
        lock.lock(); defer { lock.unlock() }
        pending.append(Data(row.utf8))
        if pending.count >= 4096 {
            try? file?.write(contentsOf: pending); pending.removeAll(keepingCapacity: true)
        }
    }
    func flush() {
        lock.lock(); defer { lock.unlock() }
        try? file?.write(contentsOf: pending); pending.removeAll(keepingCapacity: true)
    }
}
