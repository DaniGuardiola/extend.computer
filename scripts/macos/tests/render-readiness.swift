import Foundation

@main struct RenderReadinessChecks {
    static func main() {
        let readiness = RenderReadiness()
        let countLock = NSLock()
        var completions = 0
        readiness.callback = {
            countLock.lock(); completions += 1; countLock.unlock()
            // The callback must run outside the latch lock.
            precondition(!readiness.complete(success: true))
        }
        precondition(!readiness.complete(success: false))
        precondition(completions == 0, "Failed GPU work cannot establish readiness")
        precondition(readiness.complete(success: true))
        precondition(!readiness.complete(success: false))
        precondition(!readiness.complete(success: true))
        precondition(completions == 1, "Static-frame redraws cannot duplicate readiness")

        let concurrent = RenderReadiness()
        var concurrentCompletions = 0
        concurrent.callback = { countLock.lock(); concurrentCompletions += 1; countLock.unlock() }
        DispatchQueue.concurrentPerform(iterations: 128) { index in
            concurrent.complete(success: index % 3 != 0)
        }
        precondition(concurrentCompletions == 1, "Concurrent Metal completions must emit once")
        print("Render readiness: failed GPU work, one-shot static frame, reentrancy and concurrent completion passed")
    }
}
