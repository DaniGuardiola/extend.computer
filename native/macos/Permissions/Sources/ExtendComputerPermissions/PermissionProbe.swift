import Foundation
import Darwin

// A long-lived process can retain a positive AX trust result after revocation.
// Use the same fresh, non-prompting helper status check as the desktop backend.
enum PermissionProbe {
    @MainActor
    static func check(_ permission: Int32, completion: @escaping @MainActor @Sendable (Bool?) -> Void) {
        let helper = Bundle.main.resourceURL?
            .appendingPathComponent("helpers/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor")
        DispatchQueue.global(qos: .utility).async {
            let granted = helper.flatMap { read($0, permission: permission) }
            DispatchQueue.main.async { completion(granted) }
        }
    }

    private static func read(_ helper: URL, permission: Int32) -> Bool? {
        let process = Process()
        let output = Pipe()
        process.executableURL = helper
        process.arguments = ["status"]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do { try process.run() } catch { return nil }
        let deadline = ProcessInfo.processInfo.systemUptime + 4
        while process.isRunning {
            if ProcessInfo.processInfo.systemUptime >= deadline {
                kill(process.processIdentifier, SIGKILL)
                process.waitUntilExit()
                return nil
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        guard process.terminationStatus == 0,
              let data = try? output.fileHandleForReading.read(upToCount: 1024),
              let status = String(data: data, encoding: .utf8) else { return nil }
        return status.contains(permission == 0 ? "listen=true" : "post=true")
    }
}
