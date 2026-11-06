import ExpoModulesCore
import UIKit

// iOS bridge for the JS API in ../index.tsx. Names, arguments and JSON shapes
// mirror android/.../ExtendDisplayModule.kt so the screens stay platform-neutral.
// Every `TODO(engine)` returns the idle value the Android engine reports before
// anything has started; the engine wave replaces them with ExtendEngine calls.

private func unavailable(_ what: String) -> Exception {
  Exception(name: "ExtendUnavailable", description: "\(what) isn't available on iPhone yet")
}

private func json(_ value: Any) -> String {
  guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]),
        let text = String(data: data, encoding: .utf8) else { return "{}" }
  return text
}

private enum Store {
  static let defaults = UserDefaults.standard
  static let appearanceKey = "appearance.value"
  static let localNameKey = "engine-local.name"
  static let sharingPrefix = "engine-screen-sharing."
}

private enum LocalDevice {
  static func systemName() -> String {
    // iOS 16+ returns the model ("iPhone") without the device-name entitlement.
    let name = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
    return name.isEmpty ? UIDevice.current.model : name
  }
  static func name() -> String {
    Store.defaults.string(forKey: Store.localNameKey) ?? systemName()
  }
  static func info() -> [String: Any] {
    let bundle = Bundle.main.infoDictionary ?? [:]
    let saved = Store.defaults.string(forKey: Store.localNameKey)
    return [
      "name": saved ?? systemName(),
      "defaultName": systemName(),
      "customName": saved != nil,
      // TODO(engine): fingerprint from the engine identity.
      "fingerprint": "",
      "version": bundle["CFBundleShortVersionString"] as? String ?? "",
      "build": bundle["CFBundleVersion"] as? String ?? "",
    ]
  }
  static func setName(_ raw: String) throws {
    let name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    if name.isEmpty || name.utf8.count > 100 || name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) {
      throw Exception(name: "InvalidDeviceName", description: "Enter a device name up to 100 bytes")
    }
    if name == systemName() { Store.defaults.removeObject(forKey: Store.localNameKey) }
    else { Store.defaults.set(name, forKey: Store.localNameKey) }
    // TODO(engine): push the new name to the engine so peers see it.
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
// keep-awake. Info.plist sets UIViewControllerBasedStatusBarAppearance=NO (Expo
// default), so the application-level status bar API applies. The home indicator
// and edge deferral are answered by the window's root view controller, so that
// one instance gets an isa-swizzled subclass that reads `Immersive.enabled`.
@MainActor enum Immersive {
  static var enabled = false

  static func apply(_ on: Bool) {
    enabled = on
    UIApplication.shared.setStatusBarHidden(on, with: .fade)
    UIApplication.shared.isIdleTimerDisabled = on
    for scene in UIApplication.shared.connectedScenes {
      for window in (scene as? UIWindowScene)?.windows ?? [] {
        guard let root = window.rootViewController else { continue }
        install(root)
        root.setNeedsUpdateOfHomeIndicatorAutoHidden()
        root.setNeedsUpdateOfScreenEdgesDeferringSystemGestures()
      }
    }
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
      let hidesBase = original(hidesSel, BoolGetter.self), edgesBase = original(edgesSel, EdgeGetter.self)
      let childHomeBase = original(childHomeSel, Getter.self), childEdgesBase = original(childEdgesSel, Getter.self)
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
      add(made, hidesSel, hides, "B@:")
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
  private static let idleAccount: [String: Any] = [
    "server": "https://extend.computer", "email": NSNull(), "device_id": NSNull(), "devices": [],
    "browser_pending": false, "native_pending": false, "busy": false, "loaded": true,
    "storage_error": false, "directory_fresh": false, "error": NSNull(),
  ]
  private static let closedIncoming: [String: Any] = [
    "state": "closed", "port": 0, "code": NSNull(), "secondsRemaining": 0, "addresses": [], "request": NSNull(),
  ]

  public func definition() -> ModuleDefinition {
    Name("ExtendDisplay")

    OnCreate {
      let value = Store.defaults.string(forKey: Store.appearanceKey) ?? "system"
      if value != "system" { DispatchQueue.main.async { applyAppearance(value) } }
    }

    // MARK: Accounts
    AsyncFunction("getAccountState") { () -> String in json(Self.idleAccount) } // TODO(engine)
    AsyncFunction("resumeAccount") { } // TODO(engine)
    AsyncFunction("refreshAccount") { () -> String in json(Self.idleAccount) } // TODO(engine)
    AsyncFunction("startAccountBrowser") { () throws -> String in throw unavailable("Account sign-in") } // TODO(engine)
    AsyncFunction("startAccountPassword") { () throws in throw unavailable("Account sign-in") } // TODO(engine)
    AsyncFunction("cancelAccountSignIn") { () -> String in json(Self.idleAccount) } // TODO(engine)
    AsyncFunction("logoutAccount") { () -> String in json(Self.idleAccount) } // TODO(engine)
    AsyncFunction("configureAccount") { (_: String) throws -> String in throw unavailable("Changing the account server") } // TODO(engine)
    AsyncFunction("removeAccountDevice") { (_: String) throws -> String in throw unavailable("Removing account devices") } // TODO(engine)
    AsyncFunction("openAccountPage") { (_: String) throws in throw unavailable("Account pages") } // TODO(engine)
    Function("hasActiveConnection") { false } // TODO(engine)

    // MARK: Discovery and pairing
    Function("startDiscovery") { } // TODO(engine)
    Function("stopDiscovery") { } // TODO(engine)
    Function("getNearbyDevices") { () -> [String: Any] in ["state": "idle", "candidates": [[String: Any]]()] } // TODO(engine)
    AsyncFunction("startVisualPairing") { (_: String, _: String) throws -> String in throw unavailable("Nearby pairing") } // TODO(engine)
    Function("getVisualPairing") { (id: String) -> String in // TODO(engine)
      json(["id": id, "state": "closed", "request": NSNull(), "secondsRemaining": 0])
    }
    Function("respondVisualPairing") { (_: String, _: String, _: Bool) throws in throw unavailable("Nearby pairing") } // TODO(engine)
    Function("stopVisualPairing") { (_: String) in } // TODO(engine)
    Function("startIncomingPairing") { () -> String in json(Self.closedIncoming) } // TODO(engine)
    Function("getIncomingPairing") { () -> String in json(Self.closedIncoming) } // TODO(engine)
    Function("respondIncomingPairing") { (_: String, _: Bool) throws in throw unavailable("Pairing") } // TODO(engine)
    Function("stopIncomingPairing") { } // TODO(engine)
    Function("reserveManualPairing") { () throws -> String in throw unavailable("Manual pairing") } // TODO(engine)
    AsyncFunction("startManualPairing") { (_: String, _: String, _: String, _: String) throws -> String in throw unavailable("Manual pairing") } // TODO(engine)
    AsyncFunction("getManualPairing") { (id: String) -> String in json(["id": id, "state": "closed", "error": ""]) } // TODO(engine)
    Function("stopManualPairing") { (_: String) in } // TODO(engine)
    AsyncFunction("pairDevice") { (_: String, _: String, _: String) throws -> [String: Any] in throw unavailable("Pairing") } // TODO(engine)

    // MARK: Receiving
    Function("setReceivingEnabled") { (_: Bool) in } // TODO(engine)
    Function("getReceivingState") { () -> String in json(["enabled": false, "state": "stopped", "port": 0]) } // TODO(engine)
    // iOS has no public API for injecting input into other apps: report the
    // Android "off" shape and make the toggles no-ops (src/platform.ts hides them).
    Function("getInputReceivingState") { () -> String in
      json(["enabled": false, "permission": false, "active": false, "error": ""])
    }
    Function("setInputReceivingEnabled") { (_: Bool) in }
    Function("openInputReceivingSettings") { }
    Function("resumeReceiving") { } // TODO(engine)

    // MARK: Devices
    // TODO(engine): load or create the Keychain identity and return its real fingerprint/key.
    AsyncFunction("getEngineIdentity") { () -> [String: String] in ["fingerprint": "", "publicKey": ""] }
    AsyncFunction("getLocalDeviceInfo") { () -> [String: Any] in LocalDevice.info() }
    AsyncFunction("setLocalDeviceName") { (name: String) throws -> [String: Any] in
      try LocalDevice.setName(name)
      return LocalDevice.info()
    }
    AsyncFunction("getPeerDeviceInfo") { (_: String) throws -> [String: Any] in throw unavailable("Paired devices") } // TODO(engine)
    AsyncFunction("setPeerDeviceLabel") { (_: String, _: String) throws -> [String: Any] in throw unavailable("Paired devices") } // TODO(engine)
    AsyncFunction("setPeerDeviceAddress") { (_: String, _: String) throws -> [String: Any] in throw unavailable("Paired devices") } // TODO(engine)
    AsyncFunction("getPairedDevices") { () -> [[String: Any]] in [] } // TODO(engine)
    AsyncFunction("getDeviceAvailability") { (_: String) -> [String: Any] in // TODO(engine)
      ["state": "unavailable", "reason": "Not available on iPhone yet", "route": "lan"]
    }
    AsyncFunction("checkDevice") { (_: String) throws -> [String: Double] in throw unavailable("Checking devices") } // TODO(engine)
    AsyncFunction("forgetDevice") { (_: String) throws in throw unavailable("Forgetting devices") } // TODO(engine)
    AsyncFunction("getRemovedPeers") { () -> [[String: String]] in [] } // TODO(engine)
    AsyncFunction("dismissRemovedPeer") { (_: String) -> String in json([String: Any]()) } // TODO(engine)
    Function("getDeviceName") { LocalDevice.name() }

    // MARK: Control
    AsyncFunction("startControl") { (_: String) throws -> String in throw unavailable("Controlling devices") } // TODO(engine)
    Function("stopControl") { (_: String) in } // TODO(engine)
    Function("controlEvent") { (_: String, _: String) in } // TODO(engine)

    // MARK: Display receive
    Function("getLaunchSession") { () -> [String: Any] in ["available": false, "host": "Mac", "mode": "extend"] } // TODO(engine)
    Function("clearLaunchSession") { (_: String?) in } // TODO(engine)
    Function("getMediaState") { () -> String in json(["state": "idle"]) } // TODO(engine)
    AsyncFunction("getDisplayReceiverSettings") { (_: String) throws -> String in throw unavailable("Display settings") } // TODO(engine)
    AsyncFunction("requestDisplayReceiverSettings") { (_: String, _: Double, _: String) throws -> String in throw unavailable("Display settings") } // TODO(engine)

    // MARK: Screen sharing
    AsyncFunction("startScreenSharing") { (_: String) -> [String: Any] in // TODO(engine)
      ["started": false, "error": "Screen sharing isn't available on iPhone yet"]
    }
    Function("getScreenSharingState") { () -> String in json(["state": "idle", "error": "", "peer": ""]) } // TODO(engine)
    Function("getScreenSharingPreferences") { (peer: String) -> String in json(SharingOptions.read(peer)) }
    Function("setScreenSharingPreferences") { (peer: String, value: String) throws in try SharingOptions.save(peer, value) }
    AsyncFunction("updateScreenSharingPreferences") { (_: String, _: String) -> [String: Any] in // TODO(engine)
      ["applied": false, "error": "Screen sharing isn't running", "preferences": ""]
    }
    Function("stopScreenSharing") { } // TODO(engine)

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
      AsyncFunction("editingShortcut") { (_: ExtendDisplayView, _: String, _: String) -> Bool in false } // TODO(engine)
      Prop("touchEnabled") { (view: ExtendDisplayView, enabled: Bool) in view.touchEnabled = enabled }
      Prop("keyboardEnabled") { (view: ExtendDisplayView, enabled: Bool) in view.keyboardEnabled = enabled }
      Prop("sessionId") { (view: ExtendDisplayView, id: String?) in view.sessionId = id }
    }
    View(ExtendControlView.self) {
      ViewName("Control")
      Events("onStatus")
      AsyncFunction("editingShortcut") { (_: ExtendControlView, _: String, _: String) -> Bool in false } // TODO(engine)
      Prop("sessionId") { (view: ExtendControlView, id: String?) in view.sessionId = id }
      Prop("keyboard") { (view: ExtendControlView, enabled: Bool) in view.keyboard = enabled }
    }
  }
}

// TODO(engine): render the received stream (AVSampleBufferDisplayLayer) with a cursor layer.
class ExtendDisplayView: ExpoView {
  let onStatus = EventDispatcher()
  let onInputStatus = EventDispatcher()
  let onKeyboardStatus = EventDispatcher()
  var sessionId: String?
  var touchEnabled = false
  var keyboardEnabled = false

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    backgroundColor = .black
    clipsToBounds = true
  }
}

// TODO(engine): forward touch/keyboard to the control session.
class ExtendControlView: ExpoView {
  let onStatus = EventDispatcher()
  var sessionId: String?
  var keyboard = false

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    clipsToBounds = true
  }
}
