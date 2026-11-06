import CryptoKit
import Foundation
import Security

// Pure account wire validation, ported from Android's AccountProtocol.java.
// Discovery and heartbeat are never peer authentication.

/// User-facing account failure. `message` is safe to show and never contains a credential.
public struct AccountError: LocalizedError, CustomStringConvertible {
  public let message: String
  public init(_ message: String) { self.message = message }
  public var errorDescription: String? { message }
  public var description: String { message }
}

enum AccountProtocol {
  private static let loopbackHosts: Set<String> = ["localhost", "127.0.0.1", "::1"]

  /// Canonical account server origin: lowercase scheme/host, default port elided,
  /// no path/query/fragment/credentials. HTTP only for exact loopback hosts.
  static func origin(_ raw: String) throws -> String {
    let invalid = AccountError("Use the account server origin without a path, query or credentials.")
    let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    guard let parts = URLComponents(string: trimmed), let rawScheme = parts.scheme,
          var host = parts.percentEncodedHost, !host.isEmpty,
          parts.percentEncodedUser == nil, parts.percentEncodedPassword == nil,
          parts.percentEncodedQuery == nil, parts.percentEncodedFragment == nil,
          parts.percentEncodedPath.isEmpty || parts.percentEncodedPath == "/"
    else { throw invalid }
    if let port = parts.port, port <= 0 || port > 65535 { throw invalid }
    let scheme = rawScheme.lowercased()
    host = host.lowercased()
    let bare = host.hasPrefix("[") && host.hasSuffix("]") ? String(host.dropFirst().dropLast()) : host
    let ipv6 = bare.contains(":")
    let allowed = ipv6 ? CharacterSet(charactersIn: "0123456789abcdef:.") : CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789.-")
    guard bare.unicodeScalars.allSatisfy({ allowed.contains($0) }) else { throw invalid }
    guard scheme == "https" || (scheme == "http" && loopbackHosts.contains(bare)) else {
      throw AccountError("Account servers require HTTPS. Localhost HTTP is allowed for development.")
    }
    var port = parts.port
    if port == (scheme == "https" ? 443 : 80) { port = nil }
    return "\(scheme)://\(ipv6 ? "[\(bare)]" : bare)\(port.map { ":\($0)" } ?? "")"
  }

  static func hex(_ value: String?) -> Bool {
    guard let value, value.utf8.count == 64 else { return false }
    return value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
  }

  static func text(_ value: [String: Any], _ key: String, _ max: Int) throws -> String {
    guard let text = value[key] as? String, !text.isEmpty, text.utf8.count <= max,
          !text.unicodeScalars.contains(where: { $0.value < 0x20 || (0x7f...0x9f).contains($0.value) })
    else { throw AccountError("Invalid account response.") }
    return text
  }

  static func id(_ value: [String: Any], _ key: String) throws -> String {
    let id = try text(value, key, 64)
    guard hex(id) else { throw AccountError("Invalid account device identifier.") }
    return id
  }

  static func randomBytes(_ count: Int) -> Data {
    var bytes = Data(count: count)
    let status = bytes.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, count, $0.baseAddress!) }
    precondition(status == errSecSuccess, "System random generator unavailable")
    return bytes
  }
  static func hexBytes<D: Sequence>(_ bytes: D) -> String where D.Element == UInt8 {
    bytes.map { String(format: "%02x", $0) }.joined()
  }
  static func randomHex() -> String { hexBytes(randomBytes(32)) }
  static func base64URL(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
  }
  static func verifier() -> String { base64URL(randomBytes(32)) }
  static func challenge(_ verifier: String) -> String { base64URL(Data(SHA256.hash(data: Data(verifier.utf8)))) }
  static func sha256Hex(_ text: String) -> String { hexBytes(SHA256.hash(data: Data(text.utf8))) }

  private static func constantTimeEqual(_ a: String, _ b: String) -> Bool {
    let x = Array(a.utf8), y = Array(b.utf8)
    guard x.count == y.count else { return false }
    return zip(x, y).reduce(UInt8(0)) { $0 | ($1.0 ^ $1.1) } == 0
  }

  /// Returns the code from `GET /callback?state=..&code=.. HTTP/1.1` when it
  /// carries exactly the expected state; nil for anything else.
  static func callback(_ requestLine: String, state: String) -> String? {
    guard requestLine.hasPrefix("GET "), requestLine.hasSuffix(" HTTP/1.1") else { return nil }
    let target = String(requestLine.dropFirst(4).dropLast(9))
    guard target.hasPrefix("/"), !target.hasPrefix("//"), !target.contains("#"),
          let mark = target.firstIndex(of: "?"), target[..<mark] == "/callback" else { return nil }
    var query: [String: String] = [:]
    for pair in target[target.index(after: mark)...].split(separator: "&", omittingEmptySubsequences: false) {
      let field = pair.split(separator: "=", omittingEmptySubsequences: false)
      guard field.count == 2,
            let key = field[0].replacingOccurrences(of: "+", with: " ").removingPercentEncoding,
            let value = field[1].replacingOccurrences(of: "+", with: " ").removingPercentEncoding,
            query.updateValue(value, forKey: key) == nil else { return nil }
    }
    guard query.count == 2, let received = query["state"], let code = query["code"],
          hex(received), hex(code), constantTimeEqual(state, received) else { return nil }
    return code
  }

  private static func isBool(_ value: Any?) -> Bool {
    guard let number = value as? NSNumber else { return false }
    return CFGetTypeID(number) == CFBooleanGetTypeID()
  }

  /// Validated account directory rows; only whitelisted fields survive.
  static func directory(_ response: [String: Any]) throws -> [[String: Any]] {
    guard let rows = response["devices"] as? [[String: Any]] else { throw AccountError("Invalid account response.") }
    guard rows.count <= 100 else { throw AccountError("Account device list is too large.") }
    var ids = Set<String>(), fingerprints = Set<String>(), clean: [[String: Any]] = []
    for row in rows {
      let rowID = try id(row, "id"), fingerprint = try id(row, "fingerprint")
      guard ids.insert(rowID).inserted, fingerprints.insert(fingerprint).inserted else {
        throw AccountError("Duplicate account device.")
      }
      let verified = row["key_verified"]
      let validKey = (isBool(verified) && (verified as? Bool) == true)
        || (!isBool(verified) && (verified as? NSNumber).map { $0 == 1 } == true)
      let online = isBool(row["online"]) && (row["online"] as? Bool) == true
      clean.append(["id": rowID, "fingerprint": fingerprint, "name": try text(row, "name", 100),
                    "platform": try text(row, "platform", 32), "key_verified": validKey, "online": online])
    }
    return clean
  }

  static func encode(_ value: Any) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]),
          let text = String(data: data, encoding: .utf8) else { return "{}" }
    return text
  }
}
