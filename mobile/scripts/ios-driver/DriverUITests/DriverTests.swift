import XCTest

/// Runs the action list in the DRIVER_ACTIONS environment variable, one action per line:
///   launch <bundle-id>       launch (terminating first) and focus an app
///   activate <bundle-id>     bring an app forward without relaunching
///   tap <label>              tap the first element whose label or identifier matches exactly
///   tapcontains <text>       tap the first element whose label contains text
///   tapxy <x> <y>            tap normalized screen coordinates (0...1)
///   type <text>              type into the focused element
///   field <label>=<text>     replace the text of the text field with that label
///   swipe up|down|left|right
///   wait <seconds>
///   waitfor <label> [secs]   wait for an element to exist
///   shot <name>              attach a screenshot
///   dump <name>              attach the focused app's accessibility tree
/// Failures are recorded but later actions still run.
final class DriverTests: XCTestCase {
  var app = XCUIApplication(bundleIdentifier: "com.apple.springboard")

  func attachShot(_ name: String) {
    let a = XCTAttachment(screenshot: XCUIScreen.main.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a)
  }
  func attachDump(_ name: String) {
    let a = XCTAttachment(string: app.debugDescription); a.name = name; a.lifetime = .keepAlways; add(a)
  }
  func element(_ p: NSPredicate) -> XCUIElement { app.descendants(matching: .any).matching(p).firstMatch }

  func testRun() throws {
    continueAfterFailure = true
    let script = ProcessInfo.processInfo.environment["DRIVER_ACTIONS"] ?? ""
    for raw in script.components(separatedBy: "\n") {
      let line = raw.trimmingCharacters(in: .whitespaces)
      if line.isEmpty || line.hasPrefix("#") { continue }
      let parts = line.split(separator: " ", maxSplits: 1).map(String.init)
      let cmd = parts[0], arg = parts.count > 1 ? parts[1] : ""
      XCTContext.runActivity(named: line) { _ in
        switch cmd {
        case "launch": app = XCUIApplication(bundleIdentifier: arg); app.terminate(); app.launch()
        case "activate": app = XCUIApplication(bundleIdentifier: arg); app.activate()
        case "tap", "tapcontains":
          let p = cmd == "tap" ? NSPredicate(format: "label == %@ OR identifier == %@", arg, arg)
                               : NSPredicate(format: "label CONTAINS %@", arg)
          let e = element(p)
          if e.waitForExistence(timeout: 5) { e.tap() } else { XCTFail("not found: \(arg)") }
        case "tapxy":
          let n = arg.split(separator: " ").compactMap { Double($0) }
          app.coordinate(withNormalizedOffset: CGVector(dx: n[0], dy: n[1])).tap()
        case "type": app.typeText(arg)
        case "field":
          let parts = arg.split(separator: "=", maxSplits: 1).map(String.init)
          let field = app.textFields[parts[0]].firstMatch
          if field.waitForExistence(timeout: 5) {
            field.tap()
            let old = field.value as? String ?? ""
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: old.count + 4) + (parts.count > 1 ? parts[1] : ""))
          } else { XCTFail("not found: \(parts[0])") }
        case "swipe":
          switch arg { case "up": app.swipeUp(); case "down": app.swipeDown(); case "left": app.swipeLeft(); default: app.swipeRight() }
        case "wait": Thread.sleep(forTimeInterval: Double(arg) ?? 1)
        case "waitfor":
          let n = arg.split(separator: " ").map(String.init)
          let secs = n.count > 1 ? Double(n.last!) ?? 10 : 10
          let label = n.count > 1 && Double(n.last!) != nil ? n.dropLast().joined(separator: " ") : arg
          if !element(NSPredicate(format: "label == %@ OR identifier == %@", label, label)).waitForExistence(timeout: secs) { XCTFail("timeout: \(label)") }
        case "shot": attachShot(arg)
        case "dump": attachDump(arg)
        default: XCTFail("unknown action \(cmd)")
        }
      }
    }
  }
}
