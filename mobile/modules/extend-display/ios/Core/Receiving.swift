import Foundation

/// User-enabled availability for paired devices (Android: ReceivingService).
/// iOS has no foreground service: while enabled, the listeners run only while
/// the app is active. `pause()` on background keeps the setting; `resume()` on
/// becoming active restarts them.
enum Receiving {
  private static let lock = NSLock()
  private static var phase = "stopped"
  private static var epoch: UInt64 = 0
  private static var advertisement: BonjourAdvertisement?
  /// Set by the display renderer: decoder capabilities JSON for the display
  /// receiver (Android: MediaSession.capabilities). Nil leaves video off.
  static var displayCapabilities: (() -> String?)?

  static var enabled: Bool { LocalStore.receiving }

  static func setEnabled(_ enabled: Bool) throws {
    do { try LocalStore.setReceiving(enabled) } catch { throw EngineError("Couldn't save availability") }
    if enabled { start() } else { stop() }
  }

  static func resume() {
    guard enabled else { return }
    start()
  }

  static func pause() { stop() }

  static func state() -> String {
    lock.lock(); let current = phase; lock.unlock()
    let snapshot = (try? Engine.receivingState()) ?? [:]
    return JSONText.encode([
      "enabled": enabled,
      "state": current,
      "port": current == "available" ? snapshot["port"] as? Int ?? 0 : 0,
    ])
  }

  private static func start() {
    lock.lock()
    if phase == "available" || phase == "starting" { lock.unlock(); return }
    phase = "starting"
    epoch += 1
    let started = epoch
    lock.unlock()
    Engine.queue.async {
      do {
        let fingerprint = try Engine.fingerprint()
        lock.lock(); defer { lock.unlock() }
        guard epoch == started, enabled else { return }
        let name = Engine.localName()
        _ = try Engine.startReceiving(name)
        if let capabilities = displayCapabilities?() { try? Engine.startDisplayReceiver(capabilities) }
        let presence = try Engine.startPresence(name)
        advertisement = try? BonjourAdvertisement(
          type: ServiceType.presence, name: "extend-presence-\(fingerprint.prefix(12))",
          port: presence["port"] as? Int ?? 0, attributes: [("version", "1"), ("capability", "presence")])
        phase = "available"
      } catch {
        lock.lock(); defer { lock.unlock() }
        guard epoch == started else { return }
        shutdown()
        phase = "failed"
      }
    }
  }

  private static func stop() {
    lock.lock(); defer { lock.unlock() }
    epoch += 1
    shutdown()
    phase = "stopped"
  }

  private static func shutdown() {
    advertisement?.stop()
    advertisement = nil
    Engine.stopDisplayReceiver()
    Engine.stopPresence()
    Engine.stopReceiving()
  }
}
