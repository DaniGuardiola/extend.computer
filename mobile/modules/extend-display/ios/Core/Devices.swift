import Foundation

// Paired devices, route hints and checks. Ported from Android's NativeEngine
// (devices/check/forget), DeviceSettings, PeerAddress and DeviceAvailability.

private func isFingerprint(_ value: String) -> Bool {
  value.utf8.count == 64 && value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
}

/// Numeric route hints only. Saving an address never authenticates or pairs.
enum PeerAddress {
  private static func invalid() -> EngineError { EngineError("Enter a numeric IPv4:port or [IPv6]:port address.") }

  static func normalize(_ raw: String) throws -> String {
    if raw.utf8.count > 128 || raw.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) { throw invalid() }
    let address = raw.trimmingCharacters(in: .whitespaces)
    var host: String, port: Substring
    if address.hasPrefix("[") {
      guard let end = address.firstIndex(of: "]"), address.distance(from: address.startIndex, to: end) >= 2 else { throw invalid() }
      let after = address.index(after: end)
      guard after < address.endIndex, address[after] == ":" else { throw invalid() }
      host = String(address[address.index(after: address.startIndex)..<end])
      port = address[address.index(after: after)...]
      try ipv6(host)
      host = "[\(host.lowercased())]"
    } else {
      let parts = address.split(separator: ":", omittingEmptySubsequences: false)
      guard parts.count == 2, !parts[0].isEmpty else { throw invalid() }
      host = String(parts[0]); port = parts[1]
      let bytes = try ipv4(host)
      if bytes.allSatisfy({ $0 == 0 }) || (224...239).contains(bytes[0]) { throw invalid() }
    }
    guard (1...5).contains(port.count), port.allSatisfy(\.isASCII), port.allSatisfy(\.isNumber),
          let number = Int(port), (1...65535).contains(number) else { throw invalid() }
    return "\(host):\(number)"
  }
  private static func ipv4(_ host: String) throws -> [Int] {
    let parts = host.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 4 else { throw invalid() }
    return try parts.map { part in
      guard (1...3).contains(part.count), part.allSatisfy({ $0.isASCII && $0.isNumber }),
            part == "0" || !part.hasPrefix("0"), let value = Int(part), value <= 255 else { throw invalid() }
      return value
    }
  }
  private static func ipv6(_ host: String) throws {
    let pieces = host.split(separator: "%", maxSplits: 1, omittingEmptySubsequences: false)
    let literal = String(pieces[0])
    if pieces.count == 2 {
      guard (1...10).contains(pieces[1].count), pieces[1].allSatisfy({ $0.isASCII && $0.isNumber }),
            let scope = UInt64(pieces[1]), scope <= 0xffff_ffff else { throw invalid() }
    }
    var parsed = in6_addr()
    guard literal.contains(":"), inet_pton(AF_INET6, literal, &parsed) == 1 else { throw invalid() }
    let bytes = withUnsafeBytes(of: parsed) { Array($0) }
    if bytes.allSatisfy({ $0 == 0 }) || bytes[0] == 0xff { throw invalid() }
  }

  static func save(_ peer: String, _ raw: String) throws {
    guard isFingerprint(peer) else { throw EngineError("Invalid device identity.") }
    let address = try normalize(raw)
    // Same monitor as check and unpair: a stale save can't recreate metadata
    // after a completed local unpair.
    Engine.lock.lock(); defer { Engine.lock.unlock() }
    guard try Engine.isPaired(peer) else { throw EngineError("This device is no longer paired.") }
    var metadata = LocalStore.peer(peer) ?? [:]
    metadata["address"] = address
    try LocalStore.setPeer(peer, metadata)
  }
  static func verifyAndSave(_ peer: String, _ address: String) throws -> JSON {
    Engine.lock.lock(); defer { Engine.lock.unlock() }
    let endpoint = try normalize(address)
    // Neither a discovery hint nor a presence reply alone replaces the route.
    let result = try Engine.probePeer(endpoint, peer)
    try save(peer, endpoint)
    return result
  }
}

enum Devices {
  static func normalizeName(_ raw: String) throws -> String {
    let name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    if name.utf8.count > 100 || name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) {
      throw EngineError("Use a name up to 100 bytes without control characters.")
    }
    return name
  }

  static func list() throws -> [JSON] {
    try Engine.identity()
    let names = try Engine.peerNames()
    let account = (try? Engine.accountPeerNames()) ?? [:]
    return try Engine.pairedPeers().map { peer in
      var item = LocalStore.peer(peer) ?? [:]
      if item["name"] == nil { item["name"] = account[peer] ?? names[peer] ?? "Device \(peer.prefix(8))" }
      if item["address"] == nil { item["address"] = "" }
      item["peer"] = peer
      item["trustSource"] = account[peer] != nil ? "account" : "manual"
      let authenticated = names[peer] ?? account[peer] ?? ""
      let label = LocalStore.label(peer)
      let defaultName = item["name"] as? String ?? ""
      item["defaultName"] = defaultName
      item["authenticatedName"] = authenticated.isEmpty ? NSNull() : authenticated
      item["label"] = label ?? NSNull()
      item["name"] = label ?? defaultName
      return item
    }
  }

  static func info(_ peer: String) throws -> JSON {
    guard isFingerprint(peer) else { throw EngineError("Invalid device identity.") }
    guard let item = try list().first(where: { $0["peer"] as? String == peer }) else {
      throw EngineError("This device is no longer paired.")
    }
    return item
  }

  static func setLabel(_ peer: String, _ raw: String) throws -> JSON {
    _ = try info(peer) // Never create labels for arbitrary fingerprints.
    let name = try normalizeName(raw)
    try LocalStore.setLabel(peer, name.isEmpty ? nil : name)
    return try info(peer)
  }

  static func setAddress(_ peer: String, _ address: String) throws -> JSON {
    try Engine.identity()
    try PeerAddress.save(peer, address)
    return try info(peer)
  }

  static func pair(address: String, code: String, name: String) throws -> JSON {
    try Engine.identity()
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty || trimmed.count > 128 { throw EngineError("Enter a device name") }
    let address = address.trimmingCharacters(in: .whitespaces)
    let peer = try Engine.pairCode(address, code)
    do { try LocalStore.setPeer(peer, ["address": address, "name": trimmed]) } catch {
      throw EngineError("Paired device, but its address could not be saved")
    }
    return ["peer": peer, "name": trimmed, "address": address]
  }

  static func forget(_ peer: String) throws {
    Engine.lock.lock(); defer { Engine.lock.unlock() }
    try Engine.identity()
    let saved = LocalStore.peer(peer)
    guard let operation = try Engine.forgetPeer(peer)["operation"] as? String else {
      throw EngineError("Couldn't unpair this device")
    }
    // The hint uses only the saved literal endpoint; local revocation is done.
    if let address = saved?["address"] as? String, !address.isEmpty {
      try? Engine.notifyUnpair(peer, address, operation)
    }
    do { try LocalStore.setPeer(peer, nil) } catch {
      throw EngineError("Device unpaired, but its saved address could not be removed")
    }
    try LocalStore.setLabel(peer, nil)
  }

  static func usbSelected(_ snapshot: JSON, _ peer: String) -> Bool {
    guard snapshot["state"] as? String == "connected", snapshot["peer"] as? String == peer,
          let id = snapshot["id"] as? String else { return false }
    return (1...20).contains(id.count) && id.allSatisfy { $0.isASCII && $0.isNumber }
  }

  /// Saved route host and the engine's last authenticated host, both at the
  /// fixed presence port. Hints only: a pinned reply establishes status.
  static func endpoints(_ address: String, _ engineHost: String) -> [String] {
    var result: [String] = []
    func add(_ raw: String) {
      var host = raw.trimmingCharacters(in: .whitespaces)
      if host.hasPrefix("[") && host.hasSuffix("]") { host = String(host.dropFirst().dropLast()) }
      if host.isEmpty || host.contains("[") || host.contains("]") { return }
      host = host.lowercased()
      let endpoint = (host.contains(":") ? "[\(host)]" : host) + ":48178"
      if !result.contains(endpoint) { result.append(endpoint) }
    }
    let trimmed = address.trimmingCharacters(in: .whitespaces)
    if let colon = trimmed.lastIndex(of: ":"), colon > trimmed.startIndex {
      let port = trimmed[trimmed.index(after: colon)...]
      let host = String(trimmed[..<colon])
      let validPort = (1...5).contains(port.count) && port.allSatisfy { $0.isASCII && $0.isNumber } && (1...65535).contains(Int(port) ?? 0)
      let bracketed = host.hasPrefix("[") && host.hasSuffix("]")
      if validPort && (bracketed || !host.contains(":")) { add(host) }
    }
    add(engineHost)
    return result
  }

  static func offline(_ error: Error) -> Bool {
    error.localizedDescription.contains("Could not reach this device's availability service")
  }

  static func query(_ endpoints: [String], _ peer: String) throws -> JSON {
    var failed: Error?
    for endpoint in endpoints {
      do { return try Engine.resolvePresence(endpoint, peer) } catch {
        // A verification failure must not become Offline because a later
        // stale hint refused the connection.
        if failed == nil || offline(failed!) { failed = error }
      }
    }
    throw failed ?? EngineError("Device address unavailable")
  }

  /// Pinned probe over the best route (USB, LAN, nearby presence, internet).
  static func check(_ peer: String) throws -> JSON {
    Engine.lock.lock(); defer { Engine.lock.unlock() }
    try Engine.identity()
    // The cable selects an epoch, not an authenticated connection.
    let usb = try Engine.usbState()
    if usbSelected(usb, peer), let id = usb["id"] as? String {
      let remote = try Engine.resolveUsbPresence(peer, id)
      if remote["receiving"] as? Bool != true { throw EngineError("This device isn't accepting connections") }
      let verified = try Engine.probeUsbPeer(peer, remote["carrier_id"] as? String ?? "")
      _ = try info(peer)
      return verified
    }
    _ = try info(peer)
    var address = LocalStore.peer(peer)?["address"] as? String ?? ""
    let host = try Engine.peerHosts()[peer] ?? ""
    var presence: JSON?
    var presenceError: Error?
    do { presence = try query(endpoints(address, host), peer) } catch { presenceError = error }
    if presence == nil {
      // Bounded browse; each candidate still has to prove the paired identity.
      let discovery = NearbyDiscovery(presenceOnly: true)
      discovery.start()
      Thread.sleep(forTimeInterval: 2)
      let candidates = discovery.snapshot()["candidates"] as? [JSON] ?? []
      discovery.stop()
      var hosts: [String] = []
      for pass in 0..<2 {
        for candidate in candidates {
          let hint = (candidate["id"] as? String ?? "").contains(String(peer.prefix(12)))
          if hint != (pass == 0) { continue }
          for endpoint in candidate["addresses"] as? [String] ?? [] where hosts.count < 8 && !hosts.contains(endpoint) {
            hosts.append(endpoint)
          }
        }
      }
      for endpoint in hosts {
        do { presence = try Engine.resolvePresence(endpoint, peer); break } catch { presenceError = error }
      }
    }
    if let presence {
      if presence["receiving"] as? Bool != true { throw EngineError("This device isn't accepting connections") }
      address = presence["address"] as? String ?? address
      return try PeerAddress.verifyAndSave(peer, address)
    }
    // Internet membership is a route hint; presence and probe still pin.
    do {
      let remote = try Engine.resolveAccountPresence(peer, refresh: true)
      if remote["receiving"] as? Bool != true { throw EngineError("This device isn't accepting connections") }
      let verified = try Engine.probeAccountPeer(peer)
      _ = try info(peer)
      return verified
    } catch let error as EngineError where error.message == "This device isn't accepting connections" {
      throw error
    } catch {}
    if !address.isEmpty {
      let result = try Engine.probePeer(address, peer)
      _ = try info(peer)
      return result
    }
    if let presenceError { throw presenceError }
    throw EngineError("Device address unavailable. Make sure both devices are on the same network.")
  }
}

/// Bounded pinned status checks. Never pairs, sends input or changes routes.
enum DeviceAvailability {
  private static let lock = NSLock()
  private static var checking = false

  private static func status(_ state: String, _ reason: String = "", route: String? = nil) -> JSON {
    var result: JSON = ["state": state, "reason": reason]
    if let route { result["route"] = route }
    return result
  }

  static func read(_ peer: String) throws -> JSON {
    lock.lock()
    if checking { lock.unlock(); return status("checking") }
    checking = true
    lock.unlock()
    defer { lock.lock(); checking = false; lock.unlock() }
    try Engine.identity()
    func known() throws -> Bool { try Engine.isPaired(peer) }
    if try !known() { return status("unavailable", "This device is no longer paired.") }
    let usb = try Engine.usbState()
    if Devices.usbSelected(usb, peer), let id = usb["id"] as? String {
      // No LAN fallback once this carrier is selected; attachment alone never means Online.
      do {
        let presence = try Engine.resolveUsbPresence(peer, id)
        if try !known() { return status("unavailable", "This device is no longer paired.") }
        return status(presence["receiving"] as? Bool == true ? "online" : "receiving_off", route: "usb")
      } catch {
        return status("unavailable", "Couldn't verify the USB connection. Check connection to retry.", route: "usb")
      }
    }
    let address = LocalStore.peer(peer)?["address"] as? String ?? ""
    let host = try Engine.peerHosts()[peer] ?? ""
    do {
      let presence = try Devices.query(Devices.endpoints(address, host), peer)
      if try !known() { return status("unavailable", "This device is no longer paired.") }
      return status(presence["receiving"] as? Bool == true ? "online" : "receiving_off")
    } catch let unreachable {
      if let presence = try? Engine.resolveAccountPresence(peer, refresh: false) {
        if try !known() { return status("unavailable", "This device is no longer paired.") }
        return status(presence["receiving"] as? Bool == true ? "online" : "receiving_off", route: "internet")
      }
      // Presence is separate from active sessions; never fall back to a probe.
      let offline = Devices.offline(unreachable)
      return status(offline ? "offline" : "unavailable", offline
        ? "Couldn't reach this device on the local network."
        : "Couldn't verify availability. Check connection to retry.")
    }
  }
}
