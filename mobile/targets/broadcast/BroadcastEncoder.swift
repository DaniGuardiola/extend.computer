import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

// Minimal copy of VideoEncoder from native/macos/display/VideoCodec.swift
// (hardware-only, real-time, no reordering, Annex-B output with parameter sets
// on keyframes) without the macOS metrics/experiment hooks. If the encoder
// moves to native/apple/shared, the extension should compile that instead.

struct EncodedFrame {
  let timestamp: UInt64
  let keyframe: Bool
  let bytes: Data
}

final class BroadcastEncoder {
  let config: SharingConfig
  private let hevc: Bool
  private var session: VTCompressionSession?
  /// At most two frames inside VideoToolbox; further captures are dropped,
  /// never queued, which keeps the extension well inside its memory limit.
  private let inFlight = DispatchSemaphore(value: 2)
  private let lock = NSLock()
  private var forceKeyframe = true
  private var lastSubmitted: CMTime?
  private let output: (EncodedFrame) -> Void
  private let failure: (Error) -> Void

  init(config: SharingConfig, output: @escaping (EncodedFrame) -> Void, failure: @escaping (Error) -> Void) throws {
    self.config = config; self.output = output; self.failure = failure
    hevc = config.codec == "hevc"
    // Every supported iPhone encodes H.264/HEVC in hardware; the key only
    // exists in the SDK from iOS 17.4.
    var specification: [CFString: Any] = [:]
    if #available(iOS 17.4, *) { specification[kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder] = true }
    var created: VTCompressionSession?
    let status = VTCompressionSessionCreate(allocator: nil, width: Int32(config.width), height: Int32(config.height),
      codecType: hevc ? kCMVideoCodecType_HEVC : kCMVideoCodecType_H264,
      encoderSpecification: specification as CFDictionary, imageBufferAttributes: nil, compressedDataAllocator: nil,
      outputCallback: { context, _, status, flags, sample in
        guard let context else { return }
        let encoder = Unmanaged<BroadcastEncoder>.fromOpaque(context).takeUnretainedValue()
        defer { encoder.inFlight.signal() }
        if status != noErr { encoder.failure(SharingError("Hardware encoding failed (\(status)).")); return }
        guard !flags.contains(.frameDropped), let sample, CMSampleBufferDataIsReady(sample) else { return }
        do { encoder.output(try encoder.frame(sample)) } catch { encoder.failure(error) }
      }, refcon: Unmanaged.passUnretained(self).toOpaque(), compressionSessionOut: &created)
    if status != noErr {
      if let created { VTCompressionSessionInvalidate(created) }
      throw SharingError("Hardware encoding doesn't support \(config.width) × \(config.height) with \(hevc ? "HEVC" : "H.264") (\(status)).")
    }
    guard let created else { throw SharingError("Hardware encoder unavailable.") }
    session = created
    let profile = hevc ? kVTProfileLevel_HEVC_Main_AutoLevel : kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel
    for (key, value) in [
      (kVTCompressionPropertyKey_RealTime, kCFBooleanTrue as CFTypeRef),
      (kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse as CFTypeRef),
      (kVTCompressionPropertyKey_ProfileLevel, profile),
      (kVTCompressionPropertyKey_AverageBitRate, config.bitrate as CFNumber),
      (kVTCompressionPropertyKey_ExpectedFrameRate, config.fps as CFNumber),
      (kVTCompressionPropertyKey_ColorPrimaries, kCVImageBufferColorPrimaries_ITU_R_709_2),
      (kVTCompressionPropertyKey_TransferFunction, kCVImageBufferTransferFunction_ITU_R_709_2),
      (kVTCompressionPropertyKey_YCbCrMatrix, kCVImageBufferYCbCrMatrix_ITU_R_709_2),
      (kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, 1 as CFNumber),
    ] as [(CFString, CFTypeRef)] {
      let status = VTSessionSetProperty(created, key: key, value: value)
      guard status == noErr else { invalidate(); throw SharingError("Configure hardware encoder \(key) failed (\(status)).") }
    }
    let prepared = VTCompressionSessionPrepareToEncodeFrames(created)
    guard prepared == noErr else { invalidate(); throw SharingError("Prepare hardware encoder failed (\(prepared)).") }
  }

  func requestKeyframe() { lock.lock(); forceKeyframe = true; lock.unlock() }

  /// Returns false when the frame was skipped (encoder busy or stale timestamp).
  @discardableResult
  func encode(_ image: CVPixelBuffer, timestamp: CMTime) throws -> Bool {
    lock.lock(); defer { lock.unlock() }
    guard let session else { return false }
    if let lastSubmitted, CMTimeCompare(timestamp, lastSubmitted) <= 0 { return false }
    guard inFlight.wait(timeout: .now()) == .success else { return false }
    let keyframe = forceKeyframe
    let properties = keyframe ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
    let status = VTCompressionSessionEncodeFrame(session, imageBuffer: image, presentationTimeStamp: timestamp,
      duration: .invalid, frameProperties: properties, sourceFrameRefcon: nil, infoFlagsOut: nil)
    guard status == noErr else { inFlight.signal(); throw SharingError("Submit video frame failed (\(status)).") }
    forceKeyframe = false
    lastSubmitted = timestamp
    return true
  }

  func invalidate() {
    lock.lock(); let session = self.session; self.session = nil; lock.unlock()
    guard let session else { return }
    VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
    VTCompressionSessionInvalidate(session)
  }
  deinit { invalidate() }

  private func frame(_ sample: CMSampleBuffer) throws -> EncodedFrame {
    guard let format = CMSampleBufferGetFormatDescription(sample), let block = CMSampleBufferGetDataBuffer(sample) else {
      throw SharingError("Encoder returned incomplete frame.")
    }
    let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
    let keyframe = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
    let length = CMBlockBufferGetDataLength(block)
    guard length <= 8 * 1024 * 1024 else { throw SharingError("Encoded frame too large.") }
    var bytes = Data(capacity: length + (keyframe ? 256 : 16))
    if keyframe {
      for index in 0..<(hevc ? 3 : 2) {
        var pointer: UnsafePointer<UInt8>?; var size = 0; var count = 0; var header: Int32 = 0
        let status = hevc
          ? CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: index,
              parameterSetPointerOut: &pointer, parameterSetSizeOut: &size, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &header)
          : CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: index,
              parameterSetPointerOut: &pointer, parameterSetSizeOut: &size, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &header)
        guard status == noErr else { throw SharingError("Read video parameter sets failed (\(status)).") }
        guard header == 4, let pointer else { throw SharingError("Unsupported video framing.") }
        bytes.append(contentsOf: [0, 0, 0, 1]); bytes.append(pointer, count: size)
      }
    }
    var avcc = Data(count: length)
    let copied = avcc.withUnsafeMutableBytes { buffer in
      CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: buffer.baseAddress!)
    }
    guard copied == noErr else { throw SharingError("Read encoded frame failed (\(copied)).") }
    // AVCC length prefixes → Annex-B start codes.
    var position = 0
    while position < avcc.count {
      guard position + 4 <= avcc.count else { throw SharingError("Truncated video length.") }
      let count = avcc[position..<position + 4].reduce(0) { ($0 << 8) | Int($1) }; position += 4
      guard count > 0, position + count <= avcc.count else { throw SharingError("Invalid video length.") }
      bytes.append(contentsOf: [0, 0, 0, 1]); bytes.append(avcc[position..<position + count]); position += count
    }
    guard bytes.count <= 8 * 1024 * 1024 else { throw SharingError("Encoded frame too large.") }
    let time = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample))
    return EncodedFrame(timestamp: UInt64(max(0, time) * 1_000_000_000), keyframe: keyframe, bytes: bytes)
  }
}
