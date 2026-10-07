import AppKit
import WebKit

final class Workload: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    func applicationDidFinishLaunching(_ notification: Notification) {
        guard CommandLine.arguments.count == 3,
              let url = URL(string: CommandLine.arguments[1]),
              url.host == "127.0.0.1", url.port == 8765,
              let screen = NSScreen.screens.first(where: { $0.localizedName == CommandLine.arguments[2] }) else {
            print("Expected localhost workload and virtual display"); exit(1)
        }
        window = NSWindow(contentRect: NSRect(origin: .zero, size: screen.frame.size), styleMask: [.borderless], backing: .buffered, defer: false, screen: screen)
        window.setFrame(screen.frame, display: true)
        guard window.frame == screen.frame else { print("Workload frame mismatch"); exit(1) }
        let web = WKWebView(frame: NSRect(origin: .zero, size: screen.frame.size))
        window.contentView = web
        window.title = "Display benchmark workload"
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        web.load(URLRequest(url: url))
        print("Workload placed: \(Int(screen.frame.width))x\(Int(screen.frame.height)) scale \(screen.backingScaleFactor) frame \(window.frame)")
        fflush(stdout)
    }
}
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let delegate = Workload()
app.delegate = delegate
app.run()
