// Optional numeric telemetry only. The capture queue owns these counters.
// Disabled sessions neither increment counters nor emit diagnostic events.
struct CaptureDiagnostics {
    enum Stage: Int, CaseIterable {
        case callback, configuring, wrongType, invalidSample, sampleValid
        case noAttachments, attachmentCast, emptyAttachments, noStatus, statusCast, noImage

        var metric: String {
            switch self {
            case .callback: return "capture_callback_count"
            case .configuring: return "capture_drop_configuring_count"
            case .wrongType: return "capture_drop_type_count"
            case .invalidSample: return "capture_drop_invalid_sample_count"
            case .sampleValid: return "capture_sample_valid_count"
            case .noAttachments: return "capture_drop_no_attachments_count"
            case .attachmentCast: return "capture_drop_attachment_cast_count"
            case .emptyAttachments: return "capture_drop_empty_attachments_count"
            case .noStatus: return "capture_drop_no_status_count"
            case .statusCast: return "capture_drop_status_cast_count"
            case .noImage: return "capture_drop_no_image_count"
            }
        }
    }

    private let enabled: Bool
    private var counts = [UInt64](repeating: 0, count: Stage.allCases.count)
    init(enabled: Bool) { self.enabled = enabled }

    mutating func record(_ stage: Stage) -> UInt64? {
        guard enabled, counts[stage.rawValue] != .max else { return nil }
        counts[stage.rawValue] += 1
        let count = counts[stage.rawValue]
        // Show the first few callbacks, then logarithmically bounded progress.
        return count <= 4 || count.nonzeroBitCount == 1 ? count : nil
    }
}
