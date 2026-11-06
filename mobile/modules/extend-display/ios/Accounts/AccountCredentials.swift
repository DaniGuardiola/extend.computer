import Foundation
import Security

/// Saved account login. Same JSON fields as Android's credential record; it
/// never crosses into React Native (the bridge only sees `AccountsController.state()`).
public struct AccountCredential: Codable, Equatable {
  var token: String
  var email: String
  var keyVerified: Bool
  var accountId: String?
  var deviceId: String?
  var deviceToken: String?
  var registeredName: String?
  var registeredFingerprint: String?

  enum CodingKeys: String, CodingKey {
    case token, email
    case keyVerified = "key_verified"
    case accountId = "account_id"
    case deviceId = "device_id"
    case deviceToken = "device_token"
    case registeredName = "registered_name"
    case registeredFingerprint = "registered_fingerprint"
  }
}

public protocol AccountCredentialStore: AnyObject {
  /// nil when nothing is saved for `origin`; throws when a saved record can't be read.
  func load(origin: String) throws -> AccountCredential?
  func save(origin: String, _ credential: AccountCredential) throws
  func clear(origin: String) throws
}

/// Keychain credentials, one generic-password item per server origin.
///
/// Android encrypts a no-backup file with a non-exportable Keystore AES-GCM key
/// and binds it to the origin as AAD. Here the Keychain provides the encryption
/// and the device binding: items are `AfterFirstUnlockThisDeviceOnly` and never
/// synchronizable, so neither iCloud Keychain nor a backup restore carries a
/// login to another device. The item name is SHA-256(origin) and the record
/// repeats the origin, which is checked on load (the AAD equivalent). A damaged
/// record is reported, never silently replaced.
public final class KeychainAccountCredentials: AccountCredentialStore {
  private struct Record: Codable { let v: Int; let origin: String; let credential: AccountCredential }
  let service: String
  private let accessGroup: String?
  private let lock = NSLock()

  public init(service: String = "computer.extend.accounts.v1", accessGroup: String? = nil) {
    self.service = service
    self.accessGroup = accessGroup
  }

  private func query(_ origin: String) -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: AccountProtocol.sha256Hex(origin) + ".v1",
      kSecAttrSynchronizable as String: kCFBooleanFalse!,
    ]
    if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
    return query
  }

  public func load(origin: String) throws -> AccountCredential? {
    lock.lock(); defer { lock.unlock() }
    var search = query(origin)
    search[kSecReturnData as String] = true
    search[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    let status = SecItemCopyMatching(search as CFDictionary, &item)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess else { throw AccountError("Saved login is unavailable. Unlock the device and try again.") }
    guard let data = item as? Data, data.count <= 16384,
          let record = try? JSONDecoder().decode(Record.self, from: data), record.v == 1, record.origin == origin
    else { throw AccountError("Saved login is damaged. Sign out and try again.") }
    return record.credential
  }

  public func save(origin: String, _ credential: AccountCredential) throws {
    lock.lock(); defer { lock.unlock() }
    let data = try JSONEncoder().encode(Record(v: 1, origin: origin, credential: credential))
    guard data.count <= 16000 else { throw AccountError("Account credential is too large.") }
    let attributes: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]
    var status = SecItemUpdate(query(origin) as CFDictionary, attributes as CFDictionary)
    if status == errSecItemNotFound {
      status = SecItemAdd(query(origin).merging(attributes) { $1 } as CFDictionary, nil)
    }
    guard status == errSecSuccess else { throw AccountError("Couldn't save the login. Try again.") }
  }

  public func clear(origin: String) throws {
    lock.lock(); defer { lock.unlock() }
    let status = SecItemDelete(query(origin) as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw AccountError("Couldn't remove saved login.")
    }
  }
}
