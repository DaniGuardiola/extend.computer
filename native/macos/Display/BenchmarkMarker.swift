import Foundation
import CoreVideo
import CoreImage
import Metal

// Explicit benchmark-only observation of the synthetic fixture's checksum
// counter. No image storage or hashing, and no work when the flag is disabled.
final class BenchmarkMarker {
    private let logicalWidth: Int
    private let logicalHeight: Int
    private var reportedFormat = false
    private let stateLock = NSLock()
    private var cachedGPUContext: CIContext?
    private func gpuContext() -> CIContext? {
        stateLock.lock(); defer { stateLock.unlock() }
        if cachedGPUContext == nil, let device = MTLCreateSystemDefaultDevice() {
            cachedGPUContext = CIContext(mtlDevice: device, options: [.cacheIntermediates: false])
        }
        return cachedGPUContext
    }
    init?(logicalWidth: Int, logicalHeight: Int, syntheticSelfTest: Bool = false) {
        guard syntheticSelfTest || ProcessInfo.processInfo.environment["EXTEND_DISPLAY_BENCHMARK_MARKER"] == "1",
              logicalWidth >= 640, logicalHeight >= 480 else { return nil }
        self.logicalWidth = logicalWidth; self.logicalHeight = logicalHeight
    }
    static func checksum(_ value: Int) -> Int {
        var crc = 0
        for bit in (0..<12).reversed() {
            let feedback = ((crc >> 3) & 1) ^ ((value >> bit) & 1)
            crc = (crc << 1) & 15
            if feedback != 0 { crc ^= 3 }
        }
        return crc
    }
    static func decodePlane(width: Int, height: Int, logicalWidth: Int, logicalHeight: Int,
                            luma: (Int, Int) -> Double) -> Int? {
        guard logicalWidth >= 640, logicalHeight >= 480, width % logicalWidth == 0 else { return nil }
        let scale = width / logicalWidth
        guard (1...3).contains(scale), height == logicalHeight * scale else { return nil }
        var word = 0
        for bit in 0..<16 {
            let x = (logicalWidth - 374 + bit * 20) * scale
            var states: [Bool] = []
            for bottom in [120, 168] {
                let y = height - bottom * scale
                guard x >= 1, y >= 1, x + 1 < width, y + 1 < height else { return nil }
                var mean = 0.0
                for dy in -1...1 { for dx in -1...1 { mean += luma(x + dx, y + dy) } }
                mean /= 9
                guard mean.isFinite, mean <= 64 || mean >= 192 else { return nil }
                states.append(mean >= 192)
            }
            guard states[0] != states[1] else { return nil }
            word = (word << 1) | (states[0] ? 1 : 0)
        }
        let value = word >> 4
        return (word & 15) == checksum(value) ? value : nil
    }
    // Compressed IOSurfaces cannot be treated as linear NV12 planes. Read only
    // two 3-pixel-high synthetic counter strips into a six-row GPU atlas.
    // Production decoder format stays untouched; no full-frame CPU readback.
    private func gpuValue(_ image: CVPixelBuffer) -> Int? {
        let width = CVPixelBufferGetWidth(image), height = CVPixelBufferGetHeight(image)
        guard width % logicalWidth == 0 else { return nil }
        let scale = width / logicalWidth
        guard (1...3).contains(scale), height == logicalHeight * scale, let context = gpuContext() else { return nil }
        let firstX = (logicalWidth - 374) * scale - 1
        let atlasWidth = 300 * scale + 3
        let source = CIImage(cvPixelBuffer: image)
        var atlas = CIImage(color: .black).cropped(to: CGRect(x: 0, y: 0, width: atlasWidth, height: 6))
        for (index, bottom) in [120, 168].enumerated() {
            // Convert top-down CVPixelBuffer coordinates to Core Image's
            // bottom-left coordinates, including the physical pixel center.
            let firstY = bottom * scale - 2
            let strip = source.cropped(to: CGRect(x: firstX, y: firstY, width: atlasWidth, height: 3))
                .transformed(by: CGAffineTransform(translationX: -CGFloat(firstX), y: CGFloat(index * 3 - firstY)))
            atlas = strip.composited(over: atlas)
        }
        var rgba = [UInt8](repeating: 0, count: atlasWidth * 6 * 4)
        rgba.withUnsafeMutableBytes { bytes in
            context.render(atlas, toBitmap: bytes.baseAddress!, rowBytes: atlasWidth * 4,
                bounds: CGRect(x: 0, y: 0, width: atlasWidth, height: 6), format: .RGBA8,
                colorSpace: CGColorSpace(name: CGColorSpace.sRGB))
        }
        return Self.decodePlane(width: width, height: height, logicalWidth: logicalWidth, logicalHeight: logicalHeight) { x, y in
            let lowerCenter = height - 120 * scale
            let upperCenter = height - 168 * scale
            let row = abs(y - lowerCenter) <= 1 ? 1 + lowerCenter - y : 4 + upperCenter - y
            // CI bitmap output rows start at the top of the six-row atlas.
            let offset = ((5 - row) * atlasWidth + x - firstX) * 4
            return 0.2126 * Double(rgba[offset]) + 0.7152 * Double(rgba[offset + 1]) + 0.0722 * Double(rgba[offset + 2])
        }
    }
    func value(_ image: CVPixelBuffer, forceGPU: Bool = false) -> Int? {
        let format = CVPixelBufferGetPixelFormatType(image)
        let linear = [kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                      kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVPixelFormatType_32BGRA].contains(format)
        if forceGPU || !linear { return gpuValue(image) }
        guard CVPixelBufferLockBaseAddress(image, .readOnly) == kCVReturnSuccess else { return nil }
        defer { CVPixelBufferUnlockBaseAddress(image, .readOnly) }
        let planar = CVPixelBufferIsPlanar(image)
        guard let address = planar ? CVPixelBufferGetBaseAddressOfPlane(image, 0) : CVPixelBufferGetBaseAddress(image) else { return nil }
        let stride = planar ? CVPixelBufferGetBytesPerRowOfPlane(image, 0) : CVPixelBufferGetBytesPerRow(image)
        let bytes = address.assumingMemoryBound(to: UInt8.self)
        return Self.decodePlane(width: CVPixelBufferGetWidth(image), height: CVPixelBufferGetHeight(image),
                                    logicalWidth: logicalWidth, logicalHeight: logicalHeight) { x, y in
            if planar { return Double(bytes[y * stride + x]) }
            let offset = y * stride + x * 4
            return 0.2126 * Double(bytes[offset + 2]) + 0.7152 * Double(bytes[offset + 1]) + 0.0722 * Double(bytes[offset])
        }
    }
    func event(_ stage: String, image: CVPixelBuffer, timestamp: UInt64) {
        let format = CVPixelBufferGetPixelFormatType(image)
        stateLock.lock()
        let needsFormat = !reportedFormat; reportedFormat = true
        stateLock.unlock()
        if needsFormat { DisplayMetrics.shared.event(stage + "_pixel_format", bytes: Int(format)) }
        let start = DispatchTime.now().uptimeNanoseconds
        let decoded = value(image)
        DisplayMetrics.shared.event(stage + "_read_us", timestamp: timestamp,
            bytes: Int((DispatchTime.now().uptimeNanoseconds - start) / 1000))
        if let decoded { DisplayMetrics.shared.event(stage, timestamp: timestamp, bytes: decoded) }
        else { DisplayMetrics.shared.event(stage + "_invalid", timestamp: timestamp) }
    }
}

// Synthetic pixels only, shared by pure/GPU crop tests and hardware-codec tests.
func benchmarkMarkerTestImage(logicalWidth: Int, logicalHeight: Int, scale: Int, value: Int) throws -> CVPixelBuffer {
    guard logicalWidth >= 640, logicalHeight >= 480, (1...3).contains(scale), (0...4095).contains(value),
          logicalWidth * scale <= 4096, logicalHeight * scale <= 4096 else {
        throw DisplayFailure("Invalid synthetic marker geometry.")
    }
    let width = logicalWidth * scale, height = logicalHeight * scale
    var image: CVPixelBuffer?
    try check(CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA,
        [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &image), "Create synthetic marker frame")
    guard let image else { throw DisplayFailure("Synthetic marker frame unavailable.") }
    CVPixelBufferLockBaseAddress(image, [])
    defer { CVPixelBufferUnlockBaseAddress(image, []) }
    let bytes = CVPixelBufferGetBaseAddress(image)!.assumingMemoryBound(to: UInt8.self)
    let stride = CVPixelBufferGetBytesPerRow(image)
    memset(bytes, 0, stride * height)
    let word = (value << 4) | BenchmarkMarker.checksum(value)
    for bit in 0..<16 {
        for (row, bottom) in [120, 168].enumerated() {
            let white = (((word >> (15 - bit)) & 1) != 0) != (row == 1)
            for y in (height - (bottom + 16) * scale)...(height - (bottom - 16) * scale) {
                for x in ((logicalWidth - 374 + bit * 20 - 8) * scale)...((logicalWidth - 374 + bit * 20 + 8) * scale) {
                    let offset = y * stride + x * 4
                    for channel in 0..<3 { bytes[offset + channel] = white ? 255 : 0 }
                    bytes[offset + 3] = 255
                }
            }
        }
    }
    return image
}

func benchmarkMarkerSelfTest() throws {
    let logicalWidth = 1728, logicalHeight = 1085
    for scale in [1, 2, 3] {
        let width = logicalWidth * scale, height = logicalHeight * scale
        for value in [0, 1, 15, 16, 511, 2048, 4095] {
            let word = (value << 4) | BenchmarkMarker.checksum(value)
            func sample(_ x: Int, _ y: Int, corruptBit: Bool = false, ambiguous: Bool = false) -> Double {
                let bit = Int((Double(x) / Double(scale) - Double(logicalWidth - 374) + 10) / 20)
                guard (0..<16).contains(bit) else { return 16 }
                let lower = abs(y - (height - 120 * scale)) <= 1
                let upper = abs(y - (height - 168 * scale)) <= 1
                guard lower || upper else { return 16 }
                if ambiguous && bit == 0 { return 128 }
                var white = ((word >> (15 - bit)) & 1) != 0
                if upper { white.toggle() }
                if corruptBit && bit == 15 { white.toggle() }
                return white ? 235 : 16
            }
            let good = BenchmarkMarker.decodePlane(width: width, height: height, logicalWidth: logicalWidth,
                                                  logicalHeight: logicalHeight, luma: { sample($0, $1) })
            guard good == value else { throw DisplayFailure("Benchmark marker decode failed.") }
            let corrupt = BenchmarkMarker.decodePlane(width: width, height: height, logicalWidth: logicalWidth,
                logicalHeight: logicalHeight, luma: { sample($0, $1, corruptBit: true) })
            guard corrupt == nil else { throw DisplayFailure("Benchmark marker accepted invalid checksum.") }
            let ambiguous = BenchmarkMarker.decodePlane(width: width, height: height, logicalWidth: logicalWidth,
                logicalHeight: logicalHeight, luma: { sample($0, $1, ambiguous: true) })
            guard ambiguous == nil else { throw DisplayFailure("Benchmark marker accepted ambiguous pixels.") }
        }
        guard BenchmarkMarker.decodePlane(width: width, height: height, logicalWidth: logicalWidth,
            logicalHeight: logicalHeight, luma: { _, _ in 235 }) == nil else {
            throw DisplayFailure("Benchmark marker accepted non-complementary rows.")
        }
    }
    guard BenchmarkMarker.decodePlane(width: 3455, height: 2170, logicalWidth: logicalWidth,
        logicalHeight: logicalHeight, luma: { _, _ in 235 }) == nil else {
        throw DisplayFailure("Benchmark marker accepted incorrect backing geometry.")
    }
    let retinaImage = try benchmarkMarkerTestImage(logicalWidth: 640, logicalHeight: 480, scale: 2, value: 273)
    let retinaMarker = BenchmarkMarker(logicalWidth: 640, logicalHeight: 480, syntheticSelfTest: true)!
    guard retinaMarker.value(retinaImage) == 273, retinaMarker.value(retinaImage, forceGPU: true) == 273 else {
        throw DisplayFailure("Retina synthetic marker CPU/GPU crop orientation failed.")
    }
    print("PASS: synthetic benchmark marker, complementary bits, checksum, ambiguous samples, backing geometry and Retina GPU crop.")
}
