import Foundation
import Security

@objc protocol ProbeProtocol {
    func ping(reply: @escaping (String) -> Void)
}
final class Probe: NSObject, ProbeProtocol {
    func ping(reply: @escaping (String) -> Void) { reply("pong") }
}
final class Delegate: NSObject, NSXPCListenerDelegate {
    let probe = Probe()
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        connection.exportedInterface = NSXPCInterface(with: ProbeProtocol.self)
        connection.exportedObject = probe
        connection.resume()
        return true
    }
}

// Nonprivileged smoke test only. No network, interface, or service installation.
guard CommandLine.arguments.count == 2 else {
    print("Usage: XPCIdentityProbe <code-signing-requirement>")
    exit(64)
}
let requirement = CommandLine.arguments[1]
var validated: SecRequirement?
guard SecRequirementCreateWithString(requirement as CFString, [], &validated) == errSecSuccess else {
    fatalError("Invalid requirement")
}
let listener = NSXPCListener.anonymous()
let delegate = Delegate()
listener.delegate = delegate
listener.setConnectionCodeSigningRequirement(requirement)
listener.resume()
let connection = NSXPCConnection(listenerEndpoint: listener.endpoint)
connection.remoteObjectInterface = NSXPCInterface(with: ProbeProtocol.self)
connection.setCodeSigningRequirement(requirement)
connection.resume()
let completed = DispatchSemaphore(value: 0)
let proxy = connection.remoteObjectProxyWithErrorHandler { error in
    print("REJECTED: \(error.localizedDescription)")
    completed.signal()
} as! ProbeProtocol
proxy.ping { value in
    print("ACCEPTED: \(value)")
    completed.signal()
}
if completed.wait(timeout: .now() + 5) == .timedOut { print("TIMEOUT"); exit(2) }
connection.invalidate()
listener.invalidate()
