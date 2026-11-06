import Foundation
import dnssd

// DNS-SD through the system mDNSResponder (Android: NsdManager). Raw multicast
// sockets would need the multicast entitlement, so the Rust engine's mdns-sd
// advertiser is never used on iOS. Service types (engine/src/discovery.rs):
// `_extendpair._tcp` (foreground pairing), `_extendprobe._tcp` (desktop
// probe), `_extendpresence._tcp` (availability). All must be listed in
// Info.plist `NSBonjourServices`; the first browse or registration triggers the
// local network permission prompt.
enum ServiceType {
  static let pairing = "_extendpair._tcp"
  static let probe = "_extendprobe._tcp"
  static let presence = "_extendpresence._tcp"
}

private func txtRecord(_ attributes: [(String, String)]) -> Data {
  var data = Data()
  for (key, value) in attributes {
    let entry = Array("\(key)=\(value)".utf8.prefix(255))
    data.append(UInt8(entry.count))
    data.append(contentsOf: entry)
  }
  return data
}

/// One registered service; withdrawn on `stop()` or deinit.
final class BonjourAdvertisement {
  private var ref: DNSServiceRef?

  init(type: String, name: String, port: Int, attributes: [(String, String)]) throws {
    guard (1...65535).contains(port) else { throw EngineError("Invalid service port") }
    let txt = txtRecord(attributes)
    var ref: DNSServiceRef?
    let error = txt.withUnsafeBytes { bytes in
      // A nil callback is allowed: registration happens in mDNSResponder and
      // lives until the reference is deallocated. Name conflicts auto-rename.
      DNSServiceRegister(&ref, 0, 0, name, type, nil, nil, UInt16(port).bigEndian,
                         UInt16(txt.count), bytes.baseAddress, nil, nil)
    }
    guard error == kDNSServiceErr_NoError, let ref else {
      throw EngineError("Couldn't advertise this device (\(error))")
    }
    self.ref = ref
  }
  func stop() {
    if let ref { DNSServiceRefDeallocate(ref) }
    ref = nil
  }
  deinit { stop() }
}

/// Foreground-only browsing. Discovery never grants identity or trust: every
/// candidate is only a route hint that pairing or a pinned probe must prove.
final class NearbyDiscovery {
  private final class Entry {
    let id: String
    let type: String
    weak var owner: NearbyDiscovery?
    let generation: UInt64
    var resolve: DNSServiceRef?
    var lookup: DNSServiceRef?
    var name = ""
    var port: UInt16 = 0
    var pairing = false
    var presence = false
    var valid = false
    var addresses: [String] = []
    init(id: String, type: String, owner: NearbyDiscovery, generation: UInt64) {
      self.id = id; self.type = type; self.owner = owner; self.generation = generation
    }
    func close() {
      if let resolve { DNSServiceRefDeallocate(resolve) }
      if let lookup { DNSServiceRefDeallocate(lookup) }
      resolve = nil; lookup = nil
    }
  }
  private final class Browse {
    weak var owner: NearbyDiscovery?
    let type: String
    let generation: UInt64
    var ref: DNSServiceRef?
    init(owner: NearbyDiscovery, type: String, generation: UInt64) {
      self.owner = owner; self.type = type; self.generation = generation
    }
  }

  private let types: [String]
  private let queue = DispatchQueue(label: "computer.extend.discovery")
  private var browsers: [Browse] = []
  private var entries: [String: Entry] = [:]
  private var order: [String] = []
  private var requested = false
  private var running = false
  private var generation: UInt64 = 0
  private var state = "idle"

  init(presenceOnly: Bool = false) {
    types = presenceOnly ? [ServiceType.presence] : [ServiceType.pairing, ServiceType.probe]
  }
  deinit {
    // Callbacks hold only weak owners; tear down synchronously.
    browsers.forEach { if let ref = $0.ref { DNSServiceRefDeallocate(ref) } }
    entries.values.forEach { $0.close() }
  }

  func start() { queue.async { self.requested = true; self.begin() } }
  func stop() { queue.async { self.requested = false; self.end() } }
  func suspend() { queue.async { self.end() } }
  func resume() { queue.async { if self.requested { self.begin() } } }

  func snapshot() -> JSON {
    queue.sync {
      let candidates: [JSON] = order.compactMap { id in
        guard let entry = entries[id], entry.valid, !entry.addresses.isEmpty else { return nil }
        return ["id": entry.id, "name": entry.name, "addresses": entry.addresses, "pairing": entry.pairing]
      }
      return ["state": state, "candidates": candidates]
    }
  }

  private func begin() {
    if running { return }
    running = true
    state = "searching"
    generation += 1
    for type in types {
      let browse = Browse(owner: self, type: type, generation: generation)
      let context = Unmanaged.passUnretained(browse).toOpaque()
      var ref: DNSServiceRef?
      let error = DNSServiceBrowse(&ref, 0, 0, type, "local.", { _, flags, interface, error, name, type, domain, context in
        guard let context else { return }
        let browse = Unmanaged<Browse>.fromOpaque(context).takeUnretainedValue()
        guard let owner = browse.owner, owner.current(browse.generation) else { return }
        guard error == kDNSServiceErr_NoError, let name, let type, let domain else {
          owner.failed(); return
        }
        owner.browsed(String(cString: name), type: String(cString: type), domain: String(cString: domain),
                      interface: interface, added: flags & kDNSServiceFlagsAdd != 0, service: browse.type)
      }, context)
      guard error == kDNSServiceErr_NoError, let ref else { end(); state = "failed"; return }
      DNSServiceSetDispatchQueue(ref, queue)
      browse.ref = ref
      browsers.append(browse)
    }
  }

  private func end() {
    running = false
    generation += 1
    state = "idle"
    for browse in browsers { if let ref = browse.ref { DNSServiceRefDeallocate(ref) } }
    browsers.removeAll()
    entries.values.forEach { $0.close() }
    entries.removeAll()
    order.removeAll()
  }

  private func current(_ epoch: UInt64) -> Bool { running && generation == epoch }
  private func failed() { end(); state = "failed" }

  private func browsed(_ name: String, type: String, domain: String, interface: UInt32, added: Bool, service: String) {
    let id = "\(service)/\(name)"
    guard added else {
      entries.removeValue(forKey: id)?.close()
      order.removeAll { $0 == id }
      return
    }
    guard entries[id] == nil, entries.count < 64 else { return }
    let entry = Entry(id: id, type: service, owner: self, generation: generation)
    entries[id] = entry
    order.append(id)
    var ref: DNSServiceRef?
    let context = Unmanaged.passUnretained(entry).toOpaque()
    let error = DNSServiceResolve(&ref, 0, interface, name, type, domain, { _, _, interface, error, _, host, port, length, txt, context in
      guard let context else { return }
      let entry = Unmanaged<Entry>.fromOpaque(context).takeUnretainedValue()
      guard let owner = entry.owner, owner.current(entry.generation) else { return }
      guard error == kDNSServiceErr_NoError, let host else { owner.drop(entry); return }
      owner.resolved(entry, host: String(cString: host), port: UInt16(bigEndian: port), interface: interface,
                     txt: txt.map { Data(bytes: $0, count: Int(length)) } ?? Data())
    }, context)
    guard error == kDNSServiceErr_NoError, let ref else { drop(entry); return }
    DNSServiceSetDispatchQueue(ref, queue)
    entry.resolve = ref
  }

  private func drop(_ entry: Entry) {
    entry.close()
    entries.removeValue(forKey: entry.id)
    order.removeAll { $0 == entry.id }
  }

  private static func attribute(_ txt: Data, _ key: String) -> String {
    txt.withUnsafeBytes { bytes -> String in
      var length: UInt8 = 0
      guard let base = bytes.baseAddress,
            let value = TXTRecordGetValuePtr(UInt16(txt.count), base, key, &length), length <= 128 else { return "" }
      return String(decoding: UnsafeRawBufferPointer(start: value, count: Int(length)), as: UTF8.self)
    }
  }

  private func resolved(_ entry: Entry, host: String, port: UInt16, interface: UInt32, txt: Data) {
    let presence = entry.type == ServiceType.presence
    let pairing = entry.type == ServiceType.pairing
    let capability = presence ? "presence" : pairing ? "pairing" : "probe"
    let name = Self.attribute(txt, "name").trimmingCharacters(in: .whitespacesAndNewlines)
    let validName = !name.isEmpty && name.utf8.count <= 100
      && !name.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
    guard Self.attribute(txt, "version") == "1", Self.attribute(txt, "capability") == capability,
          presence || validName, port > 0 else { drop(entry); return }
    entry.name = name
    entry.pairing = pairing
    entry.presence = presence
    entry.valid = true
    if entry.port != port { entry.addresses.removeAll() }
    entry.port = port
    if let lookup = entry.lookup { DNSServiceRefDeallocate(lookup); entry.lookup = nil }
    var ref: DNSServiceRef?
    let context = Unmanaged.passUnretained(entry).toOpaque()
    // IPv4 only, like the engine's listeners and Android's unscoped filter.
    let error = DNSServiceGetAddrInfo(&ref, 0, interface, DNSServiceProtocol(kDNSServiceProtocol_IPv4), host, { _, flags, _, error, _, address, _, context in
      guard let context, error == kDNSServiceErr_NoError, let address, address.pointee.sa_family == sa_family_t(AF_INET) else { return }
      let entry = Unmanaged<Entry>.fromOpaque(context).takeUnretainedValue()
      guard let owner = entry.owner, owner.current(entry.generation) else { return }
      var ipv4 = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
      var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
      guard inet_ntop(AF_INET, &ipv4, &text, socklen_t(INET_ADDRSTRLEN)) != nil else { return }
      owner.addressed(entry, String(cString: text), added: flags & kDNSServiceFlagsAdd != 0)
    }, context)
    guard error == kDNSServiceErr_NoError, let ref else { return }
    DNSServiceSetDispatchQueue(ref, queue)
    entry.lookup = ref
  }

  private func addressed(_ entry: Entry, _ host: String, added: Bool) {
    // Skip unusable endpoints: unspecified, loopback, link-local, multicast.
    if host == "0.0.0.0" || host.hasPrefix("127.") || host.hasPrefix("169.254.") { return }
    if let first = Int(host.split(separator: ".").first ?? ""), (224...239).contains(first) { return }
    let endpoint = "\(host):\(entry.port)"
    if added {
      if !entry.addresses.contains(endpoint) && entry.addresses.count < 8 { entry.addresses.append(endpoint) }
    } else {
      entry.addresses.removeAll { $0 == endpoint }
    }
  }
}

/// IPv4 addresses for the incoming pairing screen (Android: NetworkInterface).
enum LocalAddresses {
  static func ipv4() -> [String] {
    var result: [String] = []
    var list: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&list) == 0, let first = list else { return [] }
    defer { freeifaddrs(list) }
    for item in sequence(first: first, next: { $0.pointee.ifa_next }) {
      let flags = Int32(item.pointee.ifa_flags)
      guard flags & IFF_UP != 0, flags & IFF_LOOPBACK == 0,
            let address = item.pointee.ifa_addr, address.pointee.sa_family == sa_family_t(AF_INET) else { continue }
      var ipv4 = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
      var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
      guard inet_ntop(AF_INET, &ipv4, &text, socklen_t(INET_ADDRSTRLEN)) != nil else { continue }
      let host = String(cString: text)
      // Only Wi-Fi/Ethernet (en*) and Personal Hotspot (bridge*) reach a LAN
      // peer; cellular (pdp_ip), VPN (utun/ipsec) and CLAT addresses don't.
      let name = String(cString: item.pointee.ifa_name)
      if host.hasPrefix("169.254.") || !(name.hasPrefix("en") || name.hasPrefix("bridge")) || result.contains(host) { continue }
      result.append(host)
    }
    return result
  }
}
