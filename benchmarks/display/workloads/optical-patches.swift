import AppKit

// Optical counter fixture. Two windows receive the same counter in one source
// process. Camera measurements compare physical scanout, not host clocks.
func checksum(_ value: Int) -> Int {
    var crc = 0
    for bit in (0..<12).reversed() {
        let feedback = ((crc >> 3) & 1) ^ ((value >> bit) & 1)
        crc = (crc << 1) & 15
        if feedback != 0 { crc ^= 3 }
    }
    return crc
}
final class Patch: NSView {
    var counter = 0
    override func draw(_ rect: NSRect) {
        NSColor.black.setFill(); bounds.fill()
        let corners: [(CGFloat, CGFloat, NSColor)] = [(0,140,.magenta),(364,140,.cyan),(0,0,.red),(364,0,.green)]
        for (x,y,color) in corners { color.setFill(); NSRect(x:x,y:y,width:20,height:20).fill() }
        let word = (counter << 4) | checksum(counter)
        for bit in 0..<16 {
            let high = ((word >> (15-bit)) & 1) != 0
            for row in 0..<2 {
                (high != (row == 1) ? NSColor.white : NSColor.black).setFill()
                NSRect(x:32+CGFloat(bit)*20,y:32+CGFloat(row)*48,width:20,height:48).fill()
            }
        }
    }
}
final class App: NSObject, NSApplicationDelegate {
    var windows: [NSWindow] = []; var patches: [Patch] = []; var timer: Timer?
    var counter = 0
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments
        if args.count == 2 && args[1] == "--screens" {
            for s in NSScreen.screens { print("\(s.localizedName): \(s.frame) scale \(s.backingScaleFactor)") }
            exit(0)
        }
        guard args.count == 4, let seconds = Double(args[2]), seconds > 0, seconds <= 600,
              let hz = Double(args[3]), hz > 0, hz <= 60,
              let main = NSScreen.screens.first else { exit(2) }
        let screens: [NSScreen]
        if args[1] == "calibration" { screens = [main, main] }
        else {
            let target = args[1] == "auto" && NSScreen.screens.count == 2 ? NSScreen.screens[1] : NSScreen.screens.first(where: {$0.localizedName == args[1]})
            guard let virtual = target, virtual != main else { print("Target display unavailable"); exit(2) }
            screens = [main, virtual]
        }
        for (i, screen) in screens.enumerated() {
            let x = i == 0 ? screen.frame.minX + 32 : screen.frame.maxX - 416
            let frame = NSRect(x:x,y:screen.frame.minY+64,width:384,height:160)
            let window = NSWindow(contentRect:frame,styleMask:[.borderless],backing:.buffered,defer:false,screen:screen)
            window.level = .floating
            let patch = Patch(frame:NSRect(origin:.zero,size:frame.size))
            window.contentView = patch; window.setFrame(frame,display:true); window.orderFrontRegardless()
            windows.append(window); patches.append(patch)
            print("Patch \(i): \(screen.localizedName) \(frame)")
        }
        fflush(stdout)
        timer = Timer.scheduledTimer(withTimeInterval:1/hz,repeats:true) { [self] _ in
            counter = (counter+1)&4095
            for patch in patches { patch.counter=counter; patch.display() }
        }
        RunLoop.main.add(timer!,forMode:.common)
        DispatchQueue.main.asyncAfter(deadline:.now()+seconds) { NSApp.terminate(nil) }
    }
}
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = App(); app.delegate = delegate; app.run()
