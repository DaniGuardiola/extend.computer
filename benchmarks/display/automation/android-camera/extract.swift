import Foundation
import AVFoundation
import ImageIO
import UniformTypeIdentifiers
let args=CommandLine.arguments
guard args.count==4, let requested=Double(args[3]), requested.isFinite, requested>=0 else {exit(2)}
let asset=AVURLAsset(url: URL(fileURLWithPath:args[1]))
guard requested<CMTimeGetSeconds(asset.duration), let track=asset.tracks(withMediaType:.video).first else {exit(2)}
let generator=AVAssetImageGenerator(asset:asset)
generator.appliesPreferredTrackTransform=true
// Default infinite tolerances may choose a nearby keyframe outside the scene.
generator.requestedTimeToleranceBefore = .zero
generator.requestedTimeToleranceAfter = .zero
var actual=CMTime.invalid
let image=try generator.copyCGImage(at:CMTime(seconds:requested,preferredTimescale:60000),actualTime:&actual)
let actualSeconds=CMTimeGetSeconds(actual)
let fps=Double(track.nominalFrameRate)
guard fps.isFinite,fps>0 else {exit(2)}
let frameSeconds=1/fps
guard actualSeconds.isFinite,abs(actualSeconds-requested)<=frameSeconds+0.001 else {
    fputs("Extracted frame outside requested timestamp tolerance\n",stderr);exit(1)
}
let destination=CGImageDestinationCreateWithURL(URL(fileURLWithPath:args[2]) as CFURL,UTType.png.identifier as CFString,1,nil)!
CGImageDestinationAddImage(destination,image,nil)
guard CGImageDestinationFinalize(destination) else {exit(1)}
let timing:[String:Any]=["requested_playback_s":requested,"actual_playback_s":actualSeconds,
    "error_playback_ms":(actualSeconds-requested)*1000,"maximum_error_playback_ms":(frameSeconds+0.001)*1000,
    "selection":"zero-tolerance-v1"]
try JSONSerialization.data(withJSONObject:timing,options:[.sortedKeys]).write(to:URL(fileURLWithPath:args[2]+".timing.json"),options:.atomic)
