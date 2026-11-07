import ExpoModulesCore
import UIKit

// iOS bridge for the JS API in ../index.tsx. Names, arguments and JSON shapes
// mirror android/.../ExtendDisplayModule.kt so the screens stay platform-neutral.
// Engine-backed features live in Core/ (ports of native/android/src/.../*.java).
// `TODO(accounts|sharing|engine)` stubs return the idle value Android reports
// before anything has started.

private func unavailable(_ what: String) -> Exception {
  Exception(name: "ExtendUnavailable", description: "\(what) isn't available on iPhone yet")
}

private func json(_ value: Any) -> String { JSONText.encode(value) }

/// Account errors reach JS with their plain message, like Android exceptions.
private func accounts<T>(_ body: () throws -> T) throws -> T {
  do { return try body() } catch let error as AccountError {
    throw Exception(name: "ExtendAccountError", description: error.description)
  } catch let error as EngineError {
    throw Exception(name: "ExtendError", description: error.message)
  }
}

/// Native input errors (stale shortcut token, held keys) reach JS as messages.
private func inputCall<T>(_ body: () throws -> T) throws -> T {
  do { return try body() } catch let error as InputError {
    throw Exception(name: "ExtendInputError", description: error.message)
  }
}

/// Engine errors reach JS with their plain message, like Android exceptions.
private func bridged<T>(_ body: () throws -> T) throws -> T {
  do { return try body() } catch let error as EngineError {
    throw Exception(name: "ExtendError", description: error.message)
  }
}

private enum Store {
  static let defaults = UserDefaults.standard
  static let appearanceKey = "appearance.value"
  static let sharingPrefix = "engine-screen-sharing."
}

enum LocalDevice {
  private static let lock = NSLock()
  private static var system: String?
  /// Must run on the main thread once before `systemName()` is read elsewhere.
  @MainActor static func prepare() {
    // iOS 16+ returns the model ("iPhone") without the device-name entitlement.
    let name = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
    lock.lock(); system = name.isEmpty ? UIDevice.current.model : name; lock.unlock()
  }
  static func systemName() -> String {
    lock.lock(); defer { lock.unlock() }
    return system ?? "iPhone"
  }
  static func name() -> String { LocalStore.customName ?? systemName() }
  static func info() throws -> [String: Any] {
    let identity = try Engine.identity()
    let bundle = Bundle.main.infoDictionary ?? [:]
    return [
      "name": name(),
      "defaultName": systemName(),
      "customName": LocalStore.customName != nil,
      "fingerprint": identity["fingerprint"] as? String ?? "",
      "version": bundle["CFBundleShortVersionString"] as? String ?? "",
      "build": bundle["CFBundleVersion"] as? String ?? "",
    ]
  }
  static func setName(_ raw: String) throws {
    try Engine.identity()
    let name = try Devices.normalizeName(raw)
    if name.isEmpty || name == systemName() {
      do { try LocalStore.setCustomName(nil) } catch { throw EngineError("Couldn't reset device name.") }
    } else {
      do { try LocalStore.setCustomName(name) } catch { throw EngineError("Couldn't save device name") }
    }
    try Engine.updateLocalDeviceName(self.name())
  }
}

private enum SharingOptions {
  static let defaults: [String: Any] = ["resolution": "1280", "fps": 60, "quality": "balanced", "codec": "h264"]
  static func validate(_ input: String) throws -> [String: Any] {
    guard input.count <= 1024, let data = input.data(using: .utf8),
          let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let resolution = value["resolution"] as? String, let quality = value["quality"] as? String,
          let codec = value["codec"] as? String, let fps = value["fps"] as? Int else {
      throw Exception(name: "InvalidSharingSettings", description: "Invalid screen sharing settings")
    }
    guard ["fit", "1280", "1920"].contains(resolution) else { throw Exception(name: "InvalidSharingSettings", description: "Choose a supported screen resolution") }
    guard [30, 60, 120].contains(fps) else { throw Exception(name: "InvalidSharingSettings", description: "Choose 30, 60 or 120 FPS") }
    guard ["responsive", "balanced", "detail"].contains(quality) else { throw Exception(name: "InvalidSharingSettings", description: "Choose a supported video quality") }
    guard ["h264", "hevc"].contains(codec) else { throw Exception(name: "InvalidSharingSettings", description: "Choose H.264 or HEVC") }
    return ["resolution": resolution, "fps": fps, "quality": quality, "codec": codec]
  }
  static func read(_ peer: String) -> [String: Any] {
    if let saved = Store.defaults.string(forKey: Store.sharingPrefix + peer), let value = try? validate(saved) { return value }
    return defaults
  }
  static func save(_ peer: String, _ value: String) throws {
    guard peer.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else {
      throw Exception(name: "InvalidPeer", description: "Invalid paired device")
    }
    Store.defaults.set(json(try validate(value)), forKey: Store.sharingPrefix + peer)
  }
}

@MainActor private func applyAppearance(_ value: String) {
  let style: UIUserInterfaceStyle = value == "light" ? .light : value == "dark" ? .dark : .unspecified
  for scene in UIApplication.shared.connectedScenes {
    (scene as? UIWindowScene)?.windows.forEach { $0.overrideUserInterfaceStyle = style }
  }
}

// Immersive display: status bar, home indicator and system edge gestures, plus
// keep-awake. Under the UIScene lifecycle the application-level status bar API
// is ignored, so Info.plist sets UIViewControllerBasedStatusBarAppearance=YES
// (app.json) and, like the home indicator and edge deferral, status bar
// visibility is answered by the window's root view controller: that one
// instance gets an isa-swizzled subclass that reads `Immersive.enabled`.
@MainActor enum Immersive {
  static var enabled = false

  static func apply(_ on: Bool) {
    enabled = on
    UIApplication.shared.isIdleTimerDisabled = on || KeepAwake.active
    for scene in UIApplication.shared.connectedScenes {
      for window in (scene as? UIWindowScene)?.windows ?? [] {
        guard let root = window.rootViewController else { continue }
        install(root)
        UIView.animate(withDuration: 0.2) { root.setNeedsStatusBarAppearanceUpdate() }
        root.setNeedsUpdateOfHomeIndicatorAutoHidden()
        root.setNeedsUpdateOfScreenEdgesDeferringSystemGestures()
        watchSafeArea(window)
        compensateStatusBar(window)
      }
    }
  }

  // iOS 26 leaves the window's safe area at its old status bar height after
  // the status bar shows or hides; it only catches up on the next scene
  // activation. Without the status bar, leaving the display screen put the
  // home screen header under it (measured on an iPhone SE: status bar 20 pt,
  // window top inset 0). Cover the missing height with the root view
  // controller's additional inset until the window reports it, re-checked
  // whenever the window's own inset changes.
  private static func compensateStatusBar(_ window: UIWindow) {
    guard let root = window.rootViewController, let bar = window.windowScene?.statusBarManager else { return }
    let shown = enabled || bar.isStatusBarHidden ? 0 : bar.statusBarFrame.height
    let missing = max(0, shown - window.safeAreaInsets.top)
    if root.additionalSafeAreaInsets.top != missing { root.additionalSafeAreaInsets.top = missing }
  }
  private final class SafeAreaWatcher: UIView {
    override func safeAreaInsetsDidChange() {
      super.safeAreaInsetsDidChange()
      if let window { MainActor.assumeIsolated { Immersive.compensateStatusBar(window) } }
    }
  }
  private static func watchSafeArea(_ window: UIWindow) {
    if window.subviews.contains(where: { $0 is SafeAreaWatcher }) { return }
    // Attached to the window itself, so it sees the window's inset without the compensation.
    let watcher = SafeAreaWatcher(frame: window.bounds)
    watcher.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    watcher.isUserInteractionEnabled = false
    watcher.isHidden = true
    window.insertSubview(watcher, at: 0)
  }

  private static let prefix = "ExtendImmersive_"
  private static func install(_ controller: UIViewController) {
    let base: AnyClass = object_getClass(controller)!
    let baseName = NSStringFromClass(base)
    if baseName.hasPrefix(prefix) { return }
    let name = prefix + baseName
    var subclass: AnyClass? = NSClassFromString(name)
    if subclass == nil, let made = objc_allocateClassPair(base, name, 0) {
      // Outside immersive mode every query falls through to the original class.
      typealias Getter = @convention(c) (UIViewController, Selector) -> UnsafeRawPointer?
      typealias BoolGetter = @convention(c) (UIViewController, Selector) -> Bool
      typealias EdgeGetter = @convention(c) (UIViewController, Selector) -> UInt
      func original<T>(_ selector: Selector, _: T.Type) -> T {
        unsafeBitCast(class_getMethodImplementation(base, selector), to: T.self)
      }
      let hidesSel = #selector(getter: UIViewController.prefersHomeIndicatorAutoHidden)
      let edgesSel = #selector(getter: UIViewController.preferredScreenEdgesDeferringSystemGestures)
      let childHomeSel = #selector(getter: UIViewController.childForHomeIndicatorAutoHidden)
      let childEdgesSel = #selector(getter: UIViewController.childForScreenEdgesDeferringSystemGestures)
      let statusSel = #selector(getter: UIViewController.prefersStatusBarHidden)
      let childStatusSel = #selector(getter: UIViewController.childForStatusBarHidden)
      let hidesBase = original(hidesSel, BoolGetter.self), edgesBase = original(edgesSel, EdgeGetter.self)
      let childHomeBase = original(childHomeSel, Getter.self), childEdgesBase = original(childEdgesSel, Getter.self)
      let statusBase = original(statusSel, BoolGetter.self), childStatusBase = original(childStatusSel, Getter.self)
      let hides: @convention(block) (UIViewController) -> Bool = { vc in
        MainActor.assumeIsolated { Immersive.enabled } || hidesBase(vc, hidesSel)
      }
      let edges: @convention(block) (UIViewController) -> UInt = { vc in
        MainActor.assumeIsolated { Immersive.enabled } ? UIRectEdge.all.rawValue : edgesBase(vc, edgesSel)
      }
      let childHome: @convention(block) (UIViewController) -> UnsafeRawPointer? = { vc in
        MainActor.assumeIsolated { Immersive.enabled } ? nil : childHomeBase(vc, childHomeSel)
      }
      let childEdges: @convention(block) (UIViewController) -> UnsafeRawPointer? = { vc in
        MainActor.assumeIsolated { Immersive.enabled } ? nil : childEdgesBase(vc, childEdgesSel)
      }
      let status: @convention(block) (UIViewController) -> Bool = { vc in
        MainActor.assumeIsolated { Immersive.enabled } || statusBase(vc, statusSel)
      }
      let childStatus: @convention(block) (UIViewController) -> UnsafeRawPointer? = { vc in
        MainActor.assumeIsolated { Immersive.enabled } ? nil : childStatusBase(vc, childStatusSel)
      }
      add(made, hidesSel, hides, "B@:")
      add(made, statusSel, status, "B@:")
      add(made, childStatusSel, childStatus, "@@:")
      add(made, childHomeSel, childHome, "@@:")
      add(made, childEdgesSel, childEdges, "@@:")
      add(made, edgesSel, edges, "Q@:")
      objc_registerClassPair(made)
      subclass = made
    }
    if let subclass { object_setClass(controller, subclass) }
  }
  private static func add(_ cls: AnyClass, _ selector: Selector, _ block: Any, _ types: String) {
    class_addMethod(cls, selector, imp_implementationWithBlock(block), types)
  }
}

public class ExtendDisplayModule: Module {
  private var nearby: NearbyDiscovery?
  private let incoming = IncomingPairing()
  private let visual = VisualPairing()
  private let manual = ManualPairing()

  private func discovery() -> NearbyDiscovery {
    if let nearby { return nearby }
    let made = NearbyDiscovery()
    nearby = made
    return made
  }
  private func connectionActive() -> Bool {
    let media = Engine.displayState()["state"] as? String ?? ""
    let control = Engine.controlState()["state"] as? String ?? ""
    return ScreenSharing.active ||
      ["offered", "connecting", "configuring", "streaming"].contains(media) || ["connecting", "connected"].contains(control)
  }
  private func requireIdle(_ message: String) throws {
    if connectionActive() { throw Exception(name: "ExtendBusy", description: message) }
  }
  /// Route for a session: USB carrier, internet relay, or the saved LAN hint.
  private func route(_ peer: String) throws -> String {
    let checked = try Devices.check(peer)
    switch checked["route"] as? String {
    case "usb": return "usb:\(checked["carrier_id"] as? String ?? "")"
    case "internet": return ""
    default:
      guard let address = LocalStore.peer(peer)?["address"] as? String, !address.isEmpty else {
        throw EngineError("Device address unavailable")
      }
      return address
    }
  }
  /// Foreground-only engine services: USB listener and availability.
  private static func foreground() {
    Engine.queue.async {
      do {
        try Engine.identity()
        try Engine.startUsbListener()
      } catch {
        NSLog("extend.computer: USB listener unavailable: \(error.localizedDescription)")
      }
      Receiving.resume()
    }
  }

  public func definition() -> ModuleDefinition {
    Name("ExtendDisplay")

    OnCreate {
      let value = Store.defaults.string(forKey: Store.appearanceKey) ?? "system"
      Engine.localName = { LocalDevice.name() }
      DispatchQueue.main.async {
        AppFocus.install()
        LocalDevice.prepare()
        Engine.deviceKind = UIDevice.current.userInterfaceIdiom == .pad ? "tablet" : "phone"
        if value != "system" { applyAppearance(value) }
        // Decoder capabilities turn on the display receiver (probed once, on main).
        let capabilities = try? DecoderCapabilities.json()
        Receiving.displayCapabilities = { capabilities }
        Self.foreground()
      }
    }
    OnAppBecomesActive {
      AppFocus.set(true)
      Engine.queue.async { SharedAccounts.resume() }
      self.nearby?.resume()
      Self.foreground()
    }
    OnAppEntersBackground {
      AppFocus.set(false)
      self.nearby?.suspend()
      self.incoming.stop(); self.visual.stop(); self.manual.stop()
      Engine.queue.async {
        Receiving.pause()
        Engine.stopUsbListener()
      }
    }
    OnDestroy {
      self.nearby?.stop(); self.nearby = nil
      self.incoming.stop(); self.visual.stop(); self.manual.stop()
    }

    // MARK: Accounts
    AsyncFunction("getAccountState") { () throws -> String in try accounts { try SharedAccounts.get().state() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("resumeAccount") { () throws in try accounts { try SharedAccounts.get().resume() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("refreshAccount") { () throws -> String in try accounts { try SharedAccounts.get().refresh() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("startAccountBrowser") { () throws -> String in
      try accounts {
        try AppFocus.require("Return to the app to sign in")
        return try SharedAccounts.get().startBrowser()
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("startAccountPassword") { () throws in
      try accounts {
        let account = try SharedAccounts.get()
        try account.prepare()
        try AppFocus.require("Return to the app to sign in")
        try account.startPassword()
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("cancelAccountSignIn") { () throws -> String in try accounts { try SharedAccounts.get().cancel() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("logoutAccount") { () throws -> String in try accounts { try SharedAccounts.get().logout() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("configureAccount") { (server: String) throws -> String in
      try accounts { try SharedAccounts.get().configure(server) }
    }.runOnQueue(Engine.queue)
    AsyncFunction("removeAccountDevice") { (id: String) throws -> String in
      try accounts { try SharedAccounts.get().removeDevice(id) }
    }.runOnQueue(Engine.queue)
    AsyncFunction("openAccountPage") { (page: String) throws in try accounts { try SharedAccounts.get().openPage(page) } }
      .runOnQueue(Engine.queue)
    Function("hasActiveConnection") { self.connectionActive() }

    // MARK: Discovery and pairing
    Function("startDiscovery") { self.discovery().start() }
    Function("stopDiscovery") { self.nearby?.stop() }
    Function("getNearbyDevices") { () -> [String: Any] in self.discovery().snapshot() }
    AsyncFunction("startVisualPairing") { (address: String, name: String) throws -> String in
      try bridged {
        try AppFocus.require("Return to the app to pair a device")
        try self.requireIdle("Stop the current connection before pairing")
        return try self.visual.start(address: address, name: name, localName: LocalDevice.name())
      }
    }.runOnQueue(Engine.queue)
    Function("getVisualPairing") { (id: String) throws -> String in try bridged { try self.visual.state(id: id) } }
    Function("respondVisualPairing") { (id: String, request: String, allow: Bool) throws in
      try bridged {
        try AppFocus.require("Return to the app to confirm pairing")
        try self.visual.respond(id: id, request: request, allow: allow)
      }
    }
    Function("stopVisualPairing") { (id: String) throws in try bridged { try self.visual.stop(id: id) } }
    Function("startIncomingPairing") { () throws -> String in
      try bridged {
        try self.requireIdle("Stop the current connection before pairing")
        return try self.incoming.start(name: LocalDevice.name())
      }
    }
    Function("getIncomingPairing") { () throws -> String in try bridged { try self.incoming.state() } }
    Function("respondIncomingPairing") { (request: String, allow: Bool) throws in
      try bridged {
        try AppFocus.require("Return to the app to confirm pairing")
        try Engine.respondIncoming(request, allow)
      }
    }
    Function("stopIncomingPairing") { self.incoming.stop() }
    Function("reserveManualPairing") { () throws -> String in
      try bridged {
        try self.requireIdle("Stop the current connection before pairing")
        return try self.manual.reserve()
      }
    }
    AsyncFunction("startManualPairing") { (id: String, address: String, code: String, name: String) throws -> String in
      try bridged {
        try self.requireIdle("Stop the current connection before pairing")
        return try self.manual.start(id: id, address: address, code: code, name: name)
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("getManualPairing") { (id: String) throws -> String in try bridged { try self.manual.state(id: id) } }
    Function("stopManualPairing") { (id: String) in self.manual.stop(id: id) }
    AsyncFunction("pairDevice") { (address: String, code: String, name: String) throws -> [String: Any] in
      try bridged {
        try self.requireIdle("Stop the current connection before pairing")
        return try Devices.pair(address: address, code: code, name: name)
      }
    }.runOnQueue(Engine.queue)

    // MARK: Receiving
    Function("setReceivingEnabled") { (enabled: Bool) throws in try bridged { try Receiving.setEnabled(enabled) } }
    Function("getReceivingState") { () -> String in Receiving.state() }
    // iOS has no public API for injecting input into other apps: report the
    // Android "off" shape and make the toggles no-ops (src/platform.ts hides them).
    Function("getInputReceivingState") { () -> String in
      json(["enabled": false, "permission": false, "active": false, "error": ""])
    }
    Function("setInputReceivingEnabled") { (_: Bool) in }
    Function("openInputReceivingSettings") { }
    Function("resumeReceiving") { Engine.queue.async { Receiving.resume() } }

    // MARK: Devices
    AsyncFunction("getEngineIdentity") { () throws -> [String: String] in
      try bridged {
        let identity = try Engine.identity()
        return ["fingerprint": identity["fingerprint"] as? String ?? "", "publicKey": identity["public_key"] as? String ?? ""]
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("getLocalDeviceInfo") { () throws -> [String: Any] in try bridged { try LocalDevice.info() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("setLocalDeviceName") { (name: String) throws -> [String: Any] in
      try bridged {
        try AppFocus.require("Return to the app to change device name")
        try LocalDevice.setName(name)
        return try LocalDevice.info()
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("getPeerDeviceInfo") { (peer: String) throws -> [String: Any] in try bridged { try Devices.info(peer) } }
      .runOnQueue(Engine.queue)
    AsyncFunction("setPeerDeviceLabel") { (peer: String, label: String) throws -> [String: Any] in
      try bridged {
        try AppFocus.require("Return to the app to change device label")
        return try Devices.setLabel(peer, label)
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("setPeerDeviceAddress") { (peer: String, address: String) throws -> [String: Any] in
      try bridged {
        try AppFocus.require("Return to the app to change device address")
        return try Devices.setAddress(peer, address)
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("getPairedDevices") { () throws -> [[String: Any]] in try bridged { try Devices.list() } }
      .runOnQueue(Engine.queue)
    AsyncFunction("getDeviceAvailability") { (peer: String) throws -> [String: Any] in
      if !AppFocus.active { return ["state": "unavailable", "reason": "Return to the app to check availability"] }
      return try bridged {
        var result = try DeviceAvailability.read(peer)
        if result["route"] == nil { result["route"] = "lan" }
        return result
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("checkDevice") { (peer: String) throws -> [String: Double] in
      try bridged {
        try self.requireIdle("Stop the current connection before probing another device")
        let result = try Devices.check(peer)
        return ["rttMs": (result["rtt_ms"] as? NSNumber)?.doubleValue ?? 0]
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("forgetDevice") { (peer: String) throws in
      try bridged {
        try self.requireIdle("Stop the current connection before forgetting a device")
        try Devices.forget(peer)
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("getRemovedPeers") { () throws -> [[String: String]] in
      try bridged {
        try Engine.identity()
        return try Engine.removedPeers().map { ["peer": $0.key, "name": $0.value] }
      }
    }.runOnQueue(Engine.queue)
    AsyncFunction("dismissRemovedPeer") { (peer: String) throws -> String in
      try bridged {
        try Engine.identity()
        return json(try Engine.dismissRemovedPeer(peer))
      }
    }.runOnQueue(Engine.queue)
    Function("getDeviceName") { LocalDevice.name() }

    // MARK: Control
    AsyncFunction("startControl") { (peer: String) throws -> String in
      try bridged { json(try Engine.startControl(peer, try self.route(peer))) }
    }.runOnQueue(Engine.queue)
    Function("stopControl") { (id: String) in Engine.stopControl(id) }
    Function("controlEvent") { (id: String, event: String) throws in try bridged { try Engine.controlEvent(id, event) } }

    // MARK: Display receive
    Function("getLaunchSession") { () -> [String: Any] in
      let media = Engine.displayState()
      guard ["offered", "connecting", "streaming"].contains(media["state"] as? String ?? ""),
            let peer = media["peer"] as? String else {
        return ["available": false, "host": "Mac", "mode": "extend"]
      }
      let saved = LocalStore.peer(peer)?["name"] as? String
      let name = saved?.isEmpty == false ? saved! : ((try? Engine.peerNames())?[peer] ?? "Paired device")
      return ["available": true, "host": name, "mode": (media["config"] as? JSON)?["mode"] as? String ?? "extend",
              "id": media["id"] as? String ?? ""]
    }
    Function("clearLaunchSession") { (id: String?) in if let id { Engine.cancelDisplay(id) } }
    Function("getMediaState") { () -> String in json(Engine.displayState()) }
    AsyncFunction("getDisplayReceiverSettings") { (id: String) throws -> String in
      try bridged { json(try Engine.displayReceiverSettings(id)) }
    }
    AsyncFunction("requestDisplayReceiverSettings") { (id: String, revision: Double, settings: String) throws -> String in
      try bridged {
        try AppFocus.require("Return to the display to change settings")
        return json(try Engine.requestDisplayReceiverSettings(id, Int64(revision), settings))
      }
    }

    // MARK: Screen sharing
    // Capture runs in the ReplayKit broadcast extension (Sharing/, targets/broadcast).
    AsyncFunction("startScreenSharing") { (peer: String) -> [String: Any] in
      ScreenSharing.start(peer: peer, preferences: SharingOptions.read(peer))
    }.runOnQueue(Engine.queue)
    Function("getScreenSharingState") { () -> String in ScreenSharing.state() }
    Function("getScreenSharingPreferences") { (peer: String) -> String in json(SharingOptions.read(peer)) }
    Function("setScreenSharingPreferences") { (peer: String, value: String) throws in try SharingOptions.save(peer, value) }
    AsyncFunction("updateScreenSharingPreferences") { (peer: String, value: String) async -> [String: Any] in
      let result = await ScreenSharing.update(peer: peer, value: value)
      if result["applied"] as? Bool == true, let saved = result["preferences"] as? String { try? SharingOptions.save(peer, saved) }
      return result
    }
    Function("stopScreenSharing") { ScreenSharing.stop() }

    // MARK: Window
    Function("setImmersive") { (enabled: Bool) in
      DispatchQueue.main.async { Immersive.apply(enabled) }
    }
    Function("getAppearance") { () -> String in Store.defaults.string(forKey: Store.appearanceKey) ?? "system" }
    Function("setAppearance") { (value: String) throws in
      guard ["system", "light", "dark"].contains(value) else {
        throw Exception(name: "InvalidAppearance", description: "Appearance must be system, light or dark")
      }
      Store.defaults.set(value, forKey: Store.appearanceKey)
      DispatchQueue.main.async { applyAppearance(value) }
    }

    // MARK: Views
    View(ExtendDisplayView.self) {
      Events("onStatus", "onInputStatus", "onKeyboardStatus")
      AsyncFunction("editingShortcut") { (view: ExtendDisplayView, token: String, action: String) throws -> Bool in
        try inputCall { try view.editingShortcut(token: token, action: action) }
      }.runOnQueue(.main)
      Prop("touchEnabled") { (view: ExtendDisplayView, enabled: Bool) in view.touchEnabled = enabled }
      Prop("keyboardEnabled") { (view: ExtendDisplayView, enabled: Bool) in view.keyboardEnabled = enabled }
      Prop("sessionId") { (view: ExtendDisplayView, id: String?) in view.sessionId = id }
    }
    View(ExtendControlView.self) {
      ViewName("Control")
      Events("onStatus")
      AsyncFunction("editingShortcut") { (view: ExtendControlView, token: String, action: String) throws -> Bool in
        try inputCall { try view.editingShortcut(token: token, action: action) }
      }.runOnQueue(.main)
      Prop("sessionId") { (view: ExtendControlView, id: String?) in view.sessionId = id }
      Prop("keyboard") { (view: ExtendControlView, enabled: Bool) in view.keyboard = enabled }
    }
  }
}
