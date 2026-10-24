// Deterministic, generated video for native receiver verification. No screen capture.
import Foundation
import CoreVideo
import CoreMedia

@main struct MediaPattern {
    static func main() throws {
        let codec = VideoCodec(rawValue: CommandLine.arguments[1])!
        let width = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2])! : 1280
        let height = CommandLine.arguments.count > 3 ? Int(CommandLine.arguments[3])! : 800
        let lock = NSLock()
        var failed: Error?
        let encoder = try VideoEncoder(width: width, height: height, fps: 30, bitrate: 8_000_000, codec: codec,
            output: { frame in
                var timestamp = frame.timestamp.bigEndian
                var payload = Data(); withUnsafeBytes(of: &timestamp) { payload.append(contentsOf: $0) }
                payload.append(frame.keyframe ? 1 : 0); payload.append(frame.bytes)
                var length = UInt32(payload.count).bigEndian
                lock.lock(); defer { lock.unlock() }
                withUnsafeBytes(of: &length) { FileHandle.standardOutput.write(Data($0)) }
                FileHandle.standardOutput.write(payload)
            }, failure: { error in lock.lock(); failed = error; lock.unlock() })
        for index in 0..<180 {
            var image: CVPixelBuffer?
            try check(CVPixelBufferCreate(nil,width,height,kCVPixelFormatType_32BGRA,
                [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary,&image),"Create pattern")
            let buffer = image!; CVPixelBufferLockBaseAddress(buffer,[])
            let pixels = CVPixelBufferGetBaseAddress(buffer)!.assumingMemoryBound(to: UInt32.self)
            let stride = CVPixelBufferGetBytesPerRow(buffer)/4
            for y in 0..<height { for x in 0..<width {
                let color: UInt32 = y < height*5/8 ? (x < width/3 ? 0xffff0000 : x < width*2/3 ? 0xff00ff00 : 0xff0000ff)
                    : ((x/max(1,width/32)+y/max(1,width/32)+index/15)%2==0 ? 0xffeeeeee : 0xff111111)
                pixels[y*stride+x] = color
            } }
            CVPixelBufferUnlockBaseAddress(buffer,[])
            try encoder.encode(buffer,timestamp:CMTime(value:1_000_000_000+Int64(index)*33_333_333,timescale:1_000_000_000))
            encoder.drain()
        }
        if let failed { throw failed }
    }
}
