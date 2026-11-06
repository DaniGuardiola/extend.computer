import Foundation
import CoreMedia
import CoreVideo
import VideoToolbox
import CoreImage

func reportPixelColor(_ image: CVPixelBuffer, stage: String) {
    guard ProcessInfo.processInfo.environment["EXTEND_DISPLAY_METRICS"] != nil else { return }
    let tags = CVBufferCopyAttachments(image, .shouldPropagate) as? [String: Any] ?? [:]
    DisplayMetrics.shared.event(stage + "_pixel_format", bytes: Int(CVPixelBufferGetPixelFormatType(image)))
    for (suffix, key, expected) in [
        ("transfer_srgb", kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_sRGB),
        ("transfer_709", kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2),
        ("primaries_709", kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2)
    ] {
        DisplayMetrics.shared.event(stage + "_" + suffix,
            bytes: (tags[key as String] as? String) == expected as String ? 1 : 0)
    }

    var values: [String: String] = ["stage": stage,
        "pixel_format": String(CVPixelBufferGetPixelFormatType(image)),
        "ci_color_space": String(describing: CIImage(cvPixelBuffer: image).colorSpace?.name)]
    for key in [kCVImageBufferColorPrimariesKey, kCVImageBufferTransferFunctionKey, kCVImageBufferYCbCrMatrixKey] {
        values[key as String] = tags[key as String].map { String(describing: $0) } ?? "absent"
    }
    if let data = try? JSONSerialization.data(withJSONObject: values, options: [.sortedKeys]) {
        FileHandle.standardError.write(Data("Display pixel color: ".utf8) + data + Data("\n".utf8))
    }
}

// Automatic preference may retry session creation with the standard hardware
// encoder. Explicit "1" is strict so diagnostic comparisons never silently
// switch modes; configuration/prepare failures are never swallowed.
enum EncoderLowLatencyPolicy: Equatable {
    case standard, required, preferred

    init(environment: String?) {
        switch environment {
        case "1": self = .required
        case "auto": self = .preferred
        default: self = .standard
        }
    }

    var requested: Bool { self != .standard }

    func create<Session>(_ attempt: (Bool) throws -> Session) throws
        -> (session: Session, lowLatency: Bool, fallback: Bool) {
        do {
            return (try attempt(requested), requested, false)
        } catch {
            guard self == .preferred else { throw error }
            return (try attempt(false), false, true)
        }
    }
}

// Experimental controls remain unset unless explicitly requested. Query support
// first, and verify the driver's value after preparation; accepting a request
// without applying it would invalidate a comparison.
private struct EncoderControl {
    let key: CFString
    let stage: String
    let requested: NSNumber?
}
private func configureEncoderControls(_ session: VTCompressionSession) throws -> [EncoderControl] {
    let environment = ProcessInfo.processInfo.environment
    let options: [(String, CFString, String)] = [
        ("EXTEND_DISPLAY_ENCODER_MAX_FRAME_DELAY", kVTCompressionPropertyKey_MaxFrameDelayCount, "encoder_max_frame_delay"),
        ("EXTEND_DISPLAY_ENCODER_PRIORITIZE_SPEED", kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality, "encoder_prioritize_speed")
    ]
    let controls = try options.map { name, key, stage -> EncoderControl in
        if let value = environment[name], value != "0" && value != "1" {
            throw DisplayFailure("Invalid experimental encoder option \(name); use 0 or 1.")
        }
        let requested: NSNumber? = environment[name] == "1"
            ? (key == kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality ? NSNumber(value: true) : NSNumber(value: 1)) : nil
        return EncoderControl(key: key, stage: stage, requested: requested)
    }
    guard controls.contains(where: { $0.requested != nil }) || environment["EXTEND_DISPLAY_METRICS"] != nil else { return [] }
    var dictionary: CFDictionary?
    let status = VTSessionCopySupportedPropertyDictionary(session, supportedPropertyDictionaryOut: &dictionary)
    let supported = dictionary as? [String: Any] ?? [:]
    DisplayMetrics.shared.event("encoder_property_dictionary_status", bytes: Int(status))
    for control in controls {
        let metadata = supported[control.key as String] as? [String: Any]
        let readWrite = metadata?[kVTPropertyReadWriteStatusKey as String] as? String
        let writable = readWrite == kVTPropertyReadWriteStatus_ReadWrite as String ? 1
            : readWrite == kVTPropertyReadWriteStatus_ReadOnly as String ? 0 : -1
        DisplayMetrics.shared.event(control.stage + "_supported", bytes: metadata == nil ? 0 : 1)
        DisplayMetrics.shared.event(control.stage + "_writable", bytes: writable)
        DisplayMetrics.shared.event(control.stage + "_requested", bytes: control.requested == nil ? 0 : 1)
        guard let requested = control.requested else { continue }
        guard status == noErr, metadata != nil, writable != 0 else {
            DisplayMetrics.shared.flush()
            throw DisplayFailure("Experimental encoder property \(control.key) is unavailable or read-only.")
        }
        let setStatus = VTSessionSetProperty(session, key: control.key, value: requested)
        DisplayMetrics.shared.event(control.stage + "_set_status", bytes: Int(setStatus))
        DisplayMetrics.shared.flush()
        try check(setStatus, "Set experimental encoder property \(control.key)")
    }
    return controls
}
private func verifyEncoderControls(_ session: VTCompressionSession, _ controls: [EncoderControl]) throws {
    for control in controls {
        var value: Unmanaged<CFTypeRef>?
        let status = VTSessionCopyProperty(session, key: control.key, allocator: nil, valueOut: &value)
        let actual = value?.takeRetainedValue() as? NSNumber
        DisplayMetrics.shared.event(control.stage + "_read_status", bytes: Int(status))
        // A missing value has its own validity event; -1 is a valid unlimited delay.
        DisplayMetrics.shared.event(control.stage + "_value_valid", bytes: actual == nil ? 0 : 1)
        if let actual { DisplayMetrics.shared.event(control.stage + "_value", bytes: actual.intValue) }
        if let requested = control.requested {
            guard status == noErr, let actual, actual.doubleValue == requested.doubleValue else {
                DisplayMetrics.shared.flush()
                throw DisplayFailure("Experimental encoder property \(control.key) was not confirmed by driver readback.")
            }
        }
    }
    DisplayMetrics.shared.flush()
}

final class VideoEncoder {
    private let codec: VideoCodec
    private var session: VTCompressionSession?
    private let inFlight: DispatchSemaphore
    private let lock = NSLock()
    private let submissionLock = NSLock()
    private var lastSubmittedTime: CMTime?
    private var forceKeyframe = true
    private var paused = false
    private var latest: CVPixelBuffer?
    private let output: (VideoFrame) -> Void
    private let failure: (Error) -> Void
    private let lowLatencyPolicy: EncoderLowLatencyPolicy
    init(width: Int, height: Int, fps: Int, bitrate: Int, codec: VideoCodec = .h264,
         lowLatencyPolicy: EncoderLowLatencyPolicy? = nil,
         output: @escaping (VideoFrame) -> Void, failure: @escaping (Error) -> Void) throws {
        self.codec = codec; self.output = output; self.failure = failure
        self.lowLatencyPolicy = lowLatencyPolicy ?? EncoderLowLatencyPolicy(
            environment: ProcessInfo.processInfo.environment["EXTEND_DISPLAY_LOW_LATENCY_ENCODER"])
        // Compare a slightly wider bounded pipeline on hardware whose callback
        // latency exceeds two refresh intervals. Keep the production limit until
        // physical latency and smoothness measurements justify changing it.
        let slots = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_ENCODER_SLOTS"] == "3" ? 3 : 2
        inFlight = DispatchSemaphore(value: slots)
        DisplayMetrics.shared.event("config_encoder_slots", bytes: slots)
        DisplayMetrics.shared.event("encoder_low_latency_requested", bytes: self.lowLatencyPolicy.requested ? 1 : 0)
        DisplayMetrics.shared.event("encoder_low_latency_auto", bytes: self.lowLatencyPolicy == .preferred ? 1 : 0)
        let created = try self.lowLatencyPolicy.create { lowLatency -> VTCompressionSession in
            var specification: [CFString: Any] = [
                kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true,
            ]
            if lowLatency { specification[kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = true }
            var candidate: VTCompressionSession?
            let status = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height),
            codecType: codec.type, encoderSpecification: specification as CFDictionary,
            imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: { context, _, status, flags, sample in
                guard let context else { return }
                let encoder = Unmanaged<VideoEncoder>.fromOpaque(context).takeUnretainedValue()
                defer { encoder.inFlight.signal() }
                if status != noErr { encoder.failure(DisplayFailure("Hardware encoding failed (\(status)).")); return }
                guard !flags.contains(.frameDropped), let sample, CMSampleBufferDataIsReady(sample) else {
                    DisplayMetrics.shared.event("encoder_dropped"); return
                }
                do {
                    let frame = try encoder.frame(sample)
                    DisplayMetrics.shared.event("encoded", timestamp: frame.timestamp, bytes: frame.bytes.count)
                    encoder.output(frame)
                } catch { encoder.failure(error) }
            }, refcon: Unmanaged.passUnretained(self).toOpaque(), compressionSessionOut: &candidate)
            DisplayMetrics.shared.event(lowLatency ? "encoder_low_latency_create_status" : "encoder_standard_create_status", bytes: Int(status))
            if status != noErr {
                // A failed create may still populate its output. Release that
                // hardware reservation before the single automatic retry.
                if let candidate { VTCompressionSessionInvalidate(candidate) }
                try check(status, "Create hardware encoder")
            }
            guard let candidate else { throw DisplayFailure("Hardware encoder unavailable.") }
            return candidate
        }
        session = created.session
        let lowLatency = created.lowLatency
        DisplayMetrics.shared.event("encoder_low_latency_fallback", bytes: created.fallback ? 1 : 0)
        if self.lowLatencyPolicy == .preferred {
            FileHandle.standardError.write(Data("Display encoder: automatic low latency requested; applied=\(lowLatency ? 1 : 0) fallback=\(created.fallback ? 1 : 0) hardware_required=1\n".utf8))
        }
        guard let session else { throw DisplayFailure("Hardware encoder unavailable.") }
        for (key, value) in [
            (kVTCompressionPropertyKey_RealTime, true as CFTypeRef),
            (kVTCompressionPropertyKey_AllowFrameReordering, false as CFTypeRef),
            (kVTCompressionPropertyKey_ProfileLevel, (codec == .hevc ? kVTProfileLevel_HEVC_Main_AutoLevel : (lowLatency ? kVTProfileLevel_H264_High_AutoLevel : kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel))),
            (kVTCompressionPropertyKey_AverageBitRate, bitrate as CFTypeRef),
            (kVTCompressionPropertyKey_ExpectedFrameRate, fps as CFTypeRef),
            (kVTCompressionPropertyKey_ColorPrimaries, kCVImageBufferColorPrimaries_ITU_R_709_2),
            (kVTCompressionPropertyKey_TransferFunction, kCVImageBufferTransferFunction_sRGB),
            (kVTCompressionPropertyKey_YCbCrMatrix, kCVImageBufferYCbCrMatrix_ITU_R_709_2),
            (kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, 2 as CFTypeRef),
        ] {
            if key == kVTCompressionPropertyKey_AverageBitRate {
                DisplayMetrics.shared.event("bitrate_attempted", bytes: bitrate)
            }
            let status = VTSessionSetProperty(session, key: key, value: value)
            if key == kVTCompressionPropertyKey_AverageBitRate {
                DisplayMetrics.shared.event("bitrate_set_status", bytes: Int(status))
            }
            try check(status, "Configure hardware encoder \(key)")
            if key == kVTCompressionPropertyKey_AverageBitRate {
                DisplayMetrics.shared.event("bitrate", bytes: bitrate)
            }
        }
        DisplayMetrics.shared.event("encoder_low_latency", bytes: lowLatency ? 1 : 0)
        let controls = try configureEncoderControls(session)
        try check(VTCompressionSessionPrepareToEncodeFrames(session), "Prepare hardware encoder")
        try verifyEncoderControls(session, controls)
        try verifyBitrate(bitrate)
        var hardware: Unmanaged<CFTypeRef>?
        if VTSessionCopyProperty(session, key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
            allocator: nil, valueOut: &hardware) == noErr, let active = hardware?.takeRetainedValue() as? NSNumber {
            DisplayMetrics.shared.event("encoder_hardware", bytes: active.boolValue ? 1 : 0)
        }
    }
    func requestKeyframe() {
        lock.lock(); forceKeyframe = true; let image = latest; lock.unlock()
        // An unchanged desktop may produce only idle capture callbacks. Recovery
        // must not wait for the user to move a window before sending a keyframe.
        if let image { try? encode(image, timestamp: CMClockGetTime(CMClockGetHostTimeClock())) }
    }
    func pauseCapture(_ value: Bool) {
        lock.lock(); let resumed = paused && !value; paused = value; let image = latest; lock.unlock()
        DisplayMetrics.shared.event(value ? "capture_paused" : "capture_resumed")
        // The next encoded frame references the last frame actually sent, so
        // skipped captures do not require another expensive keyframe.
        if resumed, let image { try? encode(image, timestamp: CMClockGetTime(CMClockGetHostTimeClock())) }
    }
    func setBitrate(_ bitrate: Int) throws {
        guard let session else { return }
        let requested = min(80_000_000, max(1_000_000, bitrate))
        DisplayMetrics.shared.event("bitrate_attempted", bytes: requested)
        let status = VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AverageBitRate,
            value: requested as CFTypeRef)
        DisplayMetrics.shared.event("bitrate_set_status", bytes: Int(status))
        do {
            try check(status, "Update bitrate")
            DisplayMetrics.shared.event("bitrate", bytes: requested)
            try verifyBitrate(requested)
        } catch {
            // Source controls historically ignore setter errors. A fixed-rate
            // experiment must terminate instead of measuring an unverified rate.
            if ProcessInfo.processInfo.environment["EXTEND_DISPLAY_FIXED_BITRATE"] == "1" {
                failure(error)
            }
            throw error
        }
    }
    private func verifyBitrate(_ requested: Int) throws {
        let environment = ProcessInfo.processInfo.environment
        let strict = environment["EXTEND_DISPLAY_FIXED_BITRATE"] == "1"
        guard strict || environment["EXTEND_DISPLAY_METRICS"] != nil, let session else { return }
        var value: Unmanaged<CFTypeRef>?
        let status = VTSessionCopyProperty(session, key: kVTCompressionPropertyKey_AverageBitRate,
            allocator: nil, valueOut: &value)
        let actual = value?.takeRetainedValue() as? NSNumber
        DisplayMetrics.shared.event("encoder_bitrate_read_status", bytes: Int(status))
        DisplayMetrics.shared.event("encoder_bitrate_value_valid", bytes: actual == nil ? 0 : 1)
        if let actual { DisplayMetrics.shared.event("encoder_bitrate_value", bytes: actual.intValue) }
        DisplayMetrics.shared.flush()
        if strict {
            guard status == noErr, let actual, actual.int64Value == Int64(requested) else {
                throw DisplayFailure("Fixed encoder bitrate was not confirmed by driver readback.")
            }
        }
    }
    func replacement(width: Int, height: Int, fps: Int, bitrate: Int) throws -> VideoEncoder {
        try VideoEncoder(width: width, height: height, fps: fps, bitrate: bitrate, codec: codec,
                         lowLatencyPolicy: lowLatencyPolicy,
                         output: output, failure: failure)
    }
    func setFrameRate(_ fps: Int) throws {
        guard let session else { return }
        try check(VTSessionSetProperty(session, key: kVTCompressionPropertyKey_ExpectedFrameRate,
            value: fps as CFTypeRef), "Update encoder frame rate")
        DisplayMetrics.shared.event("requested_fps", bytes: fps)
    }
    func encode(_ image: CVPixelBuffer, timestamp: CMTime) throws {
        lock.lock(); latest = image; let paused = paused; lock.unlock()
        if paused { DisplayMetrics.shared.event("capture_skipped"); return }
        // A bounded pipeline keeps hardware encoding busy without
        // accumulating old desktop frames. Capture and recovery can submit from
        // different threads; serialize submissions and keep timestamps ordered.
        submissionLock.lock(); defer { submissionLock.unlock() }
        if let lastSubmittedTime, CMTimeCompare(timestamp, lastSubmittedTime) <= 0 {
            DisplayMetrics.shared.event("encode_stale"); return
        }
        guard inFlight.wait(timeout: .now()) == .success else {
            DisplayMetrics.shared.event("encode_busy"); return
        }
        guard let session else { inFlight.signal(); return }
        lock.lock(); let keyframe = forceKeyframe; forceKeyframe = false; lock.unlock()
        let properties = keyframe ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let status = VTCompressionSessionEncodeFrame(session, imageBuffer: image,
            presentationTimeStamp: timestamp, duration: .invalid, frameProperties: properties,
            sourceFrameRefcon: nil, infoFlagsOut: nil)
        if status != noErr { inFlight.signal(); throw DisplayFailure("Submit video frame failed (\(status)).") }
        lastSubmittedTime = timestamp
        DisplayMetrics.shared.event("submitted", timestamp: UInt64(max(0, CMTimeGetSeconds(timestamp)) * 1e9))
    }
    func drain() { if let session { VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid) }; DisplayMetrics.shared.flush() }
    private func frame(_ sample: CMSampleBuffer) throws -> VideoFrame {
        guard let format = CMSampleBufferGetFormatDescription(sample), let block = CMSampleBufferGetDataBuffer(sample) else {
            throw DisplayFailure("Encoder returned incomplete frame.")
        }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let keyframe = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
        var bytes = Data()
        if keyframe {
            for index in 0..<(codec == .hevc ? 3 : 2) {
                var pointer: UnsafePointer<UInt8>?; var length = 0; var count = 0; var header: Int32 = 0
                let status = codec == .hevc
                    ? CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: index,
                        parameterSetPointerOut: &pointer, parameterSetSizeOut: &length, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &header)
                    : CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: index,
                        parameterSetPointerOut: &pointer, parameterSetSizeOut: &length, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &header)
                try check(status, "Read video parameter sets")
                guard header == 4, let pointer else { throw DisplayFailure("Unsupported video framing.") }
                bytes.append(contentsOf: [0, 0, 0, 1]); bytes.append(pointer, count: length)
            }
        }
        let length = CMBlockBufferGetDataLength(block)
        guard length <= 16 * 1024 * 1024 else { throw DisplayFailure("Encoded frame too large.") }
        var avcc = Data(count: length)
        try avcc.withUnsafeMutableBytes { buffer in
            try check(CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: buffer.baseAddress!), "Read encoded frame")
        }
        var position = 0
        while position < avcc.count {
            guard position + 4 <= avcc.count else { throw DisplayFailure("Truncated video length.") }
            let count = avcc[position..<position+4].reduce(0) { ($0 << 8) | Int($1) }; position += 4
            guard count > 0, position + count <= avcc.count else { throw DisplayFailure("Invalid video length.") }
            bytes.append(contentsOf: [0, 0, 0, 1]); bytes.append(avcc[position..<position+count]); position += count
        }
        let time = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample))
        return VideoFrame(timestamp: UInt64(max(0, time) * 1_000_000_000), keyframe: keyframe, bytes: bytes)
    }
    deinit { drain(); if let session { VTCompressionSessionInvalidate(session) } }
}


// The decoder itself lives in native/apple/shared/VideoDecoding.swift. The
// helper keeps its telemetry: metrics, first-frame color report and marker.
extension VideoDecoder {
    convenience init(codec: VideoCodec = .h264, benchmarkMarker: BenchmarkMarker?,
                     output: @escaping (CVPixelBuffer, UInt64) -> Void, failure: @escaping () -> Void = {}) {
        self.init(codec: codec, metric: { DisplayMetrics.shared.event($0, timestamp: $1, bytes: $2) },
            output: output, failure: failure)
        setFirstOutput { reportPixelColor($0, stage: "decoded") }
        if let benchmarkMarker { observeMarker(benchmarkMarker) }
    }
    func configureMarker(width: Int, height: Int) {
        observeMarker(BenchmarkMarker(logicalWidth: width, logicalHeight: height))
    }
    private func observeMarker(_ marker: BenchmarkMarker?) {
        setObserver(marker.map { marker in { image, timestamp in marker.event("decoded_marker", image: image, timestamp: timestamp) } })
    }
}
