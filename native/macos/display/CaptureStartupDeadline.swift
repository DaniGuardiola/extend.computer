import Foundation

// Stream startup is not evidence that ScreenCaptureKit has supplied an image.
// This one-shot gate uses host uptime nanoseconds, independent of wall time.
// Every mutable field is private and accessed under lock. The timer may share
// this gate without sharing DisplayCapture's queue-confined native state.
final class CaptureStartupDeadline: @unchecked Sendable {
    static let failureMessage = "macOS started screen capture but did not provide video frames within 8 seconds. Check the screen-access dialog on this Mac, choose Allow if shown, then reconnect."
    private enum State {
        case awaitingStart, awaitingImage(UInt64), imageReceived, stopped, expired
    }
    private let lock = NSLock()
    private let timeoutNanoseconds: UInt64
    private var state: State = .awaitingStart

    init(timeoutNanoseconds: UInt64 = 8_000_000_000) {
        self.timeoutNanoseconds = timeoutNanoseconds
    }

    // A callback may precede startCapture's completion. Remember that image;
    // never arm a timeout for an already working or stopped stream.
    func started(at now: UInt64) -> UInt64? {
        lock.lock(); defer { lock.unlock() }
        guard case .awaitingStart = state else { return nil }
        let (deadline, overflow) = now.addingReportingOverflow(timeoutNanoseconds)
        let bounded = overflow ? UInt64.max : deadline
        state = .awaitingImage(bounded)
        return bounded
    }

    // Return true only to the single winner that must report expiration.
    func acceptedImage(at now: UInt64) -> Bool {
        lock.lock(); defer { lock.unlock() }
        switch state {
        case .awaitingStart:
            state = .imageReceived
        case .awaitingImage(let deadline):
            if now >= deadline { state = .expired; return true }
            state = .imageReceived
        case .imageReceived, .stopped, .expired:
            break
        }
        return false
    }

    func timeout(at now: UInt64) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard case .awaitingImage(let deadline) = state, now >= deadline else { return false }
        state = .expired
        return true
    }

    // Stop and expiration are serialized. A committed Stop prevents failure;
    // Stop cannot revoke an expiration that has already won the race.
    @discardableResult
    func cancel() -> Bool {
        lock.lock(); defer { lock.unlock() }
        if case .expired = state { return false }
        state = .stopped
        return true
    }
}
