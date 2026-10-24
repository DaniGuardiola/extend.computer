import Foundation
import Darwin

// Sender-only lifetime policy. This file has no capture/display dependencies so
// failures, suspended operations and shutdown deadlines can be tested with fakes.
final class CaptureLifecycle: @unchecked Sendable {
    enum Reason: String, Sendable {
        case stdinStop, stdinEOF, parentLost, startupFailure, captureFailure, encoderFailure, stdoutFailure
        var isFailure: Bool {
            switch self {
            case .stdinStop, .stdinEOF, .parentLost: return false
            default: return true
            }
        }
    }
    enum Phase: String, Sendable { case idle, starting, configuring, stoppingCursor, stoppingStream, releasingVirtual, virtualReleased, noVirtual, drainingEncoder, finishing }
    enum Outcome: String, Sendable { case requested, phase, complete, failed, timedOut }
    enum VirtualRelease: String, Sendable { case notReached, notOwned, returned }
    struct Event: Sendable {
        let outcome: Outcome
        let reason: Reason
        let phase: Phase
        let virtualRelease: VirtualRelease
        let exitCode: Int32
        let error: String?
    }
    typealias Watchdog = @Sendable (UInt64, @escaping @Sendable () -> Void) -> Void
    private struct Work {
        let phase: Phase
        let operation: @Sendable () async throws -> Void
    }
    private let lock = NSLock()
    private var pending: [Work] = []
    private var running = false
    private var cleanupStarted = false
    private var ended = false
    private var finishing = false
    private var terminalRevision: UInt64 = 0
    private var reason: Reason?
    private var error: String?
    private var exitCode: Int32 = 0
    private var phase: Phase = .idle
    private var virtualRelease: VirtualRelease = .notReached
    private var cleanup: (@Sendable () async throws -> Void)?
    private let grace: UInt64
    private let watchdog: Watchdog
    private let observe: @Sendable (Event) -> Void
    private let terminate: @Sendable (Int32) -> Void

    init(graceNanoseconds: UInt64 = 1_500_000_000,
         watchdog: @escaping Watchdog = { delay, action in
             DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + .nanoseconds(Int(clamping: delay)), execute: action)
         }, observe: @escaping @Sendable (Event) -> Void,
         terminate: @escaping @Sendable (Int32) -> Void) {
        grace = graceNanoseconds; self.watchdog = watchdog
        self.observe = observe; self.terminate = terminate
    }
    var isClosing: Bool { lock.lock(); defer { lock.unlock() }; return reason != nil }

    func installCleanup(_ operation: @escaping @Sendable () async throws -> Void) {
        lock.lock(); defer { lock.unlock() }
        precondition(!running && reason == nil && cleanup == nil)
        cleanup = operation
    }
    @discardableResult
    func enqueue(_ phase: Phase, operation: @escaping @Sendable () async throws -> Void) -> Bool {
        lock.lock()
        guard reason == nil && !ended else { lock.unlock(); return false }
        pending.append(Work(phase: phase, operation: operation))
        let launch = !running; running = true
        lock.unlock()
        if launch { Task { await self.pump() } }
        return true
    }
    @discardableResult
    func requestStop(_ reason: Reason, error: String? = nil) -> Bool {
        lock.lock()
        guard self.reason == nil && !ended else { lock.unlock(); return false }
        self.reason = reason; self.error = error
        exitCode = reason.isFailure || error != nil ? 1 : 0
        pending.removeAll()
        let event = snapshot(.requested), launch = !running
        running = true
        lock.unlock()
        // Start the bound before diagnostics or cleanup; never wait for a
        // canceled, non-cooperative OS TaskGroup child to enforce this bound.
        watchdog(grace) { [weak self] in self?.finish(.timedOut) }
        observe(event)
        if launch { Task { await self.pump() } }
        return true
    }
    func record(_ phase: Phase) {
        lock.lock()
        guard reason != nil && !ended && !finishing else { lock.unlock(); return }
        self.phase = phase
        if phase == .virtualReleased { virtualRelease = .returned }
        if phase == .noVirtual { virtualRelease = .notOwned }
        let event = snapshot(.phase)
        lock.unlock(); observe(event)
    }
    private enum Next { case operation(Work), cleanup(@Sendable () async throws -> Void), done }
    private func next() -> Next {
        lock.lock(); defer { lock.unlock() }
        if ended || finishing { running = false; return .done }
        if reason != nil {
            guard !cleanupStarted else { running = false; return .done }
            cleanupStarted = true
            return .cleanup(cleanup ?? {})
        }
        guard !pending.isEmpty else { running = false; return .done }
        let work = pending.removeFirst(); phase = work.phase
        return .operation(work)
    }
    private func pump() async {
        while true {
            switch next() {
            case .operation(let work):
                do { try await work.operation() }
                catch { requestStop(.startupFailure, error: String(describing: error)) }
            case .cleanup(let operation):
                do { try await operation(); finish(.complete) }
                catch { finish(.failed, cleanupError: String(describing: error)) }
                return
            case .done: return
            }
        }
    }
    private func snapshot(_ outcome: Outcome, cleanupError: String? = nil) -> Event {
        Event(outcome: outcome, reason: reason!, phase: phase, virtualRelease: virtualRelease,
              exitCode: outcome == .timedOut || outcome == .failed ? 1 : exitCode,
              error: cleanupError ?? error)
    }
    private func finish(_ outcome: Outcome, cleanupError: String? = nil) {
        lock.lock()
        guard !ended && reason != nil && (!finishing || outcome == .timedOut) else { lock.unlock(); return }
        let missingRelease = outcome == .complete && virtualRelease == .notReached
        finishing = true
        terminalRevision += 1
        let revision = terminalRevision
        if outcome != .timedOut { phase = .finishing }
        let event = snapshot(missingRelease ? .failed : outcome,
                             cleanupError: missingRelease ? "Cleanup did not record virtual-display ownership release." : cleanupError)
        pending.removeAll(); cleanup = nil
        lock.unlock()
        // Normal diagnostic flush may itself stall. Until termination is
        // claimed, the watchdog may supersede it with a best-effort receipt.
        observe(event)
        lock.lock()
        guard !ended && terminalRevision == revision else { lock.unlock(); return }
        ended = true
        lock.unlock(); terminate(event.exitCode)
    }
}

// Keep the legacy viewer error policy, but route sender pipe errors through
// cleanup after unlocking. The same production write path is used by fake tests.
final class CapturePacketWriter: @unchecked Sendable {
    enum Result { case written(acquired: UInt64, finished: UInt64), suppressed, failed }
    private let lock = NSLock()
    private let write: (Data) throws -> Void
    private let legacyFailure: (Error) -> Void
    private weak var lifecycle: CaptureLifecycle?
    init(write: @escaping (Data) throws -> Void, legacyFailure: @escaping (Error) -> Void) {
        self.write = write; self.legacyFailure = legacyFailure
    }
    // Called only by sender mode, before any capture owns a virtual display.
    // EPIPE must reach the throwing write path rather than terminate on SIGPIPE.
    static func prepareSenderPipe() { _ = signal(SIGPIPE, SIG_IGN) }
    func useSender(_ lifecycle: CaptureLifecycle) {
        lock.lock(); defer { lock.unlock() }; self.lifecycle = lifecycle
    }
    func packet(_ bytes: Data, measure: Bool) -> Result {
        lock.lock()
        let owner = lifecycle
        guard owner?.isClosing != true else { lock.unlock(); return .suppressed }
        let acquired = measure ? DispatchTime.now().uptimeNanoseconds : 0
        do {
            try write(bytes)
            let finished = measure ? DispatchTime.now().uptimeNanoseconds : 0
            lock.unlock(); return .written(acquired: acquired, finished: finished)
        } catch {
            lock.unlock()
            if let owner { owner.requestStop(.stdoutFailure, error: String(describing: error)) }
            else { legacyFailure(error) }
            return .failed
        }
    }
}
