#if EXTEND_ENGINE && canImport(UIKit)
import Foundation

/// The app's single account owner (Android: Accounts.get(context)), wired to
/// the engine and the UIKit presenter. Created on first use; the heartbeat
/// starts with it.
enum SharedAccounts {
  private static let lock = NSLock()
  private static var instance: AccountsController?

  static func get() throws -> AccountsController {
    lock.lock(); defer { lock.unlock() }
    if let instance { return instance }
    let made = try AccountsController(
      engine: .live(identity: { try Engine.identity() }, name: { Engine.localName() }),
      presenter: UIKitAccountPresenter())
    instance = made
    made.start()
    return made
  }

  /// Foreground refresh (Android OnActivityEntersForeground); creating the
  /// owner already refreshes once.
  static func resume() {
    lock.lock(); let current = instance; lock.unlock()
    if let current { current.resume() } else { _ = try? get() }
  }
}
#endif
