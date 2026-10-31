import Foundation
import CoreGraphics
import ApplicationServices

// Small native boundary for edge control capture and input injection.
func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}
func output(_ text: String) {
    FileHandle.standardOutput.write(Data((text + "\n").utf8))
}
let mode = CommandLine.arguments.dropFirst().first ?? "status"
if mode == "status" {
    output("listen=\(CGPreflightListenEventAccess()) post=\(CGPreflightPostEventAccess())")
    exit(0)
}
if mode == "capture-control-left" || mode == "capture-control-right" {
    let args = CommandLine.arguments
    guard args.count == 5, let width = Double(args[2]), let height = Double(args[3]), let offset = Double(args[4]) else {
        fail("Edge capture needs remote width, height, and vertical offset in logical points.")
    }
    do { try runEdgeCapture(side: mode.hasSuffix("left") ? .left : .right,
                           remoteWidth: width, remoteHeight: height, offsetY: offset) }
    catch { fail("Invalid display layout: \(error)") }
    exit(0)
}
if mode == "inject-display" {
    guard CommandLine.arguments.count == 3 else { fail("Display injection requires one local target descriptor.") }
    do {
        let target = try DisplayInputTarget.decode(Data(CommandLine.arguments[2].utf8))
        runInputReceiver(target: target)
    } catch { fail("Invalid display input target: \(error)") }
    exit(0)
}
if mode == "inject-control" { runInputReceiver(); exit(0) }
fail("Usage: ExtendComputerCursor status|capture-control-left|capture-control-right|inject-control|inject-display")
