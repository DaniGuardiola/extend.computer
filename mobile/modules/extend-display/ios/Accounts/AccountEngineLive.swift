#if EXTEND_ENGINE
import ExtendEngine
import Foundation

extension AccountEngine {
  /// Engine wiring over the `ext_*` account calls in ExtendEngine.h. `identity`
  /// and `name` come from the module's engine owner (it starts the engine and
  /// returns `{"fingerprint", "public_key"}`), e.g.
  /// `.live(identity: { try Engine.identity() }, name: { Engine.localName() })`.
  public static func live(identity: @escaping () throws -> [String: Any], name: @escaping () -> String) -> AccountEngine {
    AccountEngine(
      publicIdentity: identity,
      name: name,
      proof: { key, challenge in
        let value = try key.withCString { k in try challenge.withCString { c in try unwrap(ext_account_proof(k, c)) } }
        guard let proof = value as? String else { throw AccountError("Native engine result unavailable") }
        return proof
      },
      replacePeers: { scope, directory in
        _ = try scope.withCString { s in try directory.withCString { d in try unwrap(ext_replace_account_peers(s, d)) } }
      },
      clearPeers: { _ = try unwrap(ext_clear_account_peers()) },
      configureRelay: { server, scope, device, token, fingerprint in
        _ = try server.withCString { a in try scope.withCString { b in try device.withCString { c in
          try token.withCString { d in try fingerprint.withCString { e in try unwrap(ext_configure_account_relay(a, b, c, d, e)) } }
        } } }
      })
  }

  /// `{"ok":<value>}` → value; `{"error":".."}` → throws. Frees the native string.
  private static func unwrap(_ raw: UnsafeMutablePointer<CChar>?) throws -> Any {
    guard let raw else { throw AccountError("Native engine result unavailable") }
    defer { ext_string_free(raw) }
    guard let parsed = (try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8), options: [.fragmentsAllowed])) as? [String: Any]
    else { throw AccountError("Native engine result unavailable") }
    if let message = parsed["error"] as? String { throw AccountError(message) }
    return parsed["ok"] ?? NSNull()
  }
}
#endif
