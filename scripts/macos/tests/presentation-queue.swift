final class OwnedFrame {
    let identifier: Int
    init(_ identifier: Int) { self.identifier = identifier }
}

@main struct QueueChecks {
    static func main() {
        var queue = PresentationQueue<Int>()
        queue.append(7, timestamp: 1, arrival: 100)
        precondition(queue.take(now: 99, delay: 10) == nil)
        precondition(queue.take(now: 109, delay: 10) == nil)
        // A single static frame still appears without requiring a second one.
        precondition(queue.take(now: 110, delay: 10)?.0 == 7)
        precondition(queue.take(now: 111, delay: 10) == nil)
        queue.append(8, timestamp: 1, arrival: 120) // duplicate
        queue.append(9, timestamp: 0, arrival: 120) // stale
        precondition(queue.count == 0)
        for value in 2...10 { queue.append(value, timestamp: UInt64(value), arrival: 120) }
        precondition(queue.count == 3)
        precondition(queue.take(now: 130, delay: 10)?.0 == 8)
        precondition(queue.take(now: 130, delay: 10)?.0 == 9)
        precondition(queue.take(now: 130, delay: 10)?.0 == 10)
        var owned = PresentationQueue<OwnedFrame>()
        weak var first: OwnedFrame?
        do {
            let frame = OwnedFrame(1); first = frame
            owned.append(frame, timestamp: 1, arrival: 0)
        }
        precondition(first != nil)
        for value in 2...4 { owned.append(OwnedFrame(value), timestamp: UInt64(value), arrival: 0) }
        precondition(first == nil, "Dropped buffers must release ownership")
        print("Presentation queue: static frame, delay, ordering, burst bound and ownership passed")
    }
}
