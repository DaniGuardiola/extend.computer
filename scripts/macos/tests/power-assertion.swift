import Foundation

// Takes a real display-sleep assertion for a moment and reads it back from
// pmset, as a user would, then checks that release ends it.
@main struct PowerAssertionChecks {
    static func listed(_ name: String) -> Bool {
        let pmset = Process()
        pmset.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
        pmset.arguments = ["-g", "assertions"]
        let output = Pipe()
        pmset.standardOutput = output
        try! pmset.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        pmset.waitUntilExit()
        precondition(pmset.terminationStatus == 0, "pmset failed")
        let owner = "pid \(ProcessInfo.processInfo.processIdentifier)("
        return String(decoding: data, as: UTF8.self).split(separator: "\n").contains {
            $0.contains(owner) && $0.contains("\(PowerAssertion.preventIdleDisplaySleep) named: \"\(name)\"")
        }
    }

    static func main() {
        let name = "extend.computer power assertion check \(ProcessInfo.processInfo.processIdentifier)"
        let assertion = PowerAssertion(PowerAssertion.preventIdleDisplaySleep, name: name)!
        precondition(listed(name), "The assertion is not listed under its name")
        assertion.release()
        assertion.release() // A second release (atexit after deinit) is harmless.
        let deadline = Date().addingTimeInterval(5)
        while listed(name) {
            precondition(Date() < deadline, "The assertion outlived its release")
            Thread.sleep(forTimeInterval: 0.05)
        }
        print("Power assertion: named display-sleep assertion listed by pmset and released")
    }
}
