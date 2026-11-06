import Foundation

// Wire shapes exchanged with the shared Rust engine's display receiver
// (native/android/engine/src/media.rs + api.rs). Kept identical to Android's
// MediaSession.java so both apps drive the same engine calls the same way.

/// One packet from `next_display_frame`: 8-byte big-endian source timestamp
/// (ns), 1-byte keyframe flag, 8-byte big-endian media revision, Annex-B access unit.
struct DisplayFramePacket {
    static let headerLength = 17
    static let maxPayload = 8 * 1024 * 1024

    let timestamp: UInt64
    let keyframe: Bool
    let revision: Int64
    let accessUnit: Data

    init(_ payload: Data) throws {
        guard payload.count > Self.headerLength, payload.count <= Self.maxPayload + Self.headerLength else {
            throw DisplayFailure("Invalid video frame")
        }
        let bytes = [UInt8](payload.prefix(Self.headerLength))
        func be64(_ offset: Int) -> UInt64 { bytes[offset..<offset + 8].reduce(0) { ($0 << 8) | UInt64($1) } }
        let timestamp = be64(0), flag = bytes[8], revision = be64(9)
        // Java reads signed longs: a set top bit is a negative, invalid value.
        guard timestamp <= UInt64(Int64.max), flag <= 1, revision <= UInt64(Int64.max) else {
            throw DisplayFailure("Invalid video timestamp")
        }
        self.timestamp = timestamp; keyframe = flag == 1; self.revision = Int64(revision)
        accessUnit = payload.subdata(in: payload.startIndex + Self.headerLength ..< payload.endIndex)
    }

    var videoFrame: VideoFrame { VideoFrame(timestamp: timestamp, keyframe: keyframe, bytes: accessUnit) }
}

/// Negotiated stream configuration from `display_state().config`.
struct DisplayStreamConfig: Equatable {
    let codec: VideoCodec
    /// Logical (desktop point) size.
    let width: Int, height: Int
    let fps: Int
    let retina: Bool
    let separateCursor: Bool
    /// Encoded raster: logical size times 2 when Retina.
    var rasterWidth: Int { width * (retina ? 2 : 1) }
    var rasterHeight: Int { height * (retina ? 2 : 1) }

    init(codec: VideoCodec, width: Int, height: Int, fps: Int, retina: Bool, separateCursor: Bool = false) {
        self.codec = codec; self.width = width; self.height = height; self.fps = fps
        self.retina = retina; self.separateCursor = separateCursor
    }
    init(_ object: [String: Any]) throws {
        guard let name = object["codec"] as? String, let codec = VideoCodec(rawValue: name),
              let width = (object["width"] as? NSNumber)?.intValue, let height = (object["height"] as? NSNumber)?.intValue,
              let fps = (object["fps"] as? NSNumber)?.intValue, let retina = object["retina"] as? Bool,
              (1...4096).contains(width), (1...4096).contains(height), (1...240).contains(fps) else {
            throw DisplayFailure("Invalid display configuration")
        }
        self.init(codec: codec, width: width, height: height, fps: fps, retina: retina,
                  separateCursor: object["separate_cursor"] as? Bool ?? false)
    }
}

/// The parts of `display_state()` the receiver loop needs.
struct DisplaySessionSnapshot {
    let id: String?
    let revision: Int64
    let config: DisplayStreamConfig?

    init(json: Data) throws {
        guard let object = try JSONSerialization.jsonObject(with: json) as? [String: Any] else {
            throw DisplayFailure("Invalid display state")
        }
        id = object["id"] as? String
        revision = (object["revision"] as? NSNumber)?.int64Value ?? 0
        config = try (object["config"] as? [String: Any]).map(DisplayStreamConfig.init)
    }
}
