import Foundation

// App-side metadata that Android keeps in SharedPreferences ("engine-peers",
// "engine-labels", "engine-local"). It sits beside the engine's own state in
// the backup-excluded engine root, because it only means something for this
// device identity. Writes are atomic; a failed write throws like Android's
// `commit() == false`.
enum LocalStore {
  private static let lock = NSRecursiveLock()
  private static var loaded: JSON?

  private static func url() throws -> URL { try EngineStorage.root().appendingPathComponent("app-state.json") }

  private static func state() -> JSON {
    if let loaded { return loaded }
    let value = (try? Data(contentsOf: url())).flatMap { try? JSONSerialization.jsonObject(with: $0) as? JSON } ?? [:]
    loaded = value
    return value
  }
  private static func write(_ change: (inout JSON) -> Void) throws {
    lock.lock(); defer { lock.unlock() }
    var value = state()
    change(&value)
    do {
      let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
      try data.write(to: url(), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    } catch {
      throw EngineError("Couldn't save device settings")
    }
    loaded = value
  }
  private static func read<T>(_ key: String) -> T? {
    lock.lock(); defer { lock.unlock() }
    return state()[key] as? T
  }

  // MARK: Peer metadata: {"address": "host:port", "name": "..."} per fingerprint.

  static func peer(_ peer: String) -> JSON? { (read("peers") as [String: JSON]?)?[peer] }
  static func setPeer(_ peer: String, _ metadata: JSON?) throws {
    try write { state in
      var peers = state["peers"] as? [String: JSON] ?? [:]
      peers[peer] = metadata
      state["peers"] = peers
    }
  }

  // MARK: Labels: local presentation names, never identity.

  static func label(_ peer: String) -> String? { (read("labels") as [String: String]?)?[peer] }
  static func setLabel(_ peer: String, _ label: String?) throws {
    try write { state in
      var labels = state["labels"] as? [String: String] ?? [:]
      labels[peer] = label
      state["labels"] = labels
    }
  }

  // MARK: Local device

  static var customName: String? { read("name") }
  static func setCustomName(_ name: String?) throws { try write { state in if let name { state["name"] = name } else { state.removeValue(forKey: "name") } } }
  static var receiving: Bool { read("receiving") ?? false }
  static func setReceiving(_ enabled: Bool) throws { try write { $0["receiving"] = enabled } }
}
