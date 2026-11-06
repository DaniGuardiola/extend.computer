import Foundation
import CoreMedia
import CoreVideo
import VideoToolbox

// Shared by the macOS display helper (native/macos/display) and the iOS
// receiver (mobile/modules/extend-display/ios/Display). Foundation, CoreMedia
// and VideoToolbox only: no AppKit/UIKit and no platform telemetry. Platform
// telemetry plugs in through VideoDecoder's metric/observer hooks.

struct DisplayFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}
func check(_ status: OSStatus, _ operation: String) throws {
    guard status == noErr else { throw DisplayFailure("\(operation) failed (\(status)).") }
}

// Complete encoded access units, not individually queued NALs.
struct VideoFrame {
    let timestamp: UInt64
    let keyframe: Bool
    let bytes: Data
}

enum VideoCodec: String {
    case h264, hevc
    var type: CMVideoCodecType { self == .hevc ? kCMVideoCodecType_HEVC : kCMVideoCodecType_H264 }
}

func annexBNALs(_ data: Data) throws -> [Data] {
    guard data.count <= 16 * 1024 * 1024 else { throw DisplayFailure("Video frame too large.") }
    // Inspect the owned access unit in place. Copying it into a byte array first
    // adds a full-frame allocation on every decode; returned NALs still own
    // their bytes after this scoped borrow ends.
    return try data.withUnsafeBytes { raw in
        let bytes = raw.bindMemory(to: UInt8.self)
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
}

final class VideoDecoder {
    // RTP pacing can deliver a short frame burst. Keep that burst bounded while
    // preserving reference frames instead of resetting the decoder on each overlap.
    private let inFlight = DispatchSemaphore(value: 4)
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private let codec: VideoCodec
    private var vps: Data?; private var sps: Data?; private var pps: Data?
    private let output: (CVPixelBuffer, UInt64) -> Void
    private let failure: () -> Void
    // (stage, frame timestamp, value): the macOS helper forwards these to
    // DisplayMetrics; other hosts may ignore them.
    private let metric: (String, UInt64, Int) -> Void
    private let hookLock = NSLock()
    private var observer: ((CVPixelBuffer, UInt64) -> Void)?
    private var firstOutput: ((CVPixelBuffer) -> Void)?
    private var reportedFirst = false
    /// Hardware decode state of the current session, once known.
    private(set) var hardware: Bool?

    init(codec: VideoCodec = .h264, metric: @escaping (String, UInt64, Int) -> Void = { _, _, _ in },
         output: @escaping (CVPixelBuffer, UInt64) -> Void, failure: @escaping () -> Void = {}) {
        self.codec = codec; self.metric = metric; self.output = output; self.failure = failure
    }
    /// Called for each decoded image before `output`, on the decoder thread.
    func setObserver(_ observer: ((CVPixelBuffer, UInt64) -> Void)?) {
        hookLock.lock(); self.observer = observer; hookLock.unlock()
    }
    /// Called once for the first decoded image of this decoder instance.
    func setFirstOutput(_ callback: ((CVPixelBuffer) -> Void)?) {
        hookLock.lock(); firstOutput = callback; hookLock.unlock()
    }
    private func observe(_ image: CVPixelBuffer, timestamp: UInt64) {
        hookLock.lock()
        let first = reportedFirst ? nil : firstOutput; reportedFirst = true
        let observer = observer
        hookLock.unlock()
        first?(image)
        observer?(image, timestamp)
    }
    /// Submits one access unit. `wait` bounds how long a full in-flight window
    /// may block (default: fail immediately so the caller can request a keyframe).
    func decode(_ frame: VideoFrame, wait: DispatchTime = .now()) throws {
        guard inFlight.wait(timeout: wait) == .success else {
            metric("decoder_busy", frame.timestamp, 0)
            throw DisplayFailure("Decoder is busy; requesting a fresh keyframe.")
        }
        var submitted = false
        defer { if !submitted { inFlight.signal() } }
        let nals = try annexBNALs(frame.bytes)
        var picture = Data(); var changed = false
        for nal in nals {
            guard codec == .h264 || nal.count >= 2 else { throw DisplayFailure("Truncated HEVC NAL.") }
            let type = codec == .hevc ? (nal[0] >> 1) & 63 : nal[0] & 31
            if codec == .hevc && type == 32 { changed = changed || vps != nal; vps = nal; continue }
            if type == (codec == .hevc ? 33 : 7) { changed = changed || sps != nal; sps = nal; continue }
            if type == (codec == .hevc ? 34 : 8) { changed = changed || pps != nal; pps = nal; continue }
            var length = UInt32(nal.count).bigEndian
            withUnsafeBytes(of: &length) { picture.append(contentsOf: $0) }; picture.append(nal)
        }
        if changed || session == nil {
            guard frame.keyframe, let sps, let pps else { return }
            if let session {
                VTDecompressionSessionWaitForAsynchronousFrames(session)
                VTDecompressionSessionInvalidate(session); self.session = nil
            }
            guard codec != .hevc || vps != nil else { return }
            let parameters = codec == .hevc ? [vps!, sps, pps] : [sps, pps]
            // NSData owns stable parameter-set storage for the C API call.
            let storage = parameters.map { $0 as NSData }
            var pointers = storage.map { $0.bytes.assumingMemoryBound(to: UInt8.self) }
            var sizes = storage.map { $0.length }
            let status = codec == .hevc
                ? CMVideoFormatDescriptionCreateFromHEVCParameterSets(allocator: nil, parameterSetCount: 3,
                    parameterSetPointers: &pointers, parameterSetSizes: &sizes, nalUnitHeaderLength: 4,
                    extensions: nil, formatDescriptionOut: &format)
                : CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: nil, parameterSetCount: 2,
                    parameterSetPointers: &pointers, parameterSetSizes: &sizes, nalUnitHeaderLength: 4,
                    formatDescriptionOut: &format)
            try check(status, "Configure video decoder")
            var callback = VTDecompressionOutputCallbackRecord(decompressionOutputCallback: { context, _, status, _, image, pts, _ in
                guard let context else { return }
                let decoder = Unmanaged<VideoDecoder>.fromOpaque(context).takeUnretainedValue()
                defer { decoder.inFlight.signal() }
                guard status == noErr, let image else { decoder.failure(); return }
                let timestamp = UInt64(max(0, CMTimeGetSeconds(pts)) * 1e9)
                decoder.metric("decoded", timestamp, 0)
                decoder.observe(image, timestamp: timestamp)
                decoder.output(image, timestamp)
            }, decompressionOutputRefCon: Unmanaged.passUnretained(self).toOpaque())
            try check(VTDecompressionSessionCreate(allocator: nil, formatDescription: format!,
                decoderSpecification: Self.decoderSpecification,
                imageBufferAttributes: [kCVPixelBufferMetalCompatibilityKey: true] as CFDictionary,
                outputCallback: &callback, decompressionSessionOut: &session), "Create native decoder")
            if #available(iOS 17.0, *), let session {
                var hardware: Unmanaged<CFTypeRef>?
                if VTSessionCopyProperty(session, key: kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder,
                    allocator: nil, valueOut: &hardware) == noErr, let active = hardware?.takeRetainedValue() as? NSNumber {
                    self.hardware = active.boolValue
                    metric("decoder_hardware", 0, active.boolValue ? 1 : 0)
                }
            }
        }
        guard !picture.isEmpty, let format, let session else { return }
        var block: CMBlockBuffer?
        try check(CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: picture.count,
            blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: picture.count, flags: 0,
            blockBufferOut: &block), "Allocate decode buffer")
        try picture.withUnsafeBytes { b in try check(CMBlockBufferReplaceDataBytes(with: b.baseAddress!, blockBuffer: block!,
            offsetIntoDestination: 0, dataLength: picture.count), "Copy decode frame") }
        var size = picture.count
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: CMTime(value: Int64(frame.timestamp), timescale: 1_000_000_000), decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        try check(CMSampleBufferCreateReady(allocator: nil, dataBuffer: block, formatDescription: format,
            sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample), "Create decode sample")
        metric("decoder_submitted", frame.timestamp, 0)
        let callStarted = DispatchTime.now().uptimeNanoseconds
        let status = VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample!, flags: [._EnableAsynchronousDecompression],
            frameRefcon: nil, infoFlagsOut: nil)
        metric("decoder_call_us", frame.timestamp, Int((DispatchTime.now().uptimeNanoseconds - callStarted) / 1000))
        try check(status, "Decode frame")
        submitted = true
    }
    func drain() { if let session { VTDecompressionSessionWaitForAsynchronousFrames(session) } }
    deinit { drain(); if let session { VTDecompressionSessionInvalidate(session) } }

    // The Enable key defaults to true and is iOS 17+ API; the module targets
    // iOS 16.4, so iOS relies on the default hardware selection.
    private static var decoderSpecification: CFDictionary? {
        #if os(macOS)
        return [kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: true] as CFDictionary
        #else
        return nil
        #endif
    }
}
