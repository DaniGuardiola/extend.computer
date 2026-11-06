import ExtendEngine
import Foundation

/// `EngineSender` over the Rust engine linked into the extension
/// (ExtendEngine.xcframework). The extension starts the app's engine identity
/// (Keychain seed shared through the keychain access group) on the App Group
/// engine root, so the peer sees the same paired device as the app.
final class NativeEngineSender: EngineSender {
  private let lock = NSLock()
  /// Session id from the last start; empty before.
  private var id = ""

  init() throws {
    guard var seed = try DeviceIdentity.existingSeed() else {
      throw SharingError("Open extend.computer and pair a device before sharing your screen")
    }
    defer { seed.resetBytes(in: 0..<seed.count) }
    guard EngineStorage.appGroup != nil else { throw SharingError("Screen sharing isn't set up on this iPhone") }
    let root = try EngineStorage.root().path
    _ = try seed.withUnsafeBytes { bytes in
      // Only reads the stored local name; never renames the device.
      try Engine.call(root, Self.deviceKind) { ext_initialize(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, $0, $1) }
    }
  }

  /// UIDevice is main-actor isolated; the model identifier answers the same question.
  private static var deviceKind: String {
    var system = utsname()
    uname(&system)
    let machine = withUnsafeBytes(of: system.machine) { String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self) }
    let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] ?? ""
    return machine.hasPrefix("iPad") || simulated.hasPrefix("iPad") ? "tablet" : "phone"
  }

  private var session: String { lock.lock(); defer { lock.unlock() }; return id }

  func start(peer: String, address: String, config: SharingConfig) throws {
    let result = try Engine.call(peer, address, JSONText.encode(config.dictionary)) { ext_start_display_sender($0, $1, $2) }
    guard let started = (result as? JSON)?["id"] as? String, !started.isEmpty else {
      throw SharingError("Screen connection unavailable")
    }
    lock.lock(); id = started; lock.unlock()
  }

  func configure(_ config: SharingConfig) throws {
    try Engine.call(session, JSONText.encode(config.dictionary)) { ext_configure_display_sender($0, $1) }
  }

  func push(frame: Data, timestamp: UInt64, keyframe: Bool) throws {
    guard timestamp > 0, timestamp <= UInt64(Int64.max) else { throw SharingError("Invalid capture timestamp") }
    let session = self.session
    _ = try session.withCString { id in
      try frame.withUnsafeBytes { bytes in
        try Engine.call {
          ext_push_display_frame(id, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, Int64(timestamp), keyframe)
        }
      }
    }
  }

  func state() -> EngineSenderState {
    let value: JSON
    do { value = try Engine.call { ext_display_sender_state() } as? JSON ?? ["state": "idle"] } catch {
      value = ["state": "failed", "error": error.localizedDescription]
    }
    let session = self.session
    if !session.isEmpty, let current = value["id"] as? String, current != session {
      // Another capture replaced ours inside this process.
      return EngineSenderState(["id": session, "state": "closed", "error": "Screen sharing was replaced"])
    }
    return EngineSenderState(value)
  }

  func stop() {
    let session = self.session
    guard !session.isEmpty else { return }
    _ = try? Engine.call(session) { ext_stop_display_sender($0) }
  }
}

/// SampleHandler reports errors with "\(error)"; show the engine's own message.
extension EngineError: CustomStringConvertible {
  var description: String { message }
}
