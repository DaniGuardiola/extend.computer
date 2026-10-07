import Foundation
import AVFoundation
import ImageIO
import UniformTypeIdentifiers
let asset=AVURLAsset(url: URL(fileURLWithPath:CommandLine.arguments[1]))
let generator=AVAssetImageGenerator(asset:asset)
generator.appliesPreferredTrackTransform=true
let image=try generator.copyCGImage(at:CMTime(seconds:Double(CommandLine.arguments[3])!,preferredTimescale:600),actualTime:nil)
let destination=CGImageDestinationCreateWithURL(URL(fileURLWithPath:CommandLine.arguments[2]) as CFURL,UTType.png.identifier as CFString,1,nil)!
CGImageDestinationAddImage(destination,image,nil)
guard CGImageDestinationFinalize(destination) else {exit(1)}
