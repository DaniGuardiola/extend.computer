import Foundation

@main struct CaptureStartupChecks {
    static func main() {
        let earlyImage = CaptureStartupDeadline(timeoutNanoseconds: 100)
        precondition(!earlyImage.acceptedImage(at: 1))
        precondition(earlyImage.started(at: 10) == nil)
        precondition(!earlyImage.timeout(at: 1000))

        let onTime = CaptureStartupDeadline(timeoutNanoseconds: 100)
        precondition(onTime.started(at: 10) == 110)
        precondition(onTime.started(at: 90) == nil, "Another start cannot extend the fixed deadline")
        precondition(!onTime.timeout(at: 109))
        precondition(!onTime.acceptedImage(at: 109))
        precondition(!onTime.timeout(at: 110))

        let expired = CaptureStartupDeadline(timeoutNanoseconds: 100)
        precondition(expired.started(at: 10) == 110)
        precondition(expired.timeout(at: 110))
        precondition(!expired.timeout(at: 111))
        precondition(!expired.acceptedImage(at: 111))
        precondition(!expired.cancel(), "A later Stop cannot undo committed expiration")
        let lateImage = CaptureStartupDeadline(timeoutNanoseconds: 100)
        precondition(lateImage.started(at: 10) == 110)
        precondition(lateImage.acceptedImage(at: 110), "Late delivery cannot bypass the deadline if the timer is delayed")
        precondition(!lateImage.timeout(at: 111))

        for beforeStart in [false, true] {
            let stopped = CaptureStartupDeadline(timeoutNanoseconds: 100)
            if !beforeStart { precondition(stopped.started(at: 10) == 110) }
            precondition(stopped.cancel())
            precondition(stopped.started(at: 20) == nil)
            precondition(!stopped.acceptedImage(at: 30))
            precondition(!stopped.timeout(at: 1000), "Committed Stop must suppress timeout")
        }

        // Concurrent timer/image/Stop calls have one linearized winner. If
        // Stop won first, no expiration caller may subsequently win.
        for _ in 0..<128 {
            let race = CaptureStartupDeadline(timeoutNanoseconds: 100)
            precondition(race.started(at: 0) == 100)
            let resultsLock = NSLock()
            var failures = 0
            var stopWon = false
            DispatchQueue.concurrentPerform(iterations: 3) { index in
                let failed = index == 0 ? race.timeout(at: 100) : index == 1 ? race.acceptedImage(at: 100) : false
                let stopped = index == 2 && race.cancel()
                resultsLock.lock(); failures += failed ? 1 : 0; stopWon = stopWon || stopped; resultsLock.unlock()
            }
            precondition(failures <= 1)
            precondition(!stopWon || failures == 0)
        }
        print("Capture startup: early image, fixed deadline, late delivery, Stop and concurrent one-shot races passed")
    }
}
