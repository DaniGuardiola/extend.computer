import Foundation
import AVFoundation
import CoreVideo

// Numeric metadata only. No location tags, screen pixels or audio are exported.
let args = CommandLine.arguments
if args.count != 3 { fputs("Usage: camera-probe original.mp4 result.json\n", stderr); exit(2) }
let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
guard let track = asset.tracks(withMediaType: .video).first else { fputs("No video track\n", stderr); exit(1) }
let reader = try AVAssetReader(asset: asset)
let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
reader.add(output)
guard reader.startReading() else { fputs("Cannot read video samples\n", stderr); exit(1) }
var pts: [Double] = []
var byteCount = 0
while let sample = output.copyNextSampleBuffer() {
    pts.append(CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample)))
    byteCount += CMSampleBufferGetTotalSampleSize(sample)
    if pts.count > 100_000 { fputs("Clip too long for setup probe\n", stderr); exit(1) }
}
guard reader.status == .completed else { fputs("Incomplete video sample read\n", stderr); exit(1) }
let sorted = pts.sorted()
let gaps = zip(sorted, sorted.dropFirst()).map { ($1 - $0) * 1000 }.sorted()
func percentile(_ values: [Double], _ fraction: Double) -> Any {
    guard !values.isEmpty else { return NSNull() }
    return values[max(0, Int(ceil(Double(values.count) * fraction)) - 1)]
}
var captureMetadata: [[String: String]] = []
for format in asset.availableMetadataFormats {
    for item in asset.metadata(forFormat: format) {
        let key = String(describing: item.key ?? "" as NSString)
        if key.lowercased().contains("capture.fps") || key.lowercased().contains("capture_fps") || key.lowercased().contains("slowmotion") {
            captureMetadata.append(["key": key, "value": item.stringValue ?? "unavailable"])
        }
    }
}
let duration = CMTimeGetSeconds(asset.duration)
let result: [String: Any] = [
    "preferred_transform": [track.preferredTransform.a, track.preferredTransform.b, track.preferredTransform.c, track.preferredTransform.d, track.preferredTransform.tx, track.preferredTransform.ty],
    "width": track.naturalSize.width,
    "height": track.naturalSize.height,
    "duration_s": duration,
    "nominal_playback_fps": track.nominalFrameRate,
    "video_sample_count": pts.count,
    "average_samples_per_playback_second": duration > 0 ? Double(pts.count) / duration : 0,
    "sample_gap_p50_ms": percentile(gaps, 0.5),
    "sample_gap_p95_ms": percentile(gaps, 0.95),
    "encoded_video_bytes": byteCount,
    "capture_metadata": captureMetadata,
    "capture_cadence_verified": false
]
let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
try data.write(to: URL(fileURLWithPath: args[2]), options: [.withoutOverwriting])
