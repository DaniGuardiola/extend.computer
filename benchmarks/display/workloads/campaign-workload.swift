import AppKit
import CoreGraphics
import Darwin
import CryptoKit

func moveBenchmarkPointer(_ point: CGPoint, method: String) -> Bool {
    if method == "warp" { return CGWarpMouseCursorPosition(point) == .success }
    guard method == "mouse-events", let event = CGEvent(mouseEventSource:nil,mouseType:.mouseMoved,
        mouseCursorPosition:point,mouseButton:.left) else { return false }
    if let previous = CGEvent(source:nil)?.location {
        event.setIntegerValueField(.mouseEventDeltaX,value:Int64((point.x-previous.x).rounded()))
        event.setIntegerValueField(.mouseEventDeltaY,value:Int64((point.y-previous.y).rounded()))
    }
    event.post(tap:.cghidEventTap)
    return true
}

// Logical Quartz coordinates, bounded away from screen edges and timing patches.
func cursorPoint(_ bounds: CGRect, elapsed: Double) -> CGPoint {
    let angle = elapsed * 2 * Double.pi / 3
    return CGPoint(x: bounds.minX + bounds.width * (0.5 + 0.3 * sin(angle)),
                   y: bounds.minY + bounds.height * (0.5 + 0.25 * sin(2 * angle)))
}
if CommandLine.arguments == [CommandLine.arguments[0], "--cursor-path-selftest"] {
    let bounds = CGRect(x: -1728, y: -103, width: 1728, height: 1085)
    for tick in 0..<3600 {
        let point = cursorPoint(bounds, elapsed: Double(tick) / 120)
        precondition(bounds.insetBy(dx: bounds.width * 0.19, dy: bounds.height * 0.24).contains(point))
    }
    precondition(abs(cursorPoint(bounds, elapsed: 0).x - cursorPoint(bounds, elapsed: 3).x) < 0.001)
    print("Cursor path self-test passed."); exit(0)
}

func screenID(_ screen: NSScreen) -> CGDirectDisplayID {
    (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! NSNumber).uint32Value
}
func screenData(_ screen: NSScreen) -> [String: Any] {
    let id = screenID(screen), mode = CGDisplayCopyDisplayMode(id)
    return ["id": id, "builtin": CGDisplayIsBuiltin(id) != 0, "name": screen.localizedName,
            "x": screen.frame.minX, "y": screen.frame.minY, "width": screen.frame.width, "height": screen.frame.height,
            "scale": screen.backingScaleFactor, "pixel_width": mode?.pixelWidth ?? 0,
            "pixel_height": mode?.pixelHeight ?? 0, "refresh_hz": mode?.refreshRate ?? 0,
            "mirrored": CGDisplayIsInMirrorSet(id) != 0]
}
func checksum(_ value: Int) -> Int {
    var crc = 0
    for bit in (0..<12).reversed() {
        let feedback = ((crc >> 3) & 1) ^ ((value >> bit) & 1)
        crc = (crc << 1) & 15
        if feedback != 0 { crc ^= 3 }
    }
    return crc
}
final class Counter: NSView {
    private var cursorTracking: NSTrackingArea?
    var cursorUpdateCount = 0, mouseMoveCount = 0
    var value = 0
    var sourceReference = false
    var opticalCursor = false
    var cursorLane = 0
    var cursorEvent: Int?
    var cursorX: Double?
    override func resetCursorRects() {
        if opticalCursor { addCursorRect(bounds, cursor: opticalPointer) }
    }
    override func updateTrackingAreas() {
        if let cursorTracking { removeTrackingArea(cursorTracking) }
        cursorTracking = nil
        if opticalCursor {
            let area = NSTrackingArea(rect: .zero,
                options: [.cursorUpdate, .mouseMoved, .activeAlways, .inVisibleRect],
                owner: self, userInfo: nil)
            addTrackingArea(area); cursorTracking = area
        }
        super.updateTrackingAreas()
    }
    override func cursorUpdate(with event: NSEvent) {
        cursorUpdateCount += 1
        if opticalCursor { opticalPointer.set() } else { super.cursorUpdate(with: event) }
    }
    override func mouseMoved(with event: NSEvent) {
        mouseMoveCount += 1
        if opticalCursor { opticalPointer.set() } else { super.mouseMoved(with: event) }
    }
    override func draw(_ rect: NSRect) {
        NSColor.black.setFill(); bounds.fill()
        for (x,y,color) in [(0.0,140.0,NSColor.magenta),(364,140,.cyan),(0,0,.red),(364,0,.green)] {
            color.setFill(); NSRect(x:x,y:y,width:20,height:20).fill()
        }
        (sourceReference ? NSColor.white : .black).setFill();NSRect(x:184,y:140,width:16,height:20).fill()
        if opticalCursor && sourceReference {
            if let event = cursorEvent {
                drawCursorBadge(event, at: NSPoint(x:Double(cursorLaneX[cursorLane])-48,y:132),scale:2)
            } else {
                NSColor.yellow.setFill()
                NSRect(x:(cursorX ?? Double(cursorLaneX[cursorLane]))-8,y:132,width:16,height:16).fill()
            }
        }
        let word = (value << 4) | checksum(value)
        for bit in 0..<16 {
            for row in 0..<2 {
                ((((word >> (15-bit)) & 1) != 0) != (row == 1) ? NSColor.white : .black).setFill()
                NSRect(x:32+Double(bit)*20,y:32+Double(row)*48,width:20,height:48).fill()
            }
        }
        // Fixed high-contrast strip for paired optical quality comparison.
        for (group,step) in [2,4,8,16].enumerated() {
            for x in stride(from:0,to:80,by:step) {
                ((x/step)%2 == 0 ? NSColor.white : .black).setFill()
                NSRect(x:32+Double(group*80+x),y:4,width:Double(step),height:20).fill()
            }
        }
    }
}
let cursorLaneX = [70, 130, 250, 310]
func drawCursorBadge(_ event: Int, at origin: NSPoint, scale:Double=1) {
    (scale == 1 ? NSColor.yellow : NSColor.black).setFill(); NSRect(origin:origin,size:NSSize(width:48*scale,height:32*scale)).fill()
    let word = (event << 4) | checksum(event)
    for bit in 0..<12 {
        ((word >> (11-bit)) & 1 == 1 ? NSColor.white : .black).setFill()
        NSRect(x:origin.x+(4+Double(bit%4)*10)*scale,y:origin.y+(4+Double(bit/4)*8)*scale,width:10*scale,height:8*scale).fill()
    }
    if scale > 1 {
        // Sparse border landmarks preserve strict lane isolation even though
        // the reference cells are larger than the unchanged native cursor.
        NSColor.yellow.setFill()
        for x in [2.0,46.0] { for y in [4.0,16.0,28.0] {
            NSRect(x:origin.x+(x-2)*scale,y:origin.y+(y-2)*scale,width:4*scale,height:4*scale).fill()
        }}
    }
}
func cursorRasterHash(_ cursor: NSCursor) -> String? {
    var rect = NSRect(origin: .zero, size: cursor.image.size)
    guard let image = cursor.image.cgImage(forProposedRect: &rect, context: nil, hints: nil),
          let context = CGContext(data: nil, width: 48, height: 32, bitsPerComponent: 8, bytesPerRow: 48 * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
          let bytes = context.data else { return nil }
    context.interpolationQuality = .none
    context.draw(image, in: CGRect(x: 0, y: 0, width: 48, height: 32))
    return SHA256.hash(data: Data(bytes: bytes, count: 48 * 32 * 4)).map { String(format: "%02x", $0) }.joined()
}
func codedCursor(_ event: Int) -> NSCursor {
    let image=NSImage(size:NSSize(width:48,height:32));image.lockFocus()
    drawCursorBadge(event,at:.zero);image.unlockFocus()
    return NSCursor(image:image,hotSpot:NSPoint(x:24,y:16))
}
var opticalPointer: NSCursor = {
    let image = NSImage(size: NSSize(width: 16, height: 16))
    image.lockFocus(); NSColor.yellow.setFill(); NSRect(x: 0, y: 0, width: 16, height: 16).fill(); image.unlockFocus()
    return NSCursor(image: image, hotSpot: NSPoint(x: 8, y: 8))
}()
final class WorkloadWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}
final class Scene: NSView {
    var scene = "static", elapsed = 0.0
    override func draw(_ rect: NSRect) {
        NSColor(calibratedWhite:0.94,alpha:1).setFill(); bounds.fill()
        let w=bounds.width,h=bounds.height
        let font=NSFont.monospacedSystemFont(ofSize:18,weight:.regular)
        let style:[NSAttributedString.Key:Any]=[.font:font,.foregroundColor:NSColor.black]
        if scene == "motion" || scene == "recovery" {
            for i in 0..<60 {
                let x=(Double(i)*37+elapsed*160).truncatingRemainder(dividingBy:max(1,w))
                let y=(Double(i)*53+elapsed*90).truncatingRemainder(dividingBy:max(1,h))
                NSColor(calibratedHue:Double(i%12)/12,saturation:0.75,brightness:0.8,alpha:1).setFill()
                NSRect(x:x,y:y,width:70,height:70).fill()
            }
        } else {
            let offset=scene == "scroll" ? (elapsed*180).truncatingRemainder(dividingBy:900) : 0
            for i in 0..<100 {
                let y=h-40-Double(i)*28+offset
                if y < 0 || y > h {continue}
                if i%2 == 1 {NSColor(calibratedWhite:0.88,alpha:1).setFill();NSRect(x:0,y:y-2,width:w,height:28).fill()}
                ("\(i)  Display benchmark — Il1 O0 []{} 0123456789 fixed content and speed" as NSString).draw(at:NSPoint(x:30,y:y),withAttributes:style)
            }
            if scene == "panel" {
                let x=(w-450)/2+sin(elapsed*1.2)*max(0,(w-500)/3)
                NSColor(calibratedWhite:0.16,alpha:1).setFill();NSRect(x:x,y:h/3,width:440,height:260).fill()
                ("Moving panel" as NSString).draw(at:NSPoint(x:x+25,y:h/3+210),withAttributes:[.font:font,.foregroundColor:NSColor.white])
            }
        }
    }
}
final class App: NSObject,NSApplicationDelegate {
    var windows:[NSWindow]=[], counters:[Counter]=[], view:Scene!, timer:Timer?
    var start=0.0, ticks:[Double]=[], last=0.0, output="", seconds=0.0, initial="", scene=""
    var cursorTimer: Timer?, cursorOriginal: CGPoint?, cursorBounds = CGRect.zero
    var cursorTicks: [Double] = [], cursorLast = 0.0, cursorErrors = 0, cursorRestored = false, finished = false
    var cancellationSignals: [DispatchSourceSignal] = []
    var opticalLane = -1
    var opticalSteps = 0
    var opticalObserved: [[String: Any]] = []
    var cursorInputMethod = "warp"
    var patchSides = ["left", "right"]
    var receiverPatchLeft: Double {
        patchSides[1] == "left" ? cursorBounds.minX + 32 : cursorBounds.maxX - 416
    }
    var opticalPending: Int?
    // Event zero warms the cursor/reference drawing path before the measured clock.
    // Its observed dwell is not a measured 250 ms interval.
    func applyOpticalEvent(_ event: Int) -> Bool {
        let lane = event % 4
        let bounds = cursorBounds
        let point = CGPoint(x: receiverPatchLeft + Double(cursorLaneX[lane]),
            y: bounds.maxY - (52 + 12 + 148))
        opticalPointer = codedCursor(event)
        for patch in counters {
            patch.window?.invalidateCursorRects(for:patch)
            patch.resetCursorRects()
        }
        opticalPointer.set()
        if !moveBenchmarkPointer(point,method:cursorInputMethod) { cursorErrors += 1; finish(valid: false); return false }
        opticalPointer.set()
        opticalPending = event
        opticalLane = lane; opticalSteps += 1
        counters[0].cursorLane = lane
        counters[0].cursorEvent = event
        for patch in counters { patch.display() }
        return true
    }
    func restoreCursor() {
        cursorTimer?.invalidate(); cursorTimer = nil
        if ["cursor-optical", "cursor-sweep"].contains(scene) { NSCursor.arrow.set() }
        if let original = cursorOriginal { cursorRestored = CGWarpMouseCursorPosition(original) == .success; cursorOriginal = nil }
    }
    func applicationWillTerminate(_ notification: Notification) { restoreCursor() }
    func signature()->String { NSScreen.screens.map {"\(screenID($0)):\($0.frame):\($0.backingScaleFactor)"}.joined(separator:"|") }
    func applicationDidFinishLaunching(_ note:Notification) {
        let args=CommandLine.arguments
        if args.count == 2 && args[1] == "--context" {
            let data=try! JSONSerialization.data(withJSONObject:NSScreen.screens.map(screenData),options:[.sortedKeys])
            print(String(data:data,encoding:.utf8)!);exit(0)
        }
        if args.count == 3 && args[1] == "--focus-hook" && args[2].hasPrefix("computer.extend.benchmark.focus.") {
            DistributedNotificationCenter.default().postNotificationName(Notification.Name(args[2]),object:nil,userInfo:nil,deliverImmediately:true)
            DispatchQueue.main.asyncAfter(deadline:.now()+1) { NSApp.terminate(nil) }
            return
        }
        if args.count == 3 && args[1] == "--activate", let pid = Int32(args[2]),
           let target = NSRunningApplication(processIdentifier:pid) {
            target.activate(options:[.activateAllWindows,.activateIgnoringOtherApps])
            DispatchQueue.main.asyncAfter(deadline:.now()+1) { NSApp.terminate(nil) }
            return
        }
        if args.count == 3 && args[1] == "--windows", let pid = Int32(args[2]) {
            let all = CGWindowListCopyWindowInfo(.optionOnScreenOnly,kCGNullWindowID) as? [[String:Any]] ?? []
            let windows = all.filter { ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
            let data = try! JSONSerialization.data(withJSONObject: windows.map { ["bounds":$0[kCGWindowBounds as String] ?? [:],"onscreen":$0[kCGWindowIsOnscreen as String] ?? false] })
            print(String(data:data,encoding:.utf8)!);exit(0)
        }
        guard (5...8).contains(args.count), let duration=Double(args[3]), duration>=1,duration<=600 else {exit(2)}
        scene=args[2];seconds=duration;output=args[4]
        var options = Array(args.dropFirst(5))
        if let first = options.first, ["warp", "mouse-events"].contains(first) {
            cursorInputMethod = options.removeFirst()
        }
        if !options.isEmpty {
            guard options.count == 2, options[0] == "--patch-sides",
                  ["left,right", "right,left"].contains(options[1]) else { exit(2) }
            patchSides = options[1].components(separatedBy: ",")
        }
        guard ["warp","mouse-events"].contains(cursorInputMethod),
              cursorInputMethod != "mouse-events" || (["cursor","cursor-optical","cursor-sweep"].contains(scene) && CGPreflightPostEventAccess()) else { exit(3) }
        if scene == "cursor-optical" && duration>60 { fputs("Coded cursor phases must not exceed 60 seconds.\n",stderr);exit(2) }
        guard ["warmup","static","scroll","panel","motion","recovery","cursor","cursor-optical","cursor-sweep"].contains(scene) else {exit(2)}
        let candidates=NSScreen.screens.filter {CGDisplayIsBuiltin(screenID($0)) == 0}
        let target=args[1] == "auto" && candidates.count == 1 ? candidates[0] : NSScreen.screens.first {String(screenID($0)) == args[1]}
        guard let screen=target,let reference=NSScreen.screens.first(where:{CGDisplayIsBuiltin(screenID($0)) != 0}), screen != reference else {exit(2)}
        if scene == "cursor" || ["cursor-optical", "cursor-sweep"].contains(scene) {
            guard let original = CGEvent(source: nil)?.location else { exit(2) }
            cursorOriginal = original; cursorBounds = CGDisplayBounds(screenID(screen))
            for value in [SIGTERM, SIGINT] {
                signal(value, SIG_IGN)
                let source = DispatchSource.makeSignalSource(signal: value, queue: .main)
                source.setEventHandler { [weak self] in self?.finish(valid: false) }
                source.resume(); cancellationSignals.append(source)
            }
        }
        initial=signature()
        let window=WorkloadWindow(contentRect:screen.frame,styleMask:[.borderless],backing:.buffered,defer:false,screen:screen)
        window.setFrame(screen.frame,display:true);view=Scene(frame:NSRect(origin:.zero,size:screen.frame.size));view.scene=scene
        window.contentView=view;window.orderFrontRegardless();windows.append(window)
        if ["cursor-optical", "cursor-sweep"].contains(scene) { window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true) }
        for (i,s) in [reference,screen].enumerated() {
            let x=patchSides[i] == "left" ? s.frame.minX+32 : s.frame.maxX-416
            let patchHeight=scene == "cursor-optical" ? 200.0 : 168.0
            let frame=NSRect(x:x-12,y:s.frame.minY+52,width:408,height:patchHeight+24)
            let win=WorkloadWindow(contentRect:frame,styleMask:[.borderless],backing:.buffered,defer:false,screen:s)
            win.setFrame(frame,display:true);win.level = .floating
            win.acceptsMouseMovedEvents = true
            // Keep colored fiducials isolated from similarly colored scene content.
            let backing=NSView(frame:NSRect(origin:.zero,size:frame.size));backing.wantsLayer=true;backing.layer?.backgroundColor=NSColor.black.cgColor
            let patch=Counter(frame:NSRect(x:12,y:12,width:384,height:patchHeight));patch.sourceReference = i == 0
            patch.opticalCursor = ["cursor-optical", "cursor-sweep"].contains(scene)
            backing.addSubview(patch);win.contentView=backing;win.orderFrontRegardless()
            counters.append(patch);windows.append(win)
            if patch.opticalCursor && i == 1 { win.makeKeyAndOrderFront(nil) }
        }
        if scene == "cursor-optical" && !applyOpticalEvent(0) { return }
        // Let AppKit complete window activation and initial cursor-rectangle setup
        // before the phase clock starts. Readiness is published only afterward.
        DispatchQueue.main.asyncAfter(deadline:.now()+0.5) { [self] in
        if ["cursor-optical", "cursor-sweep"].contains(scene), let patch = counters.last {
            patch.window?.makeKeyAndOrderFront(nil)
            patch.window?.makeFirstResponder(patch)
        }
        start=ProcessInfo.processInfo.systemUptime;last=start
        if scene == "cursor" || scene == "cursor-sweep" {
            cursorLast = start
            cursorTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 120, repeats: true) { [self] _ in
                let now = ProcessInfo.processInfo.systemUptime
                cursorTicks.append(now - cursorLast); cursorLast = now
                let point: CGPoint
                if scene == "cursor-sweep" {
                    let phase = (now - start).truncatingRemainder(dividingBy: 2)
                    let x = 40 + 280 * (phase < 1 ? phase : 2 - phase)
                    point = CGPoint(x: receiverPatchLeft + x,
                                    y: cursorBounds.maxY - (52 + 12 + 140))
                    counters[0].cursorX = x
                    opticalPointer.set()
                } else { point = cursorPoint(cursorBounds, elapsed: now - start) }
                if !moveBenchmarkPointer(point,method:cursorInputMethod) {
                    cursorErrors += 1; finish(valid: false)
                }
            }
            RunLoop.main.add(cursorTimer!, forMode: .common)
        }
        timer=Timer.scheduledTimer(withTimeInterval:1.0/60,repeats:true) { [self] _ in
            let now=ProcessInfo.processInfo.systemUptime
            if scene == "cursor-optical" {
                if let pending = opticalPending {
                    let current = NSCursor.currentSystem ?? .arrow
                    let expected = cursorRasterHash(opticalPointer), observed = cursorRasterHash(current)
                    let equal = expected != nil && expected == observed && current.image.size == opticalPointer.image.size && current.hotSpot == opticalPointer.hotSpot
                    opticalObserved.append(["event": pending, "expected_raster_sha256": expected ?? "", "observed_raster_sha256": observed ?? "", "matches": equal,
                        "observed_size": [current.image.size.width, current.image.size.height], "observed_hotspot": [current.hotSpot.x, current.hotSpot.y], "app_active": NSApp.isActive, "observed_point": [CGEvent(source:nil)?.location.x ?? -999, CGEvent(source:nil)?.location.y ?? -999], "patch_mouse": counters.map { ["cursor_updates": $0.cursorUpdateCount, "mouse_moves": $0.mouseMoveCount, "window_key": $0.window!.isKeyWindow, "window_number": $0.window!.windowNumber, "x":$0.convert($0.window!.mouseLocationOutsideOfEventStream,from:nil).x,"y":$0.convert($0.window!.mouseLocationOutsideOfEventStream,from:nil).y] }])
                    opticalPending = nil
                    if !equal { cursorErrors += 1; finish(valid: false, reason:"Source cursor raster did not remain applied"); return }
                }
                let event = 1 + Int((now-start)/0.25)
                guard event < 256 else { finish(valid:false); return }
                let lane = event % 4
                if lane != opticalLane {
                    if !applyOpticalEvent(event) { return }
                }
            }
            ticks.append(now-last);last=now;view.elapsed=now-start
            if scene != "static" && scene != "warmup" {for patch in counters {patch.value=(patch.value+1)&4095;patch.display()}}
            view.display()
            if signature() != initial {finish(valid:false)}
            if now-start>=seconds {finish(valid:true)}
        }
        RunLoop.main.add(timer!,forMode:.common)
        print("ready");fflush(stdout)
        }
    }
    func finish(valid:Bool, reason:String? = nil) {
        guard !finished else { return }; finished = true
        restoreCursor()
        timer?.invalidate();let end=ProcessInfo.processInfo.systemUptime
        var data:[String:Any]=["scene":scene,"patch_sides":patchSides,"start_ns":Int64(start*1e9),"end_ns":Int64(end*1e9),"duration_s":end-start,
                             "valid_geometry":signature() == initial,"valid_fixture":valid,"failure_reason":reason ?? (valid ? "" : "Fixture interrupted or operation failed"),"source_callbacks":ticks.count,"source_callback_hz":Double(ticks.count)/(end-start),
                             "callback_gaps_ms":ticks.map {$0*1000},"screens":NSScreen.screens.map(screenData),"renderer":"appkit-campaign-v1"]
        if scene == "cursor" {
            data["cursor"] = ["pattern": "bounded-figure-eight-v1", "requested_hz": 120,
                "movement_calls": cursorTicks.count, "movement_hz": Double(cursorTicks.count)/(end-start),
                "movement_gaps_ms": cursorTicks.map { $0 * 1000 }, "errors": cursorErrors,
                "position_restored": cursorRestored,
                "shape_visibility": "not exercised", "scope": "source pointer motion; not input-to-photon latency"] as [String: Any]
        }
        if scene == "cursor-optical" {
            data["cursor"] = ["pattern": "coded-four-lane-v5", "step_period_s": 0.25,
                "preparation_event_id":0,"first_measured_event_id":1,"preparation_s":0.5,
                "center_y_logical":148,"reference_origin_y_logical":132,
                "reference_center_y_logical":164,"reference_shape_logical_size":[96,64],
                "event_bits":8,"checksum_bits":4,"shape_logical_size":[48,32],
                "steps": opticalSteps, "observed_events": opticalObserved, "errors": cursorErrors, "position_restored": cursorRestored,
                "scope": "unique cursor shape-and-position events versus physical source reference; excludes hardware input latency"] as [String: Any]
        }
        if scene == "cursor-sweep" {
            data["cursor"] = ["pattern": "horizontal-yellow-sweep-v1", "requested_hz": 120,
                "period_s": 2, "speed_logical_pixels_s": 280,
                "movement_calls": cursorTicks.count, "movement_hz": Double(cursorTicks.count)/(end-start),
                "movement_gaps_ms": cursorTicks.map { $0 * 1000 }, "errors": cursorErrors,
                "position_restored": cursorRestored,
                "scope": "continuous physical source reference and receiver pointer; excludes hardware input latency"] as [String: Any]
        }
        if var cursor = data["cursor"] as? [String:Any] { cursor["input_method"]=cursorInputMethod
            if cursorInputMethod == "warp", let calls = cursor["movement_calls"] {
                cursor["warp_calls"] = calls; cursor["warp_hz"] = cursor["movement_hz"]
                cursor["warp_gaps_ms"] = cursor["movement_gaps_ms"]
            }
            data["cursor"]=cursor }
        do {try JSONSerialization.data(withJSONObject:data,options:[.sortedKeys]).write(to:URL(fileURLWithPath:output),options:[.atomic])} catch {exit(1)}
        NSApp.terminate(nil)
    }
}
let app=NSApplication.shared;app.setActivationPolicy(.accessory)
let delegate=App();app.delegate=delegate;app.run()
