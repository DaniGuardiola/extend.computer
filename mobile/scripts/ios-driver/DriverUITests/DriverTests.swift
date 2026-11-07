import XCTest

/// Runs the action list in the DRIVER_ACTIONS environment variable, one action per line:
///   launch <bundle-id>       launch (terminating first) and focus an app
///   activate <bundle-id>     bring an app forward without relaunching
///   tap <label>              tap the first element whose label or identifier matches exactly
///   tapcontains <text>       tap the first element whose label contains text
///   tapxy <x> <y>            tap normalized screen coordinates (0...1)
///   doubletapxy <x> <y>      double tap normalized screen coordinates
///   holddrag <x1> <y1> <x2> <y2> [hold]   press (default 0.8 s) then drag
///   touches <path>|<path>... multi-finger gesture; path = "x,y,t x,y,t ..." in
///                            normalized screen coordinates and seconds (down, moves, lift)
///   type <text>              type into the focused element (\\uXXXX, \\n, \\b escapes)
///   key <name> [cmd|shift|alt|ctrl...]  hardware key: left right up down return delete escape tab or one character
///   field <label>=<text>     replace the text of the text field with that label
///   swipe up|down|left|right
///   orient portrait|left|right   rotate the device (landscape left/right)
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

  static func unescape(_ s: String) -> String {
    var out = "", it = Array(s), i = 0
    while i < it.count {
      if it[i] == "\\", i + 1 < it.count {
        let c = it[i + 1]
        if c == "n" { out += "\n"; i += 2; continue }
        if c == "b" { out += "\u{8}"; i += 2; continue }
        if c == "u", i + 5 < it.count, let v = UInt32(String(it[(i + 2)...(i + 5)]), radix: 16), let u = Unicode.Scalar(v) {
          out.unicodeScalars.append(u); i += 6; continue
        }
      }
      out.append(it[i]); i += 1
    }
    return out
  }

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
        case "doubletapxy":
          let n = arg.split(separator: " ").compactMap { Double($0) }
          app.coordinate(withNormalizedOffset: CGVector(dx: n[0], dy: n[1])).doubleTap()
        case "holddrag":
          let n = arg.split(separator: " ").compactMap { Double($0) }
          let from = app.coordinate(withNormalizedOffset: CGVector(dx: n[0], dy: n[1]))
          from.press(forDuration: n.count > 4 ? n[4] : 0.8, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: n[2], dy: n[3])))
        case "touches":
          let size = XCUIScreen.main.screenshot().image.size
          let paths: [[NSNumber]] = arg.split(separator: "|").map { path in
            path.split(separator: " ").flatMap { triple -> [NSNumber] in
              let v = triple.split(separator: ",").compactMap { Double($0) }
              return [NSNumber(value: v[0] * size.width), NSNumber(value: v[1] * size.height), NSNumber(value: v[2])]
            }
          }
          var error: NSString?
          if !DriverSynthesizeTouches(paths, &error) { XCTFail("touches: \(error ?? "")") }
        case "type": app.typeText(Self.unescape(arg))
        case "key":
          let n = arg.split(separator: " ").map(String.init)
          let names: [String: String] = ["left": XCUIKeyboardKey.leftArrow.rawValue, "right": XCUIKeyboardKey.rightArrow.rawValue,
            "up": XCUIKeyboardKey.upArrow.rawValue, "down": XCUIKeyboardKey.downArrow.rawValue, "return": XCUIKeyboardKey.return.rawValue,
            "delete": XCUIKeyboardKey.delete.rawValue, "escape": XCUIKeyboardKey.escape.rawValue, "tab": XCUIKeyboardKey.tab.rawValue]
          var flags: XCUIElement.KeyModifierFlags = []
          for m in n.dropFirst() {
            switch m { case "cmd": flags.insert(.command); case "shift": flags.insert(.shift); case "alt": flags.insert(.option); case "ctrl": flags.insert(.control); default: break }
          }
          app.typeKey(names[n[0]] ?? Self.unescape(n[0]), modifierFlags: flags)
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
        case "orient":
          XCUIDevice.shared.orientation = arg == "left" ? .landscapeLeft : arg == "right" ? .landscapeRight : .portrait
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
