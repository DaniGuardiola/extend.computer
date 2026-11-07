import Foundation

// Contract between the app (BroadcastController) and the ReplayKit broadcast
// upload extension (targets/broadcast). This file is compiled into both: the
// ExtendDisplay pod builds it from here, and plugins/with-broadcast-extension.js
// copies it into the generated extension target.
//
// The two processes share no memory. The app writes a request (peer, address,
// preferences) to App Group defaults and shows the system picker; the extension
// reads it when the broadcast starts, publishes its state back to the group and
// posts a Darwin notification on every change. Darwin notifications carry no
// payload, so the defaults are the source of truth and notifications are hints.

enum BroadcastGroup {
  /// `ExtendAppGroup` / `ExtendBroadcastExtension` are written into both
  /// Info.plists by the config plugin, so no identifier is hard-coded here.
  static var identifier: String? { Bundle.main.object(forInfoDictionaryKey: "ExtendAppGroup") as? String }
  static var extensionIdentifier: String? { Bundle.main.object(forInfoDictionaryKey: "ExtendBroadcastExtension") as? String }
  static var defaults: UserDefaults? { identifier.flatMap { UserDefaults(suiteName: $0) } }

  static let requestKey = "broadcast.request"
  static let consumedKey = "broadcast.consumed"
  static let stateKey = "broadcast.state"
  static let updateKey = "broadcast.update"
  static let updateResultKey = "broadcast.update-result"
  /// `{session, claimed}` while the extension holds the USB port (see `UsbHandoff`).
  static let usbOwnerKey = "broadcast.usb-owner"

  /// A request older than this is never started by the extension, so starting a
  /// broadcast from Control Center later can't silently reuse an old target.
  static let requestLifetime: TimeInterval = 120
  /// The extension refreshes `updated` at least this often while running; a
  /// state older than `staleAfter` means the extension was killed.
  static let heartbeat: TimeInterval = 1
  static let staleAfter: TimeInterval = 4

  enum Note: String {
    case state = "computer.extend.broadcast.state"
    case stop = "computer.extend.broadcast.stop"
    case update = "computer.extend.broadcast.update"
    /// The extension claimed or released the USB port.
    case usb = "computer.extend.broadcast.usb"
  }

  static func post(_ note: Note) {
    CFNotificationCenterPostNotification(CFNotificationCenterGetDarwinNotifyCenter(),
      CFNotificationName(note.rawValue as CFString), nil, nil, true)
  }

  /// Darwin observers can't capture context, so each process keeps one handler
  /// per name and dispatches through this table.
  private static var handlers: [String: () -> Void] = [:]
  private static let lock = NSLock()
  static func observe(_ note: Note, _ handler: @escaping () -> Void) {
    lock.lock(); let fresh = handlers[note.rawValue] == nil; handlers[note.rawValue] = handler; lock.unlock()
    guard fresh else { return }
    CFNotificationCenterAddObserver(CFNotificationCenterGetDarwinNotifyCenter(), nil, { _, _, name, _, _ in
      guard let name = name?.rawValue as String? else { return }
      BroadcastGroup.lock.lock(); let handler = BroadcastGroup.handlers[name]; BroadcastGroup.lock.unlock()
      handler?()
    }, note.rawValue as CFString, nil, .deliverImmediately)
  }
  static func ignore(_ note: Note) {
    lock.lock(); let present = handlers.removeValue(forKey: note.rawValue) != nil; lock.unlock()
    if present {
      CFNotificationCenterRemoveObserver(CFNotificationCenterGetDarwinNotifyCenter(), nil,
        CFNotificationName(note.rawValue as CFString), nil)
    }
  }

  static func read(_ key: String) -> [String: Any]? {
    guard let text = defaults?.string(forKey: key), let data = text.data(using: .utf8) else { return nil }
    return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
  }
  static func write(_ key: String, _ value: [String: Any]?) {
    guard let defaults else { return }
    if let value, let data = try? JSONSerialization.data(withJSONObject: value), let text = String(data: data, encoding: .utf8) {
      defaults.set(text, forKey: key)
    } else {
      defaults.removeObject(forKey: key)
    }
  }
}

/// The USB port (`Engine.usbPort`) can only be bound by one process. The app
/// holds it in the foreground; a broadcast sharing over USB claims it so the
/// desktop's usbmux reconnect reaches the extension's engine, and hands it back
/// when the broadcast ends. Each side's listener retries the bind until the
/// other one has let go.
enum UsbHandoff {
  /// `BroadcastRequest.address` asking the extension to share over USB.
  static let address = "usb"
  /// A claim stays valid this long without a fresh broadcast state (startup).
  static let startupGrace: TimeInterval = 10

  static func claim(session: String) {
    BroadcastGroup.write(BroadcastGroup.usbOwnerKey, ["session": session, "claimed": Date().timeIntervalSince1970])
    BroadcastGroup.post(.usb)
  }
  static func release(session: String) {
    guard BroadcastGroup.read(BroadcastGroup.usbOwnerKey)?["session"] as? String == session else { return }
    BroadcastGroup.write(BroadcastGroup.usbOwnerKey, nil)
    BroadcastGroup.post(.usb)
  }
  /// Whether a live extension holds the port. A killed extension's claim
  /// expires with its broadcast state.
  static var extensionOwns: Bool {
    guard let owner = BroadcastGroup.read(BroadcastGroup.usbOwnerKey), let session = owner["session"] as? String else {
      return false
    }
    let now = Date().timeIntervalSince1970
    if let claimed = (owner["claimed"] as? NSNumber)?.doubleValue, now - claimed < startupGrace { return true }
    guard let state = BroadcastGroup.read(BroadcastGroup.stateKey), state["session"] as? String == session,
          ["connecting", "streaming"].contains(state["state"] as? String ?? ""),
          let updated = (state["updated"] as? NSNumber)?.doubleValue else { return false }
    return now - updated < BroadcastGroup.staleAfter
  }
}

struct SharingError: Error, CustomStringConvertible, LocalizedError {
  let description: String
  init(_ description: String) { self.description = description }
  var errorDescription: String? { description }
}

/// Per-peer preferences, validated exactly like Android's ScreenSharingOptions.
struct SharingPreferences: Equatable {
  var resolution: String
  var fps: Int
  var quality: String
  var codec: String

  static let defaults = SharingPreferences(resolution: "1280", fps: 60, quality: "balanced", codec: "h264")

  init(resolution: String, fps: Int, quality: String, codec: String) {
    self.resolution = resolution; self.fps = fps; self.quality = quality; self.codec = codec
  }
  init(json text: String) throws {
    guard text.utf8.count <= 1024, let data = text.data(using: .utf8),
          let value = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
      throw SharingError("Invalid screen sharing settings")
    }
    try self.init(value)
  }
  init(_ value: [String: Any]) throws {
    guard let resolution = value["resolution"] as? String, let fps = (value["fps"] as? NSNumber)?.intValue,
          let quality = value["quality"] as? String, let codec = value["codec"] as? String else {
      throw SharingError("Invalid screen sharing settings")
    }
    guard ["fit", "1280", "1920"].contains(resolution) else { throw SharingError("Choose a supported screen resolution") }
    guard [30, 60, 120].contains(fps) else { throw SharingError("Choose 30, 60 or 120 FPS") }
    guard ["responsive", "balanced", "detail"].contains(quality) else { throw SharingError("Choose a supported video quality") }
    guard ["h264", "hevc"].contains(codec) else { throw SharingError("Choose H.264 or HEVC") }
    self.init(resolution: resolution, fps: fps, quality: quality, codec: codec)
  }
  var dictionary: [String: Any] { ["resolution": resolution, "fps": fps, "quality": quality, "codec": codec] }
}

/// The engine's display `Config` (session::display::Config) for a mirrored
/// screen of the given upright size. Same scaling rules as Android's
/// ScreenSharingOptions.config, except ReplayKit never delivers more than 60 FPS.
struct SharingConfig: Equatable {
  var width: Int
  var height: Int
  var fps: Int
  var bitrate: Int
  var codec: String

  static let maxFrameRate = 60

  init(width: Int, height: Int, fps: Int, bitrate: Int, codec: String) {
    self.width = width; self.height = height; self.fps = fps; self.bitrate = bitrate; self.codec = codec
  }
  init(_ preferences: SharingPreferences, screenWidth: Int, screenHeight: Int) throws {
    guard screenWidth > 0, screenHeight > 0, screenWidth <= 16384, screenHeight <= 16384 else {
      throw SharingError("Invalid capture region")
    }
    let edge = preferences.resolution == "fit" ? 4096 : Int(preferences.resolution) ?? 1280
    var scale = min(1, Double(edge) / Double(max(screenWidth, screenHeight)))
    scale = max(scale, max(640 / Double(screenWidth), 480 / Double(screenHeight)))
    let width = Int(Double(screenWidth) * scale) / 2 * 2, height = Int(Double(screenHeight) * scale) / 2 * 2
    guard width <= 4096, height <= 4096 else { throw SharingError("This screen shape needs a different sharing resolution") }
    let bitrate = preferences.quality == "responsive" ? 8_000_000 : preferences.quality == "detail" ? 40_000_000 : 20_000_000
    self.init(width: width, height: height, fps: min(preferences.fps, Self.maxFrameRate), bitrate: bitrate, codec: preferences.codec)
  }
  /// Engine JSON (`mode` is always mirror; unsupported capture flags are off).
  var dictionary: [String: Any] {
    ["mode": "mirror", "width": width, "height": height, "fps": fps, "bitrate": bitrate,
     "retina": false, "auto_size": false, "codec": codec, "separate_cursor": false]
  }
  init(engine value: [String: Any]) throws {
    guard let width = (value["width"] as? NSNumber)?.intValue, let height = (value["height"] as? NSNumber)?.intValue,
          let fps = (value["fps"] as? NSNumber)?.intValue, let bitrate = (value["bitrate"] as? NSNumber)?.intValue,
          let codec = value["codec"] as? String, ["h264", "hevc"].contains(codec),
          width > 0, height > 0, width <= 4096, height <= 4096, fps > 0, bitrate > 0 else {
      throw SharingError("Invalid negotiated capture settings")
    }
    self.init(width: width, height: height, fps: fps, bitrate: bitrate, codec: codec)
  }
}

/// What the app asks the extension to do; written to `requestKey`.
struct BroadcastRequest {
  var session: String
  var peer: String
  /// LAN `host:port` of the receiver, or `UsbHandoff.address` to share over the
  /// extension's own USB carrier.
  var address: String
  var preferences: SharingPreferences
  var created: Date

  var dictionary: [String: Any] {
    ["session": session, "peer": peer, "address": address, "preferences": preferences.dictionary,
     "created": created.timeIntervalSince1970]
  }
  init(session: String = UUID().uuidString, peer: String, address: String, preferences: SharingPreferences, created: Date = Date()) {
    self.session = session; self.peer = peer; self.address = address; self.preferences = preferences; self.created = created
  }
  init(_ value: [String: Any]) throws {
    guard let session = value["session"] as? String, let peer = value["peer"] as? String,
          let address = value["address"] as? String, let preferences = value["preferences"] as? [String: Any],
          let created = (value["created"] as? NSNumber)?.doubleValue else {
      throw SharingError("Start sharing from extend.computer")
    }
    try self.init(session: session, peer: peer, address: address, preferences: SharingPreferences(preferences),
                  created: Date(timeIntervalSince1970: created))
  }
}
