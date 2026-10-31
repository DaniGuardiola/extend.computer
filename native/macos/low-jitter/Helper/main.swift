import Foundation
import Darwin
import Security

#if APP_SERVICE
import SystemConfiguration
private var serviceName: String { (try? ServiceIdentity.current().name) ?? "invalid" }
#else
private let serviceName = "computer.extend.lowjitter-development"
#endif
private let installDirectory = "/Library/PrivilegedHelperTools/computer.extend.lowjitter-development"

@objc protocol LowJitterProtocol {
    func begin(reply: @escaping (String?, String?) -> Void)
    func renew(_ token: String, reply: @escaping (String?) -> Void)
    func end(_ token: String, reply: @escaping (String?) -> Void)
    func status(reply: @escaping (Int, Bool) -> Void)
}

struct Configuration: Decodable {
    let uid: UInt32
    let requirement: String
    static func load() throws -> Configuration {
        #if APP_SERVICE
        let identity = try ServiceIdentity.current()
        return Configuration(uid: 0, requirement: identity.requirement)
        #else
        let fd = open(installDirectory + "/configuration.json", O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { throw HelperError.configuration }
        defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == 0, info.st_mode & 0o022 == 0,
              info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1,
              info.st_size > 0, info.st_size <= 4096 else { throw HelperError.configuration }
        var bytes = [UInt8](repeating: 0, count: Int(info.st_size))
        guard read(fd, &bytes, bytes.count) == bytes.count else { throw HelperError.configuration }
        let config = try JSONDecoder().decode(Configuration.self, from: Data(bytes))
        // Development identity is exactly one code-directory hash, not an
        // arbitrary requirement supplied by a connecting process.
        guard config.uid != 0, config.requirement.range(of: "^cdhash H\"[0-9a-f]{40}\"$", options: .regularExpression) != nil else {
            throw HelperError.configuration
        }
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(config.requirement as CFString, [], &requirement) == errSecSuccess else {
            throw HelperError.configuration
        }
        return config
        #endif
    }
}
enum HelperError: Error { case configuration, unavailable, connectionInvalidated, connectionInterrupted, proxyError, rejected(String), usage }

final class SessionEndpoint: NSObject, LowJitterProtocol {
    let owner = UUID()
    let controller: LeaseController
    let queue: DispatchQueue
    private var disconnected = false
    init(controller: LeaseController, queue: DispatchQueue) {
        self.controller = controller; self.queue = queue
    }
    func begin(reply: @escaping (String?, String?) -> Void) {
        queue.async {
            do {
                guard !self.disconnected else { throw HelperError.unavailable }
                reply(try self.controller.begin(owner: self.owner).uuidString, nil)
            }
            catch { reply(nil, String(describing: error)) }
        }
    }
    func renew(_ token: String, reply: @escaping (String?) -> Void) {
        queue.async {
            do {
                guard let id = UUID(uuidString: token) else { throw LeaseController.Failure.unknownLease }
                try self.controller.renew(id, owner: self.owner)
                reply(nil)
            } catch { reply(String(describing: error)) }
        }
    }
    func end(_ token: String, reply: @escaping (String?) -> Void) {
        queue.async {
            do {
                guard let id = UUID(uuidString: token) else { throw LeaseController.Failure.unknownLease }
                try self.controller.end(id, owner: self.owner)
                reply(nil)
            } catch { reply(String(describing: error)) }
        }
    }
    func status(reply: @escaping (Int, Bool) -> Void) {
        queue.async { reply(self.controller.activeLeaseCount, self.controller.needsRestoration) }
    }
    func disconnect() {
        queue.async {
            self.disconnected = true
            try? self.controller.disconnect(owner: self.owner)
        }
    }
}

final class ListenerDelegate: NSObject, NSXPCListenerDelegate {
    let controller: LeaseController
    let queue: DispatchQueue
    let uid: UInt32
    init(controller: LeaseController, queue: DispatchQueue, uid: UInt32) {
        self.controller = controller; self.queue = queue; self.uid = uid
    }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        #if APP_SERVICE
        var consoleUID: uid_t = 0
        _ = SCDynamicStoreCopyConsoleUser(nil, &consoleUID, nil)
        guard consoleUID > 0, connection.effectiveUserIdentifier == consoleUID else { return false }
        #else
        guard connection.effectiveUserIdentifier == uid else { return false }
        #endif
        let endpoint = SessionEndpoint(controller: controller, queue: queue)
        connection.exportedInterface = NSXPCInterface(with: LowJitterProtocol.self)
        connection.exportedObject = endpoint
        connection.invalidationHandler = { endpoint.disconnect() }
        connection.interruptionHandler = { endpoint.disconnect() }
        connection.resume()
        return true
    }
}

func makeController() throws -> LeaseController {
    var timebase = mach_timebase_info_data_t()
    mach_timebase_info(&timebase)
    let ratio = Double(timebase.numer) / Double(timebase.denom) / 1_000_000_000
    #if APP_SERVICE
    let identity = try ServiceIdentity.current()
    let directory = "/private/var/db/" + identity.name
    // Fixed root-owned parent; never follow an attacker-created journal directory.
    let parent = open("/private/var/db", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard parent >= 0 else { throw HelperError.configuration }
    defer { close(parent) }
    var info = stat()
    guard fstat(parent, &info) == 0, info.st_uid == 0, info.st_mode & 0o022 == 0 else { throw HelperError.configuration }
    guard mkdirat(parent, identity.name, 0o700) == 0 || errno == EEXIST else { throw HelperError.configuration }
    let system = try MacAWDLSystem(directoryPath: directory)
    #else
    let system = try MacAWDLSystem()
    #endif
    return try LeaseController(system: system, clock: { Double(mach_continuous_time()) * ratio })
}

func serve() throws -> Never {
    guard geteuid() == 0 else { throw MacAWDLSystem.Failure.requiresRoot }
    let config = try Configuration.load()
    let controller = try makeController()
    let queue = DispatchQueue(label: "computer.extend.lowjitter.lifecycle")
    let delegate = ListenerDelegate(controller: controller, queue: queue, uid: config.uid)
    let listener = NSXPCListener(machServiceName: serviceName)
    listener.setConnectionCodeSigningRequirement(config.requirement)
    listener.delegate = delegate
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + 1, repeating: 1)
    timer.setEventHandler { try? controller.tick() }
    timer.resume()
    signal(SIGTERM, SIG_IGN)
    signal(SIGINT, SIG_IGN)
    let signals = [SIGTERM, SIGINT].map { number -> DispatchSourceSignal in
        let source = DispatchSource.makeSignalSource(signal: number, queue: queue)
        source.setEventHandler {
            do { try controller.shutdown(); exit(0) } catch { exit(1) }
        }
        source.resume()
        return source
    }
    listener.resume()
    withExtendedLifetime((delegate, listener, timer, signals)) { dispatchMain() }
}

// BEGIN XPC_REQUEST_DIAGNOSTIC
// Fixed categories only. A failed connection wakes the current request and
// rejects later ones; replies after failure cannot overwrite the first result.
final class XPCFailureChannel {
    private let lock = NSLock()
    private var failure: HelperError?
    private var waiting: (UUID, (HelperError) -> Void)?
    func subscribe(_ id: UUID, _ callback: @escaping (HelperError) -> Void) {
        lock.lock()
        let failed = failure
        if failed == nil { waiting = (id, callback) }
        lock.unlock()
        if let failed { callback(failed) }
    }
    func unsubscribe(_ id: UUID) {
        lock.lock(); defer { lock.unlock() }
        if waiting?.0 == id { waiting = nil }
    }
    func fail(_ error: HelperError) {
        lock.lock()
        guard failure == nil else { lock.unlock(); return }
        failure = error
        let callback = waiting?.1
        waiting = nil
        lock.unlock()
        callback?(error)
    }
}
final class XPCRequestResult<T> {
    let ready = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var result: Result<T, HelperError>?
    func complete(_ value: Result<T, HelperError>) {
        lock.lock()
        guard result == nil else { lock.unlock(); return }
        result = value
        lock.unlock()
        ready.signal()
    }
    func completed() -> Bool {
        lock.lock(); defer { lock.unlock() }
        return result != nil
    }
    func value() throws -> T {
        lock.lock(); defer { lock.unlock() }
        return try result!.get()
    }
}
func request<T>(_ failures: XPCFailureChannel, timeout: TimeInterval = 5,
                _ operation: (@escaping (T) -> Void) -> Void) throws -> T {
    let result = XPCRequestResult<T>()
    let id = UUID()
    failures.subscribe(id) { result.complete(.failure($0)) }
    defer { failures.unsubscribe(id) }
    if !result.completed() { operation { result.complete(.success($0)) } }
    if result.ready.wait(timeout: .now() + timeout) != .success {
        result.complete(.failure(.unavailable))
    }
    return try result.value()
}
// END XPC_REQUEST_DIAGNOSTIC

func client(hold: Bool, pipeBound: Bool = false, persistent: Bool = false) throws {
    let config = try Configuration.load()
    #if APP_SERVICE
    guard geteuid() != 0 else { throw HelperError.configuration }
    #else
    guard geteuid() == config.uid else { throw HelperError.configuration }
    #endif
    let connection = NSXPCConnection(machServiceName: serviceName, options: .privileged)
    connection.setCodeSigningRequirement(config.requirement)
    connection.remoteObjectInterface = NSXPCInterface(with: LowJitterProtocol.self)
    let failures = XPCFailureChannel()
    connection.invalidationHandler = { failures.fail(.connectionInvalidated) }
    connection.interruptionHandler = { failures.fail(.connectionInterrupted) }
    connection.resume()
    defer { connection.invalidate() }
    let proxy = connection.remoteObjectProxyWithErrorHandler { _ in failures.fail(.proxyError) } as! LowJitterProtocol
    if !hold {
        let status: (Int, Bool) = try request(failures) { done in proxy.status { done(($0, $1)) } }
        print("protocol=2 leases=\(status.0) restoration_pending=\(status.1)")
        return
    }
    let response: (String?, String?) = try request(failures) { done in proxy.begin { done(($0, $1)) } }
    guard let token = response.0, response.1 == nil else { throw HelperError.rejected(response.1 ?? "Missing lease") }
    if pipeBound { print("READY"); fflush(stdout) }
    else { print("AWDL lease active for 30 seconds; disconnect or expiry restores prior state.") }
    let deadline = ProcessInfo.processInfo.systemUptime + 30
    while persistent || ProcessInfo.processInfo.systemUptime < deadline {
        if pipeBound {
            var descriptor = pollfd(fd: STDIN_FILENO, events: Int16(POLLIN | POLLHUP), revents: 0)
            let result = poll(&descriptor, 1, 2000)
            if result < 0 {
                if errno == EINTR { continue }
                throw HelperError.unavailable
            }
            // EOF, data, or pipe errors all terminate the lease. Parent sends no
            // commands; keeping this pipe open is the entire lifetime contract.
            if result > 0 { break }
        } else {
            Thread.sleep(forTimeInterval: 2)
        }
        let error: String? = try request(failures) { done in proxy.renew(token, reply: done) }
        if let error { throw HelperError.rejected(error) }
    }
    let error: String? = try request(failures) { done in proxy.end(token, reply: done) }
    if let error { throw HelperError.rejected(error) }
    if !pipeBound { print("Lease ended; restoration completed if extend.computer paused AWDL.") }
}

do {
    guard CommandLine.arguments.count == 2 else { throw HelperError.usage }
    switch CommandLine.arguments[1] {
    case "serve": try serve()
    case "recover": try makeController().shutdown()
    case "status": try client(hold: false)
    case "test-30s": try client(hold: true)
    case "lease": try client(hold: true, pipeBound: true)
    case "session": try client(hold: true, pipeBound: true, persistent: true)
    default: throw HelperError.usage
    }
} catch {
    if CommandLine.arguments.dropFirst().first == "status" {
        let category: String
        switch error {
        case HelperError.connectionInvalidated: category = "connection_invalidated"
        case HelperError.connectionInterrupted: category = "connection_interrupted"
        case HelperError.proxyError: category = "proxy_error"
        case HelperError.unavailable: category = "timeout"
        case HelperError.configuration: category = "configuration"
        #if APP_SERVICE
        case is ServiceIdentity.Failure: category = "signing_identity"
        #endif
        default: category = "other"
        }
        FileHandle.standardError.write(Data("readiness_failure=\(category)\n".utf8))
    } else {
        FileHandle.standardError.write(Data("extend.computer low-jitter helper: \(error)\n".utf8))
    }
    exit(1)
}
