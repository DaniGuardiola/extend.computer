import Foundation

// Independent baseline retained for differential malformed-input and lifetime
// checks. This is the project's previous parser, not a reference product.
func baselineNALs(_ data: Data) throws -> [Data] {
    let bytes = [UInt8](data)
    guard bytes.count <= 16 * 1024 * 1024 else { throw DisplayFailure("Video frame too large.") }
    var starts: [(Int, Int)] = []; var i = 0
    while i + 3 <= bytes.count {
        if bytes[i] == 0 && bytes[i+1] == 0 {
            if bytes[i+2] == 1 { starts.append((i, i+3)); i += 3; continue }
            if i+3 < bytes.count && bytes[i+2] == 0 && bytes[i+3] == 1 { starts.append((i, i+4)); i += 4; continue }
        }
        i += 1
    }
    guard starts.first?.0 == 0 else { throw DisplayFailure("Invalid video access unit.") }
    return try starts.enumerated().map { index, start in
        let end = index+1 < starts.count ? starts[index+1].0 : bytes.count
        guard end > start.1 else { throw DisplayFailure("Empty video NAL.") }
        return Data(bytes[start.1..<end])
    }
}

enum Outcome: Equatable {
    case nals([Data]), failure(String)
}
func outcome(_ data: Data, _ parse: (Data) throws -> [Data]) -> Outcome {
    do { return .nals(try parse(data)) }
    catch { return .failure(String(describing: error)) }
}
func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    if !condition() { throw DisplayFailure(message) }
}

private enum CreationFailure: Error { case unavailable, configuration }

// Injected session creation failures exercise the production policy without
// allocating a hardware encoder or capturing/reading a screen.
func lowLatencyCreationChecks() throws {
    for value in [nil, "0"] as [String?] {
        var attempts: [Bool] = []
        let selected = try EncoderLowLatencyPolicy(environment: value).create { requested in
            attempts.append(requested); return "standard hardware"
        }
        try require(attempts == [false] && !selected.lowLatency && !selected.fallback, "Standard policy requested low latency")
    }
    var attempts: [Bool] = []
    let preferred = EncoderLowLatencyPolicy(environment: "auto")
    let supported = try preferred.create { requested in
        attempts.append(requested); return "low latency hardware"
    }
    try require(attempts == [true] && supported.lowLatency && !supported.fallback, "Supported preference retried")
    attempts = []
    let fallback = try preferred.create { requested in
        attempts.append(requested)
        if requested { throw CreationFailure.unavailable }
        return "standard hardware"
    }
    try require(attempts == [true, false] && !fallback.lowLatency && fallback.fallback, "Unavailable preference did not retry standard hardware once")
    attempts = []
    do {
        let _: (session: String, lowLatency: Bool, fallback: Bool) = try EncoderLowLatencyPolicy(environment: "1").create { requested in
            attempts.append(requested); throw CreationFailure.unavailable
        }
        throw DisplayFailure("Explicit low latency failure was swallowed")
    } catch CreationFailure.unavailable {
        try require(attempts == [true], "Explicit low latency retried another mode")
    }
    attempts = []
    do {
        let _: (session: String, lowLatency: Bool, fallback: Bool) = try preferred.create { requested in
            attempts.append(requested); throw CreationFailure.unavailable
        }
        throw DisplayFailure("Both hardware creation failures were swallowed")
    } catch CreationFailure.unavailable {
        try require(attempts == [true, false], "Automatic hardware failure retried indefinitely")
    }
    attempts = []
    do {
        let _ = try preferred.create { requested in
            attempts.append(requested); return "low latency hardware"
        }
        throw CreationFailure.configuration
    } catch CreationFailure.configuration {
        try require(attempts == [true], "Configuration failure silently changed encoder mode")
    }
}

@main struct DisplayCodecChecks {
    static func main() throws {
        try lowLatencyCreationChecks()
        let edgeCases: [[UInt8]] = [[], [0], [0,0], [0,0,1], [0,0,0,1],
            [9,0,0,1,4], [0,0,1,0,0,1], [0,0,0,1,0,0,1,4],
            [0,0,1,4,0,0], [0,0,0,1,4,0,0,1,5], [0,0,1,4,0,0,0,1,5]]
        for bytes in edgeCases {
            let data = Data(bytes)
            try require(outcome(data, annexBNALs) == outcome(data, baselineNALs), "Edge-case parser behavior changed")
        }
        var seed: UInt64 = 0x63b1792
        func next() -> UInt64 {
            seed = seed &* 6364136223846793005 &+ 1442695040888963407
            return seed
        }
        for index in 0..<2_000 {
            var bytes = (0..<Int(next() % 512)).map { _ in UInt8(truncatingIfNeeded: next() >> 32) }
            if index % 2 == 0 { bytes.insert(contentsOf: [0,0,1], at: 0) }
            if bytes.count > 12 && index % 3 == 0 {
                bytes.replaceSubrange(8..<12, with: [0,0,0,1])
            }
            let storage = Data([99,88] + bytes + [77])
            let slice = storage[2..<storage.count-1]
            try require(outcome(slice, annexBNALs) == outcome(slice, baselineNALs), "Differential or sliced-storage mismatch \(index)")
        }
        var source = Data([0,0,0,1,17,18,19,0,0,1,20,21])
        let retained = try annexBNALs(source)
        source.resetBytes(in: 0..<source.count)
        try require(retained == [Data([17,18,19]),Data([20,21])], "Returned NAL storage does not survive input mutation")
        let oversized = Data(count: 16 * 1024 * 1024 + 1)
        try require(outcome(oversized, annexBNALs) == .failure("Video frame too large."), "Size limit not enforced")

        var cases: [[String: Any]] = []
        var consumed = 0
        for size in [48 * 1024, 256 * 1024, 1024 * 1024] {
            var data = Data(repeating: 85, count: size)
            data.replaceSubrange(0..<4, with: [0,0,0,1])
            let repetitions = max(20, 16 * 1024 * 1024 / size)
            func measure(_ parse: (Data) throws -> [Data]) throws -> Double {
                let start = DispatchTime.now().uptimeNanoseconds
                for _ in 0..<repetitions { consumed &+= try parse(data).reduce(0) { $0 + $1.count } }
                return Double(DispatchTime.now().uptimeNanoseconds - start) / Double(repetitions) / 1_000
            }
            _ = try measure(annexBNALs); _ = try measure(baselineNALs)
            var old: [Double] = [], current: [Double] = []
            for round in 0..<7 {
                if round % 2 == 0 { old.append(try measure(baselineNALs)); current.append(try measure(annexBNALs)) }
                else { current.append(try measure(annexBNALs)); old.append(try measure(baselineNALs)) }
            }
            let baseline = old.sorted()[3], optimized = current.sorted()[3]
            cases.append(["access_unit_bytes":size,"repetitions_per_round":repetitions,
                "baseline_median_us":baseline,"current_median_us":optimized,
                "speedup":baseline / optimized,"baseline_rounds_us":old,"current_rounds_us":current])
        }
        let report: [String: Any] = ["status":"passed","differential_cases":2_000 + edgeCases.count,
            "ownership_and_size_checks":true,"low_latency_creation_checks":true,"microbenchmarks":cases,"consumed_bytes":consumed,
            "limits":["Synthetic Annex B access units; not end-to-end decoder cost or product CPU savings.",
                      "Seven alternating warmed rounds; current machine and compiler determine timing."]]
        print(String(data: try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted,.sortedKeys]), encoding: .utf8)!)
    }
}
