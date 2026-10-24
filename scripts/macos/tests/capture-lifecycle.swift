import Foundation
import Darwin

private enum Failure: Error { case injected }
private final class Log: @unchecked Sendable {
    private let lock = NSLock()
    private var rows: [String] = []
    private var outcomes: [CaptureLifecycle.Event] = []
    private var codes: [Int32] = []
    func add(_ row: String) { lock.lock(); rows.append(row); lock.unlock() }
    func event(_ event: CaptureLifecycle.Event) { lock.lock(); outcomes.append(event); lock.unlock() }
    func exit(_ code: Int32) { lock.lock(); codes.append(code); rows.append("exit"); lock.unlock() }
    var entries: [String] { lock.lock(); defer { lock.unlock() }; return rows }
    var events: [CaptureLifecycle.Event] { lock.lock(); defer { lock.unlock() }; return outcomes }
    var exits: [Int32] { lock.lock(); defer { lock.unlock() }; return codes }
}
private final class Watchdog: @unchecked Sendable {
    private let lock = NSLock()
    private var actions: [@Sendable () -> Void] = []
    func arm(_ action: @escaping @Sendable () -> Void) { lock.lock(); actions.append(action); lock.unlock() }
    func fire() { lock.lock(); let pending = actions; actions = []; lock.unlock(); pending.forEach { $0() } }
}
private final class Gate: @unchecked Sendable {
    private let lock = NSLock()
    private var open = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    func wait() async {
        await withCheckedContinuation { continuation in
            lock.lock()
            if open { lock.unlock(); continuation.resume() }
            else { waiters.append(continuation); lock.unlock() }
        }
    }
    func release() { lock.lock(); open = true; let pending = waiters; waiters = []; lock.unlock(); pending.forEach { $0.resume() } }
}
private final class Holder: @unchecked Sendable { var lifecycle: CaptureLifecycle!; var writer: CapturePacketWriter! }
private func eventually(_ predicate: () -> Bool) async {
    for _ in 0..<2000 {
        if predicate() { return }
        try? await Task.sleep(nanoseconds: 1_000_000)
    }
    preconditionFailure("Fake operation did not reach the expected boundary")
}
private func make(_ log: Log, _ watchdog: Watchdog,
                  observe: (@Sendable (CaptureLifecycle.Event) -> Void)? = nil) -> CaptureLifecycle {
    CaptureLifecycle(graceNanoseconds: 100, watchdog: { delay, action in
        precondition(delay == 100); watchdog.arm(action)
    }, observe: { event in log.event(event); observe?(event) }, terminate: { log.exit($0) })
}
private func cleanup(_ life: CaptureLifecycle, _ log: Log, virtual: Bool = true) {
    life.installCleanup { [weak life] in
        log.add("cursor-stop"); life?.record(.stoppingCursor)
        log.add("stream-stop"); life?.record(.stoppingStream)
        if virtual { log.add("virtual-release"); life?.record(.virtualReleased) }
        else { life?.record(.noVirtual) }
        log.add("encoder-drain"); life?.record(.drainingEncoder)
    }
}

@main struct CaptureLifecycleChecks {
    static func main() async {
        if let mode = CommandLine.arguments.dropFirst().first {
            // Run in a subprocess: a real pipe with no reader must exercise
            // EPIPE/SIGPIPE, not merely an injected Swift Error.
            precondition(["pipe-source", "pipe-viewer", "pipe-baseline"].contains(mode))
            _ = signal(SIGPIPE, SIG_DFL)
            var descriptors = [Int32](repeating: 0, count: 2)
            precondition(pipe(&descriptors) == 0); close(descriptors[0])
            let output = FileHandle(fileDescriptor: descriptors[1], closeOnDealloc: true)
            if mode == "pipe-baseline" {
                do { try output.write(contentsOf: Data([1])) } catch { _exit(0) }
                preconditionFailure("Baseline unexpectedly wrote to a closed pipe")
            }
            let log = Log(), timer = Watchdog(), life = make(log, timer)
            let writer = CapturePacketWriter(write: { try output.write(contentsOf: $0) }, legacyFailure: { _ in _exit(0) })
            if mode == "pipe-source" {
                CapturePacketWriter.prepareSenderPipe(); writer.useSender(life); cleanup(life, log)
            }
            _ = writer.packet(Data([1]), measure: false)
            precondition(mode == "pipe-source", "Viewer changed its legacy termination policy")
            await eventually { !log.exits.isEmpty }
            precondition(log.exits == [1] && log.events.first?.reason == .stdoutFailure)
            precondition(log.entries == ["cursor-stop", "stream-stop", "virtual-release", "encoder-drain", "exit"])
            print("Real closed pipe: EPIPE reached sender shutdown, virtual release preceded recorded exit1; no display APIs invoked.")
            return
        }
        // Every real sender terminal entrance uses the same production policy.
        for reason in [CaptureLifecycle.Reason.stdinStop, .stdinEOF, .parentLost, .startupFailure, .captureFailure, .encoderFailure, .stdoutFailure] {
            let log = Log(), timer = Watchdog(), life = make(log, timer)
            cleanup(life, log)
            let fatal = [.startupFailure, .captureFailure, .encoderFailure, .stdoutFailure].contains(reason)
            precondition(life.requestStop(reason, error: fatal ? "original failure" : nil))
            precondition(!life.requestStop(.parentLost))
            await eventually { !log.exits.isEmpty }
            precondition(log.entries == ["cursor-stop", "stream-stop", "virtual-release", "encoder-drain", "exit"])
            precondition(log.exits == [fatal ? 1 : 0])
            precondition(log.events.last?.outcome == .complete && log.events.last?.reason == reason)
            timer.fire(); precondition(log.exits.count == 1)
        }

        // A Stop commits while start is suspended. No actor reentrancy can run
        // cleanup concurrently or allow a queued configure after that commit.
        for phase in [CaptureLifecycle.Phase.starting, .configuring] {
            let log = Log(), timer = Watchdog(), gate = Gate(), life = make(log, timer)
            life.installCleanup { [weak life] in
                precondition(log.entries.contains("lease-released"))
                log.add("virtual-release"); life?.record(.virtualReleased)
            }
            life.enqueue(phase) {
                log.add("lease-held"); await gate.wait()
                log.add("lease-released")
                if !life.isClosing { log.add("late-ready-or-topology") }
            }
            await eventually { log.entries.contains("lease-held") }
            life.enqueue(.configuring) { log.add("queued-configure") }
            precondition(life.requestStop(.stdinStop))
            precondition(!life.enqueue(.configuring) { log.add("new-configure") })
            precondition(log.exits.isEmpty && !log.entries.contains("virtual-release"))
            gate.release(); await eventually { !log.exits.isEmpty }
            precondition(log.entries == ["lease-held", "lease-released", "virtual-release", "exit"])
        }

        // A native drain stall must not keep ownership of the virtual display.
        do {
            let log = Log(), timer = Watchdog(), gate = Gate(), life = make(log, timer)
            life.installCleanup { [weak life] in
                log.add("virtual-release"); life?.record(.virtualReleased)
                life?.record(.drainingEncoder); log.add("drain-blocked"); await gate.wait()
            }
            life.requestStop(.stdinStop); await eventually { log.entries.contains("drain-blocked") }
            timer.fire(); precondition(log.exits == [1])
            precondition(log.events.last?.outcome == .timedOut && log.events.last?.virtualRelease == .returned)
            gate.release(); await Task.yield(); precondition(log.exits.count == 1)
        }

        // A non-cooperative OS operation cannot be mislabeled graceful. The
        // independent watchdog does not wait for the suspended async operation.
        for blockCleanup in [false, true] {
            let log = Log(), timer = Watchdog(), gate = Gate(), life = make(log, timer)
            if blockCleanup {
                life.installCleanup { [weak life] in life?.record(.stoppingStream); log.add("blocked"); await gate.wait(); life?.record(.virtualReleased) }
            } else {
                cleanup(life, log)
                life.enqueue(.starting) { log.add("blocked"); await gate.wait() }
                await eventually { log.entries.contains("blocked") }
            }
            life.requestStop(.parentLost)
            await eventually { log.entries.contains("blocked") }
            timer.fire(); precondition(log.exits == [1])
            precondition(log.events.last?.outcome == .timedOut && log.events.last?.virtualRelease == .notReached)
            gate.release(); await Task.yield(); precondition(log.exits.count == 1)
        }

        // Missing release evidence is an incomplete shutdown; mirror mode has
        // an explicit no-owned-virtual outcome instead of a fabricated release.
        for owned in [false, true] {
            let log = Log(), timer = Watchdog(), life = make(log, timer)
            if owned { life.installCleanup {} } else { cleanup(life, log, virtual: false) }
            life.requestStop(.startupFailure, error: "original failure")
            await eventually { !log.exits.isEmpty }
            precondition(log.exits == [1])
            precondition(log.events.last?.outcome == (owned ? .failed : .complete))
            precondition(log.events.last?.virtualRelease == (owned ? .notReached : .notOwned))
        }

        // Normal diagnostics cannot disable the independent shutdown bound.
        // A blocked observer is superseded by one timed-out termination.
        do {
            let log = Log(), timer = Watchdog(), blocked = DispatchSemaphore(value: 0)
            let life = make(log, timer, observe: { event in
                if event.outcome == .complete {
                    log.add("normal-diagnostics-blocked")
                    precondition(blocked.wait(timeout: .now() + 5) == .success)
                    log.add("normal-diagnostics-returned")
                }
            })
            cleanup(life, log); life.requestStop(.stdinStop)
            await eventually { log.entries.contains("normal-diagnostics-blocked") }
            timer.fire(); precondition(log.exits == [1])
            precondition(log.events.last?.outcome == .timedOut && log.events.last?.phase == .finishing)
            blocked.signal(); await eventually { log.entries.contains("normal-diagnostics-returned") }
            precondition(log.exits.count == 1)
        }

        // Once timeout commits, an older normal observer returning first must
        // not claim exit0 while timeout diagnostics are still in progress.
        do {
            let log = Log(), timer = Watchdog()
            let normal = DispatchSemaphore(value: 0), timeout = DispatchSemaphore(value: 0)
            let life = make(log, timer, observe: { event in
                if event.outcome == .complete {
                    log.add("normal-blocked"); precondition(normal.wait(timeout: .now() + 5) == .success)
                    log.add("normal-returned")
                } else if event.outcome == .timedOut {
                    log.add("timeout-blocked"); precondition(timeout.wait(timeout: .now() + 5) == .success)
                }
            })
            cleanup(life, log); life.requestStop(.stdinStop)
            await eventually { log.entries.contains("normal-blocked") }
            DispatchQueue.global().async { timer.fire() }
            await eventually { log.entries.contains("timeout-blocked") }
            normal.signal(); await eventually { log.entries.contains("normal-returned") }
            precondition(log.exits.isEmpty, "Older normal finish won after timeout committed")
            timeout.signal(); await eventually { !log.exits.isEmpty }
            precondition(log.exits == [1])
        }

        // A cleanup error preserves resource-release evidence and cannot turn
        // user Stop into an exit0 claim after a failed native stop.
        do {
            let log = Log(), timer = Watchdog(), life = make(log, timer)
            life.installCleanup { [weak life] in life?.record(.virtualReleased); throw Failure.injected }
            life.requestStop(.stdinStop); await eventually { !log.exits.isEmpty }
            precondition(log.exits == [1] && log.events.last?.outcome == .failed)
            precondition(log.events.last?.reason == .stdinStop && log.events.last?.virtualRelease == .returned)
        }

        // Execute the production packet writer's throwing branch. Its observer
        // reenters the writer; this would deadlock if failure held outputLock.
        do {
            let log = Log(), timer = Watchdog(), holder = Holder()
            let life = make(log, timer, observe: { event in
                if event.outcome == .requested {
                    if case .suppressed = holder.writer.packet(Data(), measure: false) { log.add("unlocked-reentry") }
                    else { preconditionFailure("Closing sender wrote another packet") }
                }
            })
            holder.lifecycle = life
            holder.writer = CapturePacketWriter(write: { _ in log.add("write-failed"); throw Failure.injected }, legacyFailure: { _ in preconditionFailure("Sender used legacy exit") })
            holder.writer.useSender(life); cleanup(life, log)
            if case .failed = holder.writer.packet(Data([1]), measure: true) {} else { preconditionFailure("Failure was lost") }
            await eventually { !log.exits.isEmpty }
            precondition(log.events.first?.reason == .stdoutFailure && log.exits == [1])
            precondition(log.entries.first == "write-failed" && log.entries.contains("unlocked-reentry"))
            precondition(log.entries.filter { $0 == "write-failed" }.count == 1)
        }

        // Viewer packets still use the injected legacy policy, never source
        // shutdown. Successful framed bytes retain their exact contents.
        do {
            let log = Log()
            let writer = CapturePacketWriter(write: { _ in throw Failure.injected }, legacyFailure: { _ in log.add("legacy-failure") })
            if case .failed = writer.packet(Data(), measure: false) {} else { preconditionFailure() }
            precondition(log.entries == ["legacy-failure"])
            let exact = Data([3, 0, 0, 0, 1, 9])
            let successful = CapturePacketWriter(write: { bytes in precondition(bytes == exact) }, legacyFailure: { _ in preconditionFailure() })
            if case .written(let acquired, let finished) = successful.packet(exact, measure: true) { precondition(finished >= acquired) }
            else { preconditionFailure() }
        }

        // Concurrent terminal entrances cannot create duplicate release/exit.
        for _ in 0..<64 {
            let log = Log(), timer = Watchdog(), life = make(log, timer)
            cleanup(life, log)
            DispatchQueue.concurrentPerform(iterations: 3) { index in
                life.requestStop(index == 0 ? .stdinStop : index == 1 ? .encoderFailure : .captureFailure,
                                 error: index == 0 ? nil : "original failure")
            }
            await eventually { !log.exits.isEmpty }
            timer.fire()
            precondition(log.exits.count == 1 && log.entries.filter { $0 == "virtual-release" }.count == 1)
            let requested = log.events.filter { $0.outcome == .requested }
            precondition(requested.count == 1 && log.events.last?.reason == requested.first?.reason)
        }
        print("Capture lifecycle: seven terminal reasons, serialized start/configure-stop, lease order, drain/OS/diagnostic stalls, incomplete cleanup, packet failure/reentry, legacy viewer policy and64 concurrent races passed; no display APIs invoked.")
    }
}
