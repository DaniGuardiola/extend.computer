import AppKit
import Foundation
import CoreVideo
import CoreMedia
import ScreenCaptureKit
import Darwin

// Pipes alone are insufficient when another descendant inherits an endpoint.
// Tie each helper to the app process that created it, including abrupt exits.
final class ParentExitMonitor {
    private var timer: DispatchSourceTimer?
    init(onExit: @escaping () -> Void) {
        let owner = getppid()
        guard owner > 1 else { return }
        let source = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        source.schedule(deadline: .now() + .milliseconds(250), repeating: .milliseconds(250))
        source.setEventHandler { [weak self] in
            guard let self, getppid() != owner else { return }
            self.timer?.cancel()
            onExit()
        }
        timer = source; source.resume()
    }
    deinit { timer?.cancel() }
}

func report(_ error: Error) -> Never {
    DisplayMetrics.shared.flush()
    FileHandle.standardError.write(Data("\(error)\n".utf8)); exit(1)
}
private let outputWriter = CapturePacketWriter(write: { try FileHandle.standardOutput.write(contentsOf: $0) }, legacyFailure: { _ in exit(0) })
private let measureOutput = ProcessInfo.processInfo.environment["EXTEND_DISPLAY_METRICS"] != nil
func packet(_ kind: UInt8, _ payload: Data, cursorIdentity: UInt64? = nil, cursorShape: Bool = false) {
    var size = UInt32(payload.count).bigEndian
    var bytes = Data([kind]); withUnsafeBytes(of: &size) { bytes.append(contentsOf: $0) }; bytes.append(payload)
    let queued = measureOutput ? DispatchTime.now().uptimeNanoseconds : 0
    guard case .written(let acquired, let written) = outputWriter.packet(bytes, measure: measureOutput) else { return }
    // The video callback and cursor capture share this framed pipe. Record
    // contention separately from the write itself, after releasing its lock,
    // so diagnostics can distinguish encoder work from downstream backpressure.
    if measureOutput {
        let stage = kind == 1 ? "video" : kind == 3 ? "cursor" : "control"
        DisplayMetrics.shared.event("pipe_\(stage)_wait_us", bytes: Int((acquired - queued) / 1000))
        DisplayMetrics.shared.event("pipe_\(stage)_write_us", bytes: Int((written - acquired) / 1000))
        if kind == 3, let identity = cursorIdentity {
            let cursor = cursorShape ? "cursor_shape" : "cursor_position"
            DisplayMetrics.shared.event("\(cursor)_pipe_queued", timestamp: identity, atNanoseconds: queued)
            DisplayMetrics.shared.event("\(cursor)_pipe_acquired", timestamp: identity, atNanoseconds: acquired)
            DisplayMetrics.shared.event("\(cursor)_pipe_written", timestamp: identity, atNanoseconds: written)
        }
    }
}
func exact(_ input: FileHandle, _ length: Int) throws -> Data? {
    var data = Data()
    while data.count < length {
        guard let chunk = try input.read(upToCount: length - data.count), !chunk.isEmpty else {
            if data.isEmpty { return nil }; throw DisplayFailure("Truncated display packet.")
        }
        data.append(chunk)
    }
    return data
}
func framePacket(_ frame: VideoFrame) -> Data {
    var time = frame.timestamp.bigEndian
    var data = Data(); withUnsafeBytes(of: &time) { data.append(contentsOf: $0) }
    data.append(frame.keyframe ? 1 : 0); data.append(frame.bytes); return data
}
func parseFrame(_ data: Data) throws -> VideoFrame {
    guard data.count > 9 else { throw DisplayFailure("Empty display frame.") }
    let time = data.prefix(8).reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
    guard time <= UInt64(Int64.max), data[8] < 2 else { throw DisplayFailure("Invalid display frame metadata.") }
    return VideoFrame(timestamp: time, keyframe: data[8] == 1, bytes: Data(data.dropFirst(9)))
}
func integer(_ args: [String], _ index: Int, _ range: ClosedRange<Int>) throws -> Int {
    guard args.count > index, let value = Int(args[index]), range.contains(value) else { throw DisplayFailure("Invalid display configuration.") }
    return value
}
func configurationMetrics(width: Int, height: Int, fps: Int, bitrate: Int, retina: Bool) {
    for (name, value) in [("width", width), ("height", height), ("fps", fps), ("bitrate", bitrate), ("retina", retina ? 1 : 0)] {
        DisplayMetrics.shared.event("config_" + name, bytes: value)
    }
    // Settings acknowledgments must not precede their observable configuration.
    DisplayMetrics.shared.flush()
}
func selfTest(codec: VideoCodec = .h264) throws {
    guard try annexBNALs(Data([0,0,0,1,0x67,1,0,0,1,0x68,2])).count == 2 else { throw DisplayFailure("NAL parser failed.") }
    for invalid in [Data(), Data([0,0,1]), Data([1,2,3])] {
        var rejected = false
        do { _ = try annexBNALs(invalid) }
        catch is DisplayFailure { rejected = true }
        guard rejected else { throw DisplayFailure("Parser accepted invalid input.") }
    }
    let markerCounter = 0x5A3
    let image = try benchmarkMarkerTestImage(logicalWidth: 640, logicalHeight: 480, scale: 1, value: markerCounter)
    let marker = BenchmarkMarker(logicalWidth: 640, logicalHeight: 480, syntheticSelfTest: true)!
    let cpuCounter = marker.value(image), gpuCounter = marker.value(image, forceGPU: true)
    guard cpuCounter == markerCounter, gpuCounter == markerCounter else {
        throw DisplayFailure("Synthetic marker CPU/GPU crop orientation failed (CPU=\(String(describing: cpuCounter)), GPU=\(String(describing: gpuCounter))).")
    }
    let completed = DispatchSemaphore(value: 0)
    let lock = NSLock(); var frames = [VideoFrame](); var failure: Error?
    let encoder = try VideoEncoder(width: 640, height: 480, fps: 60, bitrate: 8_000_000, codec: codec,
        output: { frame in lock.lock(); frames.append(frame); lock.unlock(); completed.signal() },
        failure: { error in lock.lock(); failure = error; lock.unlock(); completed.signal() })
    try encoder.encode(image, timestamp: CMClockGetTime(CMClockGetHostTimeClock()))
    guard completed.wait(timeout: .now() + 5) == .success else { throw DisplayFailure("Hardware encode timed out.") }
    encoder.drain()
    encoder.pauseCapture(true)
    try encoder.encode(image, timestamp: CMClockGetTime(CMClockGetHostTimeClock()))
    lock.lock(); let pausedCount = frames.count; lock.unlock()
    guard pausedCount == 1 else { throw DisplayFailure("Paused capture encoded another frame.") }
    encoder.pauseCapture(false)
    guard completed.wait(timeout: .now() + 5) == .success else { throw DisplayFailure("Capture resume timed out.") }
    encoder.drain()
    if let failure { throw failure }
    guard let frame = frames.first, frame.keyframe else { throw DisplayFailure("Missing startup keyframe.") }
    let decoded = DispatchSemaphore(value: 0); var dimensions = CGSize.zero; var decodedMarker: Int?
    let decoder = VideoDecoder(codec: codec, benchmarkMarker: marker,
        output: { image, _ in
            dimensions = CGSize(width: CVPixelBufferGetWidth(image), height: CVPixelBufferGetHeight(image))
            decodedMarker = marker.value(image); decoded.signal()
        })
    for frame in frames { try decoder.decode(try parseFrame(framePacket(frame))) }
    decoder.drain()
    guard decoded.wait(timeout: .now() + 5) == .success, dimensions == CGSize(width: 640, height: 480) else {
        throw DisplayFailure("Hardware codec round trip failed.")
    }
    guard decodedMarker == markerCounter else { throw DisplayFailure("Decoded synthetic marker orientation/checksum failed.") }
    print("PASS: \(codec.rawValue) hardware encode/native decode, synthetic marker CPU/GPU crop, framing and NAL parsing; no screen content captured.")
}

@main struct DisplayMain {
    static func main() {
        do {
            let args = Array(CommandLine.arguments.dropFirst())
            if args == ["parent-exit-test"] {
                let monitor = ParentExitMonitor { exit(0) }
                print("ready"); fflush(stdout)
                DispatchQueue.global().asyncAfter(deadline: .now() + 30) { exit(2) }
                withExtendedLifetime(monitor) { RunLoop.current.run() }
                return
            }
            if args == ["self-test"] { try benchmarkMarkerSelfTest(); try cursorSelfTest(); try selfTest(); try selfTest(codec: .hevc); return }
            if args == ["self-test", "hevc"] { try selfTest(codec: .hevc); return }
            if args == ["status"] {
                print("capture=\(CGPreflightScreenCaptureAccess()) virtual=\(ECDisplaySupported())")
                return
            }
            if args == ["capabilities"] {
                // engine::helper::display::Abilities. Extending needs the virtual display API.
                let data = try JSONSerialization.data(withJSONObject: [
                    "version": 1, "source": ECDisplaySupported() ? ["mirror", "extend"] : ["mirror"],
                    "target": true, "viewer": true, "viewer_input": true,
                    "encode": ["hevc", "h264"], "decode": ["hevc", "h264"],
                    "cursor_send": true, "cursor_overlay": true
                ] as [String: Any], options: [.sortedKeys])
                print(String(decoding: data, as: UTF8.self)); return
            }
            if args == ["screen"] {
                guard let screen = NSScreen.main else { throw DisplayFailure("No display available.") }
                let data = try JSONSerialization.data(withJSONObject: [
                    "width": Int(screen.frame.width), "height": Int(screen.frame.height),
                    "usable_width": Int(floor(screen.frame.width - screen.safeAreaInsets.left - screen.safeAreaInsets.right)),
                    "usable_height": Int(floor(screen.frame.height - screen.safeAreaInsets.top - screen.safeAreaInsets.bottom)),
                    "scale": screen.backingScaleFactor
                ])
                print(String(decoding: data, as: UTF8.self)); return
            }
            if args == ["screens"] {
                let displays = NSScreen.screens.map { screen -> [String: Any] in
                    let id = (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
                    let bounds = CGDisplayBounds(id)
                    return ["id": id, "name": screen.localizedName, "x": Int(bounds.minX), "y": Int(bounds.minY),
                        "width": Int(bounds.width), "height": Int(bounds.height), "scale": screen.backingScaleFactor]
                }
                let data = try JSONSerialization.data(withJSONObject: displays)
                print(String(decoding: data, as: UTF8.self)); return
            }
            if args == ["request-permission"] { _ = CGRequestScreenCaptureAccess(); return }
            guard let command = args.first, ["extend", "mirror", "viewer"].contains(command) else { throw DisplayFailure("Choose extend, mirror or viewer.") }
            let width = try integer(args, 1, 640...4096), height = try integer(args, 2, 480...4096)
            let requestedFPS = try integer(args, 3, 30...120), requestedBitrate = try integer(args, 4, 1_000_000...80_000_000)
            let requestedRetina = args.count > 5 && args[5] == "retina"
            configurationMetrics(width: width, height: height, fps: requestedFPS, bitrate: requestedBitrate, retina: requestedRetina)
            let codec = args.count > 7 ? VideoCodec(rawValue: args[7]) : .h264
            guard let codec else { throw DisplayFailure("Invalid video codec.") }
            let separateCursor = args.count > 8 && args[8] == "cursor"
            DisplayMetrics.shared.event("codec", bytes: codec == .hevc ? 265 : 264)
            DisplayMetrics.shared.event("separate_cursor", bytes: separateCursor ? 1 : 0)
            let activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiated, .latencyCritical], reason: "Display connection")
            defer { ProcessInfo.processInfo.endActivity(activity) }
            let app = NSApplication.shared; app.setActivationPolicy(command == "viewer" ? .regular : .accessory)
            if command == "viewer" {
                let viewer = try DisplayViewer(width: width, height: height, separateCursor: separateCursor)
                viewer.configure(width: width, height: height, fps: requestedFPS)
                viewer.input = { packet(4, $0) }
                viewer.viewportEvidence = { bytes in
                    DispatchQueue.global(qos: .utility).async { packet(2, bytes) }
                }
                viewer.closed = { DisplayMetrics.shared.flush(); packet(2, Data("{\"type\":\"closed\"}".utf8)); exit(0) }
                viewer.renderedGPU = { packet(2, Data("{\"type\":\"render_ready\",\"ready\":true}".utf8)) }
                let countLock = NSLock(); var presented = 0
                viewer.presented = {
                    countLock.lock(); presented += 1; let count = presented; countLock.unlock()
                    if count == 1 || count % 60 == 0 { packet(2, Data("{\"type\":\"presented\",\"frames\":\(count)}".utf8)) }
                }
                let decoder = VideoDecoder(codec: codec, benchmarkMarker: BenchmarkMarker(logicalWidth: width, logicalHeight: height),
                    output: { image, timestamp in viewer.present(image, timestamp: timestamp) },
                    failure: { packet(2, Data("{\"type\":\"keyframe\"}".utf8)) })
                packet(0, Data("{\"ready\":true}".utf8))
                DispatchQueue.global(qos: .userInteractive).async {
                    do {
                        while let header = try exact(.standardInput, 5) {
                            let count = header.dropFirst().reduce(0) { ($0 << 8) | Int($1) }
                            guard count > 0, count <= 16 * 1024 * 1024,
                                  let payload = try exact(.standardInput, count) else { throw DisplayFailure("Invalid viewer packet.") }
                            if header[0] == 3 {
                                viewer.cursor(payload); continue
                            }
                            if header[0] == 2 {
                                let value = try JSONSerialization.jsonObject(with: payload) as? [String: Any]
                                if value?["type"] as? String == "input_state" {
                                    try viewer.inputState(payload); continue
                                }
                                if value?["type"] as? String == "viewport", let id = value?["id"] as? Int {
                                    DispatchQueue.main.async {
                                        let size = viewer.viewport
                                        let scale = viewer.window.backingScaleFactor
                                        let fullscreen = viewer.window.styleMask.contains(.fullScreen)
                                        var message: [String: Any] = ["type": "viewport", "id": id,
                                            "width": Int(floor(size.width)), "height": Int(floor(size.height)),
                                            "scale": scale, "fullscreen": fullscreen,
                                            "window_number": viewer.window.windowNumber]
                                        if let rect = viewer.viewportCG {
                                            message["viewport_cg"] = ["x": Double(rect.minX), "y": Double(rect.minY),
                                                "width": Double(rect.width), "height": Double(rect.height)]
                                        }
                                        if let data = try? JSONSerialization.data(withJSONObject: message) { packet(2, data) }
                                    }
                                    continue
                                }
                                if value?["type"] as? String == "configuration", let id = value?["id"] as? Int,
                                   let width = value?["width"] as? Int, let height = value?["height"] as? Int,
                                   let fps = value?["fps"] as? Int, (640...4096).contains(width),
                                   (480...4096).contains(height), (30...120).contains(fps) {
                                    DispatchQueue.main.async {
                                        viewer.configure(width: width, height: height, fps: fps)
                                        decoder.configureMarker(width: width, height: height)
                                        configurationMetrics(width: width, height: height, fps: fps,
                                            bitrate: value?["bitrate"] as? Int ?? requestedBitrate, retina: value?["retina"] as? Bool ?? requestedRetina)
                                        packet(2, Data("{\"type\":\"configured\",\"id\":\(id),\"ok\":true}".utf8))
                                    }
                                    continue
                                }
                                guard value?["type"] as? String == "stop" else { throw DisplayFailure("Invalid viewer control.") }
                                break
                            }
                            guard header[0] == 1, count > 9 else { throw DisplayFailure("Invalid viewer frame.") }
                            do {
                                let frame = try parseFrame(payload)
                                DisplayMetrics.shared.event("received", timestamp: frame.timestamp)
                                try decoder.decode(frame)
                            }
                            catch {
                                DisplayMetrics.shared.event("decode_error")
                                packet(2, Data("{\"type\":\"keyframe\"}".utf8))
                            }
                        }
                        decoder.drain(); DisplayMetrics.shared.flush(); exit(0)
                    } catch { report(error) }
                }
                let parent = ParentExitMonitor { exit(0) }
                withExtendedLifetime(parent) { app.run() }; return
            }
            let fps = try integer(args, 3, 30...120), bitrate = try integer(args, 4, 1_000_000...80_000_000)
            let retina = args.count > 5 && args[5] == "retina"
            CapturePacketWriter.prepareSenderPipe()
            guard width * (retina ? 2 : 1) <= 4096, height * (retina ? 2 : 1) <= 4096 else { throw DisplayFailure("Video capture size exceeds supported limit.") }
            let lifecycle = CaptureLifecycle(observe: { event in
                if event.outcome == .timedOut {
                    // The normal diagnostic sink might be blocked. Never wait
                    // on it at the independent shutdown deadline. This helper
                    // is exiting; only its stderr descriptor is changed.
                    let message = "Display cleanup incomplete: reason=\(event.reason.rawValue) phase=\(event.phase.rawValue) virtual_release=\(event.virtualRelease.rawValue).\n"
                    let flags = fcntl(STDERR_FILENO, F_GETFL)
                    if flags >= 0 && fcntl(STDERR_FILENO, F_SETFL, flags | O_NONBLOCK) == 0 {
                        message.withCString { _ = Darwin.write(STDERR_FILENO, $0, strlen($0)) }
                    }
                    return
                }
                DisplayMetrics.shared.event("capture_shutdown_\(event.outcome.rawValue)_\(event.reason.rawValue)", bytes: Int(event.exitCode))
                DisplayMetrics.shared.event("capture_shutdown_phase_\(event.phase.rawValue)")
                if event.outcome == .requested, let error = event.error {
                    try? FileHandle.standardError.write(contentsOf: Data("\(error)\n".utf8))
                } else if event.outcome == .failed {
                    let message = "Display cleanup incomplete: reason=\(event.reason.rawValue) phase=\(event.phase.rawValue) virtual_release=\(event.virtualRelease.rawValue)\(event.outcome == .failed ? " \(event.error ?? "")" : "").\n"
                    try? FileHandle.standardError.write(contentsOf: Data(message.utf8))
                }
                if event.outcome == .complete || event.outcome == .failed { DisplayMetrics.shared.flush() }
            }, terminate: { code in _exit(code) })
            outputWriter.useSender(lifecycle)
            let encoder = try VideoEncoder(width: width * (retina ? 2 : 1), height: height * (retina ? 2 : 1), fps: fps, bitrate: bitrate, codec: codec,
                output: { frame in packet(1, framePacket(frame)) },
                failure: { [weak lifecycle] in lifecycle?.requestStop(.encoderFailure, error: String(describing: $0)) })
            let capture = DisplayCapture(encoder: encoder, lifecycle: lifecycle,
                failure: { [weak lifecycle] in lifecycle?.requestStop(.captureFailure, error: String(describing: $0)) })
            let recordPhase: @Sendable (CaptureLifecycle.Phase) -> Void = { [weak lifecycle] in lifecycle?.record($0) }
            lifecycle.installCleanup { try await capture.stop(record: recordPhase) }
            lifecycle.enqueue(.starting) {
                try await capture.start(mode: command, width: width, height: height, retina: retina,
                    displayID: args.count > 6 && args[6] != "0" ? UInt32(args[6]) : nil, fps: fps, separateCursor: separateCursor)
                guard !lifecycle.isClosing else { return }
                packet(0, try JSONSerialization.data(withJSONObject: ["ready": true, "width": width, "height": height, "retina": retina, "target": try capture.inputTarget(configurationID: 0)]))
            }
            DispatchQueue.global().async {
                while let line = readLine() {
                    guard let data = line.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
                    if value["type"] as? String == "keyframe" { capture.requestKeyframe() }
                    if value["type"] as? String == "pause", let paused = value["value"] as? Bool { capture.pauseCapture(paused) }
                    if value["type"] as? String == "bitrate", let bitrate = value["value"] as? Int { try? capture.setBitrate(bitrate) }
                    if value["type"] as? String == "configuration", let id = value["id"] as? Int,
                       let width = value["width"] as? Int, let height = value["height"] as? Int,
                       let fps = value["fps"] as? Int, let bitrate = value["bitrate"] as? Int,
                       let retina = value["retina"] as? Bool {
                        lifecycle.enqueue(.configuring) {
                            do {
                                try await capture.configure(width: width, height: height, retina: retina, fps: fps, bitrate: bitrate)
                                guard !lifecycle.isClosing else { return }
                                configurationMetrics(width: width, height: height, fps: fps, bitrate: bitrate, retina: retina)
                                packet(2, try JSONSerialization.data(withJSONObject: ["type": "target", "target": try capture.inputTarget(configurationID: id)]))
                                packet(2, Data("{\"type\":\"configured\",\"id\":\(id),\"ok\":true}".utf8))
                            } catch {
                                guard !lifecycle.isClosing else { return }
                                packet(2, Data("{\"type\":\"configured\",\"id\":\(id),\"ok\":false}".utf8))
                                FileHandle.standardError.write(Data("Display settings: \(error)\n".utf8))
                            }
                        }
                    }
                    if value["type"] as? String == "stop" { lifecycle.requestStop(.stdinStop); return }
                }
                lifecycle.requestStop(.stdinEOF)
            }
            let parent = ParentExitMonitor {
                lifecycle.requestStop(.parentLost)
            }
            withExtendedLifetime((parent, lifecycle)) { app.run() }
        } catch { report(error) }
    }
}
