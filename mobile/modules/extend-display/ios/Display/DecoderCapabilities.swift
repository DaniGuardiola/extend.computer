import Foundation
import UIKit
import VideoToolbox

/// Decoder capabilities JSON for `start_display_receiver`, in the exact shape
/// Android's MediaSession.capabilities() sends:
///   {"preferred_logical": {width, height, density, physical_width, physical_height},
///    "h264": [{width, height, fps}, ...], "hevc": [...]}
/// The engine picks the logical size and Retina raster only from these entries.
enum DecoderCapabilities {
    /// Android probes each size/rate with areSizeAndRateSupported. VideoToolbox
    /// has no such query, so sizes are limited by a pixel rate every iOS 16.4
    /// device (A12 and newer) decodes in hardware for both codecs: 4096x2304 at
    /// 60 fps (H.264 level 5.2 / HEVC level 5.1 class).
    static let maxPixelRate = 4096 * 2304 * 60

    static func probe(screen: UIScreen = UIScreen.main) throws -> [String: Any] {
        let native = screen.nativeBounds.size
        return try make(physicalWidth: Int(native.width.rounded()), physicalHeight: Int(native.height.rounded()),
                        density: Double(screen.nativeScale), hardware: { VTIsHardwareDecodeSupported($0.type) })
    }

    static func json(screen: UIScreen = UIScreen.main) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: probe(screen: screen), options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    /// Pure port of MediaSession.capabilities(); `density` is physical pixels per point.
    static func make(physicalWidth: Int, physicalHeight: Int, density: Double,
                     hardware: (VideoCodec) -> Bool, maxPixelRate: Int = maxPixelRate) throws -> [String: Any] {
        let screenWidth = max(physicalWidth, physicalHeight)
        let screenHeight = min(physicalWidth, physicalHeight)
        guard density.isFinite, density > 0, screenWidth > 0, screenHeight > 0 else {
            throw DisplayFailure("Invalid fullscreen display metrics")
        }
        let logicalWidth = Double(screenWidth) / density
        let logicalHeight = Double(screenHeight) / density
        let minimum = max(1, max(640 / logicalWidth, 480 / logicalHeight))
        // Java: (int)Math.round(x) / 2 * 2 (round half up, then even).
        func even(_ value: Double) -> Int { Int((value + 0.5).rounded(.down)) / 2 * 2 }
        let preferredWidth = even(logicalWidth * minimum)
        let preferredHeight = even(logicalHeight * minimum)
        var result: [String: Any] = [
            "preferred_logical": ["width": preferredWidth, "height": preferredHeight, "density": density,
                                  "physical_width": screenWidth, "physical_height": screenHeight],
        ]
        for codec in [VideoCodec.h264, .hevc] {
            var sizes: [[String: Int]] = []
            let supported = hardware(codec)
            for fps in [120, 90, 60, 30] {
                for width in [preferredWidth, preferredWidth * 2, screenWidth, 1920, 1600, 1280, 960, 800, 640] {
                    if width > screenWidth || width > 4096 { continue }
                    let height = width == preferredWidth ? preferredHeight
                        : width == preferredWidth * 2 ? preferredHeight * 2
                        : even(Double(width) * Double(screenHeight) / Double(screenWidth))
                    if height < 480 || height > 4096 { continue }
                    guard supported, width * height * fps <= maxPixelRate else { continue }
                    sizes.append(["width": width, "height": height, "fps": fps])
                }
            }
            result[codec.rawValue] = sizes
        }
        return result
    }
}
