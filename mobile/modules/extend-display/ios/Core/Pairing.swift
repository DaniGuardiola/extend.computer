import Foundation
import UIKit

/// Thread-safe "the user is looking at the app" flag (Android: hasWindowFocus).
/// Updated on the main thread from UIApplication notifications.
enum AppFocus {
  private static let lock = NSLock()
  private static var value = false
  private static var observers: [NSObjectProtocol] = []

  static var active: Bool { lock.lock(); defer { lock.unlock() }; return value }
  static func set(_ active: Bool) { lock.lock(); value = active; lock.unlock() }

  @MainActor static func install() {
    guard observers.isEmpty else { return }
    set(UIApplication.shared.applicationState == .active)
    let center = NotificationCenter.default
    observers = [
      center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in set(true) },
      center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { _ in set(false) },
    ]
  }
  static func require(_ message: String) throws {
    if !active { throw EngineError(message) }
  }
}

private func savePairedRoute(_ result: inout JSON, saved: inout String?) {
  guard result["state"] as? String == "paired", let peer = result["peer"] as? String, peer != saved else { return }
  // Trust is already committed. Save the selected route as a hint; pinned
  // presence verifies it later.
  let metadata: JSON = ["address": result["address"] as? String ?? "", "name": result["name"] as? String ?? ""]
  do {
    try LocalStore.setPeer(peer, metadata)
    saved = peer
  } catch {
    result["error"] = "Paired device, but its address could not be saved"
  }
}

/// Foreground pairing listener plus its DNS-SD advertisement. The code is never
/// part of DNS-SD.
final class IncomingPairing {
  private let lock = NSLock()
  private var advertisement: BonjourAdvertisement?

  func start(name: String) throws -> String {
    lock.lock(); defer { lock.unlock() }
    stopLocked()
    let fingerprint = try Engine.fingerprint()
    let state = try Engine.startIncoming(name)
    if let port = state["port"] as? Int {
      // Manual IP pairing keeps working if advertising fails.
      advertisement = try? BonjourAdvertisement(
        type: ServiceType.pairing, name: "extend-computer-\(fingerprint.prefix(12))", port: port,
        attributes: [("version", "1"), ("capability", "pairing"), ("name", name), ("discovery_id", fingerprint)])
    }
    return withAddresses(state)
  }
  func state() throws -> String {
    lock.lock(); defer { lock.unlock() }
    let snapshot = try Engine.incomingState()
    if snapshot["code"] == nil || snapshot["code"] is NSNull { withdraw() }
    return withAddresses(snapshot)
  }
  func stop() {
    lock.lock(); defer { lock.unlock() }
    stopLocked()
  }
  private func stopLocked() {
    withdraw()
    Engine.stopIncoming()
  }
  private func withdraw() {
    advertisement?.stop()
    advertisement = nil
  }
  private func withAddresses(_ snapshot: JSON) -> String {
    var result = snapshot
    let port = snapshot["port"] as? Int ?? 0
    result["addresses"] = port == 0 ? [] : LocalAddresses.ipv4().map { "\($0):\(port)" }
    return JSONText.encode(result)
  }
}

/// Reserve before async start so leaving the UI also cancels queued startup.
final class ManualPairing {
  private let lock = NSLock()
  private var nextId: UInt64 = 0
  private var operation: String?
  private var started = false
  private var savedPeer: String?

  func reserve() throws -> String {
    lock.lock(); defer { lock.unlock() }
    try AppFocus.require("Return to the app to pair a device")
    if operation != nil { throw EngineError("Pairing already in progress") }
    nextId += 1
    let id = String(nextId)
    operation = id; started = false; savedPeer = nil
    return id
  }
  func start(id: String, address: String, code: String, name: String) throws -> String {
    // Identity startup may wait; cancel doesn't take that path.
    try Engine.identity()
    lock.lock(); defer { lock.unlock() }
    try require(id)
    if !AppFocus.active { stopLocked(id); throw EngineError("Return to the app to pair a device") }
    if started { throw EngineError("Pairing already in progress") }
    let result = try Engine.startManualPairing(id, address.trimmingCharacters(in: .whitespaces), code,
                                               name.trimmingCharacters(in: .whitespacesAndNewlines))
    started = true
    return JSONText.encode(result)
  }
  func state(id: String) throws -> String {
    lock.lock(); defer { lock.unlock() }
    try require(id)
    if !started { return JSONText.encode(["id": id, "state": "starting", "error": ""]) }
    var result = try Engine.manualPairingState(id)
    savePairedRoute(&result, saved: &savedPeer)
    return JSONText.encode(result)
  }
  func stop(id: String) {
    lock.lock(); defer { lock.unlock() }
    stopLocked(id)
  }
  func stop() {
    lock.lock(); defer { lock.unlock() }
    if let operation { stopLocked(operation) }
  }
  private func stopLocked(_ id: String) {
    // Stale cleanup must never stop a replacement reservation.
    guard operation == id else { return }
    if started { Engine.stopManualPairing(id) }
    operation = nil; started = false; savedPeer = nil
  }
  private func require(_ id: String) throws {
    if operation != id { throw EngineError("Pairing operation changed") }
  }
}

/// One foreground visual operation; JS sees only public verification data.
final class VisualPairing {
  private let lock = NSLock()
  private var operation: String?
  private var savedPeer: String?

  func start(address: String, name: String, localName: String) throws -> String {
    try AppFocus.require("Return to the app to pair a device")
    try Engine.identity()
    lock.lock(); defer { lock.unlock() }
    if let operation {
      let state = (try? Engine.visualPairingState(operation))?["state"] as? String
      if state == "connecting" || state == "verify" { throw EngineError("Pairing already in progress") }
      stopLocked()
    }
    let result = try Engine.startVisualPairing(address.trimmingCharacters(in: .whitespaces),
                                               name.trimmingCharacters(in: .whitespacesAndNewlines), localName)
    operation = result["id"] as? String
    savedPeer = nil
    if !AppFocus.active {
      stopLocked()
      throw EngineError("Return to the app to pair a device")
    }
    return JSONText.encode(result)
  }
  func state(id: String) throws -> String {
    lock.lock(); defer { lock.unlock() }
    try require(id)
    var result = try Engine.visualPairingState(id)
    savePairedRoute(&result, saved: &savedPeer)
    return JSONText.encode(result)
  }
  func respond(id: String, request: String, allow: Bool) throws {
    lock.lock(); defer { lock.unlock() }
    try require(id)
    if allow && !AppFocus.active {
      stopLocked()
      throw EngineError("Return to the app to confirm pairing")
    }
    try Engine.respondVisualPairing(id, request, allow)
  }
  func stop(id: String) throws {
    lock.lock(); defer { lock.unlock() }
    try require(id)
    stopLocked()
  }
  func stop() {
    lock.lock(); defer { lock.unlock() }
    stopLocked()
  }
  private func stopLocked() {
    if let operation { Engine.stopVisualPairing(operation) }
    operation = nil; savedPeer = nil
  }
  private func require(_ id: String) throws {
    if operation != id { throw EngineError("Pairing operation changed") }
  }
}
