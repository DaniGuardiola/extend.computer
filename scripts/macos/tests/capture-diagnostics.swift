@main struct CaptureDiagnosticChecks {
    static func main() {
        var disabled = CaptureDiagnostics(enabled: false)
        for _ in 0..<1024 {
            for stage in CaptureDiagnostics.Stage.allCases { precondition(disabled.record(stage) == nil) }
        }
        var enabled = CaptureDiagnostics(enabled: true)
        var emitted = [UInt64]()
        for _ in 1...1024 {
            if let count = enabled.record(.callback) { emitted.append(count) }
        }
        precondition(emitted == [1, 2, 3, 4, 8, 16, 32, 64, 128, 256, 512, 1024])
        // Each rejection has its own cumulative count; another busy channel
        // cannot mask the first attachment/status failure.
        for stage in CaptureDiagnostics.Stage.allCases where stage != .callback {
            precondition(enabled.record(stage) == 1)
        }
        let names = CaptureDiagnostics.Stage.allCases.map { $0.metric }
        precondition(Set(names).count == names.count)
        precondition(names.allSatisfy { $0.hasPrefix("capture_") && $0.hasSuffix("_count") })
        print("Capture diagnostics: disabled path, bounded logarithmic progress and independent drop reasons passed")
    }
}
