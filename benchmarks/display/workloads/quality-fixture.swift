import AppKit
import ImageIO
import Darwin

final class RasterView: NSView {
    let raster: CGImage
    init(_ image: CGImage, frame: NSRect) { raster = image; super.init(frame: frame) }
    required init?(coder: NSCoder) { fatalError("Unsupported") }
    override func draw(_ rect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.interpolationQuality = .none
        context.draw(raster, in: bounds)
    }
}

func write(_ value: [String: Any], to path: String) throws {
    try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]).write(to: URL(fileURLWithPath: path), options: .atomic)
}

final class Fixture: NSObject, NSApplicationDelegate {
    var windows: [NSWindow] = []
    var signals: [DispatchSourceSignal] = []
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments
        if args.count == 3, args[1] == "--windows", let pid = Int32(args[2]) {
            let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
            let result = list.filter {
                ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid && ($0[kCGWindowLayer as String] as? Int) == 0
            }.map { ["id": $0[kCGWindowNumber as String] ?? 0, "bounds": $0[kCGWindowBounds as String] ?? [:]] }
            print(String(decoding: try! JSONSerialization.data(withJSONObject: result), as: UTF8.self))
            NSApp.terminate(nil); return
        }
        guard args.count == 4 else { exit(2) }
        let output = args[2], requested = args[3]
        do {
            guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw NSError(domain: "Invalid PNG", code: 1) }
            let external = NSScreen.screens.filter {
                let id = ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
                return CGDisplayIsBuiltin(id) == 0 && CGDisplayIsInMirrorSet(id) == 0
            }
            let selected = requested == "auto" && external.count == 1 ? external.first : external.first {
                String(($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0) == requested
            }
            guard let screen = selected else { throw NSError(domain: "Exactly one unmirrored external display required", code: 2) }
            let scale = screen.backingScaleFactor
            let size = NSSize(width: CGFloat(image.width) / scale, height: CGFloat(image.height) / scale)
            guard size.width <= screen.frame.width, size.height <= screen.frame.height,
                  size.width.rounded() == size.width, size.height.rounded() == size.height else {
                throw NSError(domain: "Raster must fit at integral backing scale", code: 3)
            }
            // Cover the streamed display so receiver capture contains only the
            // synthetic image and black background, never unrelated source apps.
            let backdrop = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false, screen: screen)
            backdrop.setFrame(screen.frame, display: true); backdrop.backgroundColor = .black
            backdrop.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
            backdrop.orderFrontRegardless(); windows.append(backdrop)
            let origin = NSPoint(x: floor(screen.frame.midX - size.width / 2), y: floor(screen.frame.midY - size.height / 2))
            let frame = NSRect(origin: origin, size: size)
            let target = NSWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false, screen: screen)
            target.setFrame(frame, display: true)
            target.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 2); target.colorSpace = .sRGB
            target.contentView = RasterView(image, frame: NSRect(origin: .zero, size: size))
            target.orderFrontRegardless(); windows.append(target)
            for value in [SIGTERM, SIGINT] {
                signal(value, SIG_IGN)
                let signalSource = DispatchSource.makeSignalSource(signal: value, queue: .main)
                signalSource.setEventHandler { NSApp.terminate(nil) }; signalSource.resume(); signals.append(signalSource)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                do {
                    let capture = Process(); capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                    capture.arguments = ["-x", "-o", "-l", String(target.windowNumber), "-t", "png", output + ".png"]
                    try capture.run(); capture.waitUntilExit()
                    guard capture.terminationStatus == 0 else { throw NSError(domain: "Synthetic window capture denied", code: 4) }
                    let observedFrame = target.frame
                    guard observedFrame.size == size, screen.frame.contains(observedFrame) else {
                        throw NSError(domain: "Observed synthetic window moved outside source display or resized", code: 5)
                    }
                    let metadata: [String: Any] = ["status": "ready", "window_id": target.windowNumber,
                        "display_id": (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0,
                        "display_logical_size": [screen.frame.width, screen.frame.height], "scale": scale,
                        "target_pixel_size": [image.width, image.height],
                        "target_in_display_top_left_px": [(observedFrame.minX - screen.frame.minX) * scale, (screen.frame.maxY - observedFrame.maxY) * scale],
                        "requested_target_in_display_top_left_px": [(frame.minX - screen.frame.minX) * scale, (screen.frame.maxY - frame.maxY) * scale],
                        "capture_scope": "Synthetic target window only; streamed display covered by owned black window"]
                    try write(metadata, to: output + ".json")
                } catch { try? write(["status": "failed", "error": String(describing: error)], to: output + ".json"); NSApp.terminate(nil) }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 15) { NSApp.terminate(nil) }
        } catch { try? write(["status": "failed", "error": String(describing: error)], to: output + ".json"); exit(1) }
    }
    func applicationWillTerminate(_ notification: Notification) { windows.forEach { $0.orderOut(nil) } }
}
let app = NSApplication.shared
let fixture = Fixture(); app.setActivationPolicy(.accessory); app.delegate = fixture; app.run()
