import AppKit
import CoreGraphics

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
    var value = 0
    var sourceReference = false
    override func draw(_ rect: NSRect) {
        NSColor.black.setFill(); bounds.fill()
        for (x,y,color) in [(0.0,140.0,NSColor.magenta),(364,140,.cyan),(0,0,.red),(364,0,.green)] {
            color.setFill(); NSRect(x:x,y:y,width:20,height:20).fill()
        }
        (sourceReference ? NSColor.white : .black).setFill();NSRect(x:184,y:140,width:16,height:20).fill()
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
        guard args.count == 5, let duration=Double(args[3]), duration>=1,duration<=600 else {exit(2)}
        scene=args[2];seconds=duration;output=args[4]
        guard ["warmup","static","scroll","panel","motion","recovery"].contains(scene) else {exit(2)}
        let candidates=NSScreen.screens.filter {CGDisplayIsBuiltin(screenID($0)) == 0}
        let target=args[1] == "auto" && candidates.count == 1 ? candidates[0] : NSScreen.screens.first {String(screenID($0)) == args[1]}
        guard let screen=target,let reference=NSScreen.screens.first(where:{CGDisplayIsBuiltin(screenID($0)) != 0}), screen != reference else {exit(2)}
        initial=signature()
        let window=NSWindow(contentRect:screen.frame,styleMask:[.borderless],backing:.buffered,defer:false,screen:screen)
        window.setFrame(screen.frame,display:true);view=Scene(frame:NSRect(origin:.zero,size:screen.frame.size));view.scene=scene
        window.contentView=view;window.orderFrontRegardless();windows.append(window)
        for (i,s) in [reference,screen].enumerated() {
            let x=i == 0 ? s.frame.minX+32 : s.frame.maxX-416
            let frame=NSRect(x:x-12,y:s.frame.minY+52,width:408,height:184)
            let win=NSWindow(contentRect:frame,styleMask:[.borderless],backing:.buffered,defer:false,screen:s)
            win.setFrame(frame,display:true);win.level = .floating
            // Keep colored fiducials isolated from similarly colored scene content.
            let backing=NSView(frame:NSRect(origin:.zero,size:frame.size));backing.wantsLayer=true;backing.layer?.backgroundColor=NSColor.black.cgColor
            let patch=Counter(frame:NSRect(x:12,y:12,width:384,height:160));patch.sourceReference = i == 0
            backing.addSubview(patch);win.contentView=backing;win.orderFrontRegardless()
            counters.append(patch);windows.append(win)
        }
        start=ProcessInfo.processInfo.systemUptime;last=start
        timer=Timer.scheduledTimer(withTimeInterval:1.0/60,repeats:true) { [self] _ in
            let now=ProcessInfo.processInfo.systemUptime
            ticks.append(now-last);last=now;view.elapsed=now-start
            if scene != "static" && scene != "warmup" {for patch in counters {patch.value=(patch.value+1)&4095;patch.display()}}
            view.display()
            if signature() != initial {finish(valid:false)}
            if now-start>=seconds {finish(valid:true)}
        }
        RunLoop.main.add(timer!,forMode:.common)
        print("ready");fflush(stdout)
    }
    func finish(valid:Bool) {
        timer?.invalidate();let end=ProcessInfo.processInfo.systemUptime
        let data:[String:Any]=["scene":scene,"start_ns":Int64(start*1e9),"end_ns":Int64(end*1e9),"duration_s":end-start,
                             "valid_geometry":valid,"source_callbacks":ticks.count,"source_callback_hz":Double(ticks.count)/(end-start),
                             "callback_gaps_ms":ticks.map {$0*1000},"screens":NSScreen.screens.map(screenData),"renderer":"appkit-campaign-v1"]
        do {try JSONSerialization.data(withJSONObject:data,options:[.sortedKeys]).write(to:URL(fileURLWithPath:output),options:[.atomic])} catch {exit(1)}
        NSApp.terminate(nil)
    }
}
let app=NSApplication.shared;app.setActivationPolicy(.accessory)
let delegate=App();app.delegate=delegate;app.run()
