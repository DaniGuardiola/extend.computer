import Foundation
import Security

// Device identity: a 32-byte X25519 seed in the Keychain (Android: a file
// wrapped by a non-exportable Keystore key). The item is this-device-only, so
// neither iCloud Keychain nor a backup restore clones the identity onto another
// device, and readable after first unlock so the ReplayKit broadcast extension
// can start the same engine identity. With the `keychain-access-groups`
// entitlement (with-broadcast-extension plugin) new items land in the first
// listed group, which the app and its extension share.
enum DeviceIdentity {
  private static let service = "computer.extend.identity"
  private static let account = "seed.v1"
  private static let lock = NSLock()

  private static func base() -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
      kSecAttrSynchronizable as String: false,
    ]
    if let group = Bundle.main.object(forInfoDictionaryKey: "ExtendKeychainGroup") as? String, !group.isEmpty {
      query[kSecAttrAccessGroup as String] = group
    }
    return query
  }

  private static func read() throws -> Data? {
    var query = base()
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    switch status {
    case errSecSuccess:
      guard let data = result as? Data, data.count == 32 else {
        throw EngineError("Invalid device identity; refusing replacement.")
      }
      return data
    case errSecItemNotFound:
      return nil
    case errSecInteractionNotAllowed:
      throw EngineError("Unlock this iPhone to use extend.computer.")
    default:
      throw EngineError("Device identity unavailable (\(status)).")
    }
  }

  /// The seed if the app already created one. The broadcast extension uses
  /// this: it must never mint an identity of its own.
  static func existingSeed() throws -> Data? {
    lock.lock(); defer { lock.unlock() }
    return try read()
  }

  /// Returns the seed, creating it on first use. Never replaces an existing
  /// item that can't be read: that would silently orphan every pairing.
  static func seed() throws -> Data {
    lock.lock(); defer { lock.unlock() }
    if let seed = try read() { return seed }
    var bytes = Data(count: 32)
    let generated = bytes.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
    guard generated == errSecSuccess else { throw EngineError("Couldn't create a device identity.") }
    var item = base()
    item[kSecValueData as String] = bytes
    item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    item[kSecAttrLabel as String] = "extend.computer device identity"
    let status = SecItemAdd(item as CFDictionary, nil)
    if status == errSecDuplicateItem {
      bytes.resetBytes(in: 0..<bytes.count)
      guard let seed = try read() else { throw EngineError("Device identity unavailable.") }
      return seed
    }
    guard status == errSecSuccess else {
      bytes.resetBytes(in: 0..<bytes.count)
      throw EngineError("Couldn't save the device identity (\(status)).")
    }
    return bytes
  }
}

// Engine storage (trust store, peer names, app peer metadata). Lives in the App
// Group container when the app has one (shared with the broadcast extension),
// otherwise Application Support. Excluded from backup: trust belongs to the
// this-device-only identity above. Builds before the App Group existed kept the
// root in the app's own Application Support; the app moves it into the group on
// first launch so existing pairings survive.
enum EngineStorage {
  private static let lock = NSLock()
  private static var cached: URL?

  static var appGroup: String? {
    let group = Bundle.main.object(forInfoDictionaryKey: "ExtendAppGroup") as? String
    return group?.isEmpty == false ? group : nil
  }

  static func root() throws -> URL {
    lock.lock(); defer { lock.unlock() }
    if let cached { return cached }
    let manager = FileManager.default
    let local = try manager.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
      .appendingPathComponent("engine", isDirectory: true)
    var root = local
    if let group = appGroup, let container = manager.containerURL(forSecurityApplicationGroupIdentifier: group) {
      root = container.appendingPathComponent("Library/Application Support/engine", isDirectory: true)
      try migrate(from: local, to: root)
    }
    do {
      try manager.createDirectory(at: root, withIntermediateDirectories: true,
                                  attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
      var values = URLResourceValues()
      values.isExcludedFromBackup = true
      try root.setResourceValues(values)
    } catch {
      throw EngineError("Device identity storage unavailable.")
    }
    cached = root
    return root
  }

  /// One-time move of a pre-App-Group root. Only an absent or empty group root
  /// is replaced; if both hold data the group copy wins and the old one stays
  /// in place untouched. In the extension `legacy` (its own container) never exists.
  static func migrate(from legacy: URL, to root: URL) throws {
    let manager = FileManager.default
    var directory: ObjCBool = false
    guard manager.fileExists(atPath: legacy.path, isDirectory: &directory), directory.boolValue,
          let moving = try? manager.contentsOfDirectory(atPath: legacy.path), !moving.isEmpty else { return }
    if manager.fileExists(atPath: root.path) {
      guard (try? manager.contentsOfDirectory(atPath: root.path))?.isEmpty == true else {
        NSLog("extend.computer: engine storage exists in both locations; keeping the App Group copy")
        return
      }
      try? manager.removeItem(at: root)
    }
    do {
      try manager.createDirectory(at: root.deletingLastPathComponent(), withIntermediateDirectories: true)
      try manager.moveItem(at: legacy, to: root)
    } catch {
      throw EngineError("Couldn't move device storage (\(error.localizedDescription)).")
    }
  }
}
