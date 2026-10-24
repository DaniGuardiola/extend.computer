// A bounded, main-thread-owned presentation queue. Arrival time belongs to
// the receiver clock; source timestamps only reject duplicate/older frames.
struct PresentationQueue<Value> {
    private struct Entry {
        let value: Value
        let timestamp: UInt64
        let arrival: UInt64
    }
    private var entries: [Entry] = []
    private var newest: UInt64?
    var count: Int { entries.count }

    mutating func append(_ value: Value, timestamp: UInt64, arrival: UInt64) {
        guard newest.map({ timestamp > $0 }) ?? true else { return }
        newest = timestamp
        entries.append(Entry(value: value, timestamp: timestamp, arrival: arrival))
        // Retain at most three decoded buffers even during a burst. Replacing
        // the oldest queued frame bounds latency instead of accumulating debt.
        if entries.count > 3 { entries.removeFirst(entries.count - 3) }
    }

    mutating func take(now: UInt64, delay: UInt64) -> (Value, UInt64)? {
        guard let first = entries.first, now >= first.arrival,
              now - first.arrival >= delay else { return nil }
        entries.removeFirst()
        return (first.value, first.timestamp)
    }
}
