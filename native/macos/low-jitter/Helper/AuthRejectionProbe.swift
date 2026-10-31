import Foundation

@objc protocol LowJitterProtocol {
    func status(reply: @escaping (Int, Bool) -> Void)
}

// Run only after the installed client's status command succeeds. This different
// executable has no authorized code hash and must not receive a status reply.
let connection = NSXPCConnection(machServiceName: "computer.extend.lowjitter-development", options: .privileged)
connection.remoteObjectInterface = NSXPCInterface(with: LowJitterProtocol.self)
connection.resume()
let done = DispatchSemaphore(value: 0)
let proxy = connection.remoteObjectProxyWithErrorHandler { error in
    print("REJECTED: \(error.localizedDescription)")
    done.signal()
} as! LowJitterProtocol
proxy.status { _, _ in
    print("FAIL: unauthorized executable received a reply")
    exit(1)
}
guard done.wait(timeout: .now() + 5) == .success else {
    print("INCONCLUSIVE: timeout")
    exit(2)
}
connection.invalidate()
