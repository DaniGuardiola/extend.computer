import CoreVideo
import Foundation
import ImageIO
import VideoToolbox

/// Turns ReplayKit's buffers (always in the panel's native portrait layout,
/// with the interface orientation as an attachment) into upright frames at the
/// negotiated size. Output buffers come from small pools, so a slow encoder
/// makes us drop frames instead of allocating more.
final class FrameTransform {
  private var rotation: VTPixelRotationSession?
  private var transfer: VTPixelTransferSession?
  private var rotatedPool: (pool: CVPixelBufferPool, width: Int, height: Int, format: OSType)?
  private var scaledPool: (pool: CVPixelBufferPool, width: Int, height: Int, format: OSType)?

  /// Upright size of a buffer shown with `orientation`.
  static func uprightSize(width: Int, height: Int, orientation: CGImagePropertyOrientation) -> (width: Int, height: Int) {
    switch orientation {
    case .left, .right, .leftMirrored, .rightMirrored: return (height, width)
    default: return (width, height)
    }
  }

  func invalidate() {
    if let rotation { VTPixelRotationSessionInvalidate(rotation) }
    if let transfer { VTPixelTransferSessionInvalidate(transfer) }
    rotation = nil; transfer = nil; rotatedPool = nil; scaledPool = nil
  }
  deinit { invalidate() }

  /// nil means "drop this frame" (pool exhausted); errors are fatal.
  /// Scales before rotating so every intermediate buffer is encoder-sized: a
  /// full-resolution rotated copy of a landscape screen would cost several MB
  /// of IOSurface per buffer against the extension's memory limit.
  func process(_ source: CVPixelBuffer, orientation: CGImagePropertyOrientation, width: Int, height: Int) throws -> CVPixelBuffer? {
    var image = source
    let format = CVPixelBufferGetPixelFormatType(source)
    let angle = Self.angle(orientation)
    // Size the scaled (still unrotated) frame must have to rotate into width × height.
    let unrotated = angle == nil ? (width: width, height: height)
      : Self.uprightSize(width: width, height: height, orientation: orientation)
    if CVPixelBufferGetWidth(image) != unrotated.width || CVPixelBufferGetHeight(image) != unrotated.height {
      guard let target = try buffer(&scaledPool, width: unrotated.width, height: unrotated.height, format: format) else { return nil }
      if transfer == nil {
        var created: VTPixelTransferSession?
        try check(VTPixelTransferSessionCreate(allocator: nil, pixelTransferSessionOut: &created), "Create scaler")
        transfer = created
      }
      guard let transfer else { throw SharingError("Scaler unavailable.") }
      try check(VTPixelTransferSessionTransferImage(transfer, from: image, to: target), "Scale frame")
      image = target
    }
    if let angle {
      guard let target = try buffer(&rotatedPool, width: width, height: height, format: format) else { return nil }
      if rotation == nil {
        var created: VTPixelRotationSession?
        try check(VTPixelRotationSessionCreate(nil, &created), "Create rotation")
        rotation = created
      }
      guard let rotation else { throw SharingError("Rotation unavailable.") }
      try check(VTSessionSetProperty(rotation, key: kVTPixelRotationPropertyKey_Rotation, value: angle), "Set rotation")
      try check(VTPixelRotationSessionRotateImage(rotation, image, target), "Rotate frame")
      image = target
    }
    return image
  }

  private static func angle(_ orientation: CGImagePropertyOrientation) -> CFString? {
    // EXIF semantics: `.right` (6) must be turned 90° clockwise to look upright.
    switch orientation {
    case .right, .rightMirrored: return kVTRotation_CW90
    case .left, .leftMirrored: return kVTRotation_CCW90
    case .down, .downMirrored: return kVTRotation_180
    default: return nil
    }
  }

  private func buffer(_ slot: inout (pool: CVPixelBufferPool, width: Int, height: Int, format: OSType)?,
                      width: Int, height: Int, format: OSType) throws -> CVPixelBuffer? {
    if slot == nil || slot!.width != width || slot!.height != height || slot!.format != format {
      let attributes: [CFString: Any] = [
        kCVPixelBufferPixelFormatTypeKey: format, kCVPixelBufferWidthKey: width, kCVPixelBufferHeightKey: height,
        kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
      ]
      var pool: CVPixelBufferPool?
      try check(CVPixelBufferPoolCreate(nil, [kCVPixelBufferPoolMinimumBufferCountKey: 2] as CFDictionary,
                                        attributes as CFDictionary, &pool), "Create frame pool")
      guard let pool else { throw SharingError("Frame pool unavailable.") }
      slot = (pool, width, height, format)
    }
    var image: CVPixelBuffer?
    // Two in the encoder, one being prepared, one kept for keyframe refresh.
    let status = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(nil, slot!.pool,
      [kCVPixelBufferPoolAllocationThresholdKey: 4] as CFDictionary, &image)
    if status == kCVReturnWouldExceedAllocationThreshold { return nil }
    try check(status, "Allocate frame")
    return image
  }

  private func check(_ status: OSStatus, _ operation: String) throws {
    guard status == noErr else { throw SharingError("\(operation) failed (\(status)).") }
  }
}
