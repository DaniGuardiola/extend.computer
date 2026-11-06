import ExtendEngine
import Foundation

// Swift facade over the Rust engine's C ABI (native/ios/engine, ExtendEngine.h),
// the iOS counterpart of Android's NativeEngine.java. Every `ext_*` string
// result is `{"ok":<value>}` or `{"error":"<message>"}`; `call` unwraps it and
// releases the native string. No Expo dependency, so the broadcast extension
// can compile this file too.

struct EngineError: LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}

typealias JSON = [String: Any]

enum JSONText {
  static func encode(_ value: Any) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]),
          let text = String(data: data, encoding: .utf8) else { return "{}" }
    return text
  }
  static func object(_ text: String?) -> JSON? {
    guard let data = text?.data(using: .utf8) else { return nil }
    return (try? JSONSerialization.jsonObject(with: data)) as? JSON
  }
}

enum Engine {
  /// Android's `synchronized (NativeEngine.class)`: check, address saves and
  /// unpair never interleave.
  static let lock = NSRecursiveLock()
  /// Engine work that may block on the network runs here, off the JS thread and
  /// off Expo's serial async queue.
  static let queue = DispatchQueue(label: "computer.extend.engine", qos: .userInitiated, attributes: .concurrent)
  /// "phone" or "tablet"; set by the app before the first `identity()` call.
  static var deviceKind = "phone"
  /// Display name reported to peers; set by the app.
  static var localName: () -> String = { "iPhone" }

  private static var cachedIdentity: JSON?

  // MARK: Plumbing

  @discardableResult
  static func call(_ body: () -> UnsafeMutablePointer<CChar>?) throws -> Any {
    guard let raw = body() else { throw EngineError("Native engine result unavailable") }
    defer { ext_string_free(raw) }
    guard let data = String(cString: raw).data(using: .utf8),
          let parsed = (try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])) as? JSON else {
      throw EngineError("Native engine result unavailable")
    }
    if let error = parsed["error"] as? String { throw EngineError(error) }
    return parsed["ok"] ?? NSNull()
  }
  @discardableResult
  static func call(_ a: String, _ body: (UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
    try a.withCString { pa in try call { body(pa) } }
  }
  @discardableResult
  static func call(_ a: String, _ b: String, _ body: (UnsafePointer<CChar>, UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
    try a.withCString { pa in try b.withCString { pb in try call { body(pa, pb) } } }
  }
  @discardableResult
  static func call(_ a: String, _ b: String, _ c: String,
                   _ body: (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
    try a.withCString { pa in try b.withCString { pb in try c.withCString { pc in try call { body(pa, pb, pc) } } } }
  }
  @discardableResult
  static func call(_ a: String, _ b: String, _ c: String, _ d: String,
                   _ body: (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
    try a.withCString { pa in try b.withCString { pb in try c.withCString { pc in try d.withCString { pd in try call { body(pa, pb, pc, pd) } } } } }
  }
  private static func object(_ value: Any) throws -> JSON {
    guard let object = value as? JSON else { throw EngineError("Native engine result unavailable") }
    return object
  }

  // MARK: Identity

  /// Loads (or creates) the Keychain seed and starts the engine once.
  @discardableResult
  static func identity() throws -> JSON {
    lock.lock(); defer { lock.unlock() }
    if let cachedIdentity { return cachedIdentity }
    var seed = try DeviceIdentity.seed()
    defer { seed.resetBytes(in: 0..<seed.count) }
    let root = try EngineStorage.root().path
    let result = try seed.withUnsafeBytes { bytes in
      try call(root, deviceKind) { ext_initialize(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, $0, $1) }
    }
    let identity = try object(result)
    _ = try? updateLocalDeviceName(localName())
    cachedIdentity = identity
    return identity
  }
  static func fingerprint() throws -> String {
    guard let value = try identity()["fingerprint"] as? String else { throw EngineError("Device identity unavailable") }
    return value
  }

  // MARK: Peers and presence

  static func pairedPeers() throws -> [String] { try call { ext_paired_peers() } as? [String] ?? [] }
  static func isPaired(_ peer: String) throws -> Bool { try pairedPeers().contains(peer) }
  static func peerNames() throws -> [String: String] { try call { ext_peer_names() } as? [String: String] ?? [:] }
  static func peerHosts() throws -> [String: String] { try call { ext_peer_hosts() } as? [String: String] ?? [:] }
  static func accountPeerNames() throws -> [String: String] { try call { ext_account_peer_names() } as? [String: String] ?? [:] }
  static func updateLocalDeviceName(_ name: String) throws { try call(name) { ext_update_local_device_name($0) } }
  static func resolvePresence(_ address: String, _ peer: String) throws -> JSON {
    try object(call(address, peer) { ext_resolve_presence($0, $1) })
  }
  static func probePeer(_ address: String, _ peer: String) throws -> JSON {
    try object(call(address, peer) { ext_probe_peer($0, $1) })
  }
  static func resolveAccountPresence(_ peer: String, refresh: Bool) throws -> JSON {
    try object(call(peer) { ext_resolve_account_presence($0, refresh) })
  }
  static func probeAccountPeer(_ peer: String) throws -> JSON { try object(call(peer) { ext_probe_account_peer($0) }) }
  static func forgetPeer(_ peer: String) throws -> JSON { try object(call(peer) { ext_forget_peer($0) }) }
  static func notifyUnpair(_ peer: String, _ address: String, _ operation: String) throws {
    try call(peer, address, operation) { ext_notify_unpair($0, $1, $2) }
  }
  static func removedPeers() throws -> [String: String] { try call { ext_removed_peers() } as? [String: String] ?? [:] }
  static func dismissRemovedPeer(_ peer: String) throws -> Any { try call(peer) { ext_dismiss_removed_peer($0) } }

  // MARK: USB

  static func usbState() throws -> JSON { try object(call { ext_usb_state() }) }
  static func resolveUsbPresence(_ peer: String, _ id: String) throws -> JSON {
    try object(call(peer, id) { ext_resolve_usb_presence($0, $1) })
  }
  static func probeUsbPeer(_ peer: String, _ id: String) throws -> JSON {
    try object(call(peer, id) { ext_probe_usb_peer($0, $1) })
  }
  /// Loopback listener the desktop reaches through usbmuxd (`IOS_USB_PORT`).
  static let usbPort: UInt16 = 47_177
  static func startUsbListener() throws { try call { ext_start_usb_listener(usbPort) } }
  static func stopUsbListener() { _ = try? call { ext_stop_usb_listener() } }
  static func usbListenerState() throws -> JSON { try object(call { ext_usb_listener_state() }) }

  // MARK: Pairing

  /// Returns the paired peer fingerprint.
  static func pairCode(_ address: String, _ code: String) throws -> String {
    guard let peer = try call(address, code, { ext_pair_code($0, $1) }) as? String else {
      throw EngineError("Pairing failed")
    }
    return peer
  }
  static func startManualPairing(_ id: String, _ address: String, _ code: String, _ name: String) throws -> JSON {
    try object(call(id, address, code, name) { ext_start_manual_pairing($0, $1, $2, $3) })
  }
  static func manualPairingState(_ id: String) throws -> JSON { try object(call(id) { ext_manual_pairing_state($0) }) }
  static func stopManualPairing(_ id: String) { _ = try? call(id) { ext_stop_manual_pairing($0) } }
  static func startVisualPairing(_ address: String, _ name: String, _ localName: String) throws -> JSON {
    try object(call(address, name, localName) { ext_start_visual_pairing($0, $1, $2) })
  }
  static func visualPairingState(_ id: String) throws -> JSON { try object(call(id) { ext_visual_pairing_state($0) }) }
  static func respondVisualPairing(_ id: String, _ request: String, _ allow: Bool) throws {
    try call(id, request) { ext_respond_visual_pairing($0, $1, allow) }
  }
  static func stopVisualPairing(_ id: String) { _ = try? call(id) { ext_stop_visual_pairing($0) } }
  static func startIncoming(_ name: String) throws -> JSON { try object(call(name) { ext_start_incoming($0) }) }
  static func incomingState() throws -> JSON { try object(call { ext_incoming_state() }) }
  static func respondIncoming(_ request: String, _ allow: Bool) throws {
    try call(request) { ext_respond_incoming($0, allow) }
  }
  static func stopIncoming() { _ = try? call { ext_stop_incoming() } }

  // MARK: Receiving

  static func startReceiving(_ name: String) throws -> JSON { try object(call(name) { ext_start_receiving($0) }) }
  static func receivingState() throws -> JSON { try object(call { ext_receiving_state() }) }
  static func stopReceiving() { _ = try? call { ext_stop_receiving() } }
  static func startPresence(_ name: String) throws -> JSON { try object(call(name) { ext_start_presence($0) }) }
  static func stopPresence() { _ = try? call { ext_stop_presence() } }
  static func startDisplayReceiver(_ capabilities: String) throws { try call(capabilities) { ext_start_display_receiver($0) } }
  static func stopDisplayReceiver() { _ = try? call { ext_stop_display_receiver() } }

  // MARK: Sessions

  static func displayState() -> JSON { ((try? call { ext_display_state() }) as? JSON) ?? ["state": "idle"] }
  static func controlState() -> JSON { ((try? call { ext_control_state() }) as? JSON) ?? ["state": "idle"] }
  static func displayReceiverSettings(_ id: String) throws -> Any { try call(id) { ext_display_receiver_settings($0) } }
  static func requestDisplayReceiverSettings(_ id: String, _ revision: Int64, _ settings: String) throws -> Any {
    try call(id, settings) { ext_request_display_receiver_settings($0, revision, $1) }
  }
  static func cancelDisplay(_ id: String) { _ = try? call(id) { ext_cancel_display($0) } }
  static func startControl(_ peer: String, _ address: String) throws -> Any {
    try call(peer, address) { ext_start_control($0, $1) }
  }
  static func controlEvent(_ id: String, _ event: String) throws { try call(id, event) { ext_control_event($0, $1) } }
  static func stopControl(_ id: String) { _ = try? call(id) { ext_stop_control($0) } }
}
