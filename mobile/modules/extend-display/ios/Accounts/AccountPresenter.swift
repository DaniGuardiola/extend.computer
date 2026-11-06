#if canImport(UIKit)
import AuthenticationServices
import UIKit

/// App presenter: browser sign-in in an ASWebAuthenticationSession, account
/// pages in Safari, and the native login sheet.
///
/// The sign-in page finishes by navigating to the app's `http://127.0.0.1:<port>`
/// loopback listener (see AccountBrowser). The session sheet stays over the
/// app, so the app remains in the foreground and its listener keeps running;
/// the session's own callback scheme is never used and the controller dismisses
/// the sheet once the code is exchanged.
public final class UIKitAccountPresenter: NSObject, AccountPresenter, ASWebAuthenticationPresentationContextProviding {
  /// Unused callback scheme; completion arrives through the loopback listener.
  static let callbackScheme = "extendcomputer-signin"
  private let ephemeral: Bool
  private var sessions: [ObjectIdentifier: ASWebAuthenticationSession] = [:]

  /// `ephemeral` false (default) shares Safari's website login, so a signed-in
  /// user only approves; iOS then asks once whether the app may use the site.
  public init(ephemeral: Bool = false) { self.ephemeral = ephemeral }

  static func onMain<T>(_ body: () throws -> T) rethrows -> T {
    Thread.isMainThread ? try body() : try DispatchQueue.main.sync(execute: body)
  }

  static func activeWindow() -> UIWindow? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
    return scenes.flatMap(\.windows).first(where: \.isKeyWindow) ?? scenes.first?.windows.first
  }

  public func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    Self.activeWindow() ?? ASPresentationAnchor()
  }

  public func openSignIn(_ url: URL, closed: @escaping () -> Void) throws -> () -> Void {
    try Self.onMain {
      guard Self.activeWindow() != nil else { throw AccountError("Return to the app to sign in.") }
      var key: ObjectIdentifier?
      let finished: (URL?, Error?) -> Void = { [weak self] _, _ in
        if let key { self?.sessions[key] = nil }
        closed()
      }
      let session: ASWebAuthenticationSession
      if #available(iOS 17.4, *) {
        session = ASWebAuthenticationSession(url: url, callback: .customScheme(Self.callbackScheme), completionHandler: finished)
      } else {
        session = ASWebAuthenticationSession(url: url, callbackURLScheme: Self.callbackScheme, completionHandler: finished)
      }
      session.presentationContextProvider = self
      session.prefersEphemeralWebBrowserSession = ephemeral
      key = ObjectIdentifier(session)
      sessions[key!] = session
      guard session.start() else {
        sessions[key!] = nil
        throw AccountError("Couldn't open browser sign-in. Retry.")
      }
      return { DispatchQueue.main.async { session.cancel() } }
    }
  }

  public func openPage(_ url: URL) throws {
    DispatchQueue.main.async { UIApplication.shared.open(url) }
  }

  public func presentLogin(_ accounts: AccountsController) throws {
    try Self.onMain {
      guard let window = Self.activeWindow(), var top = window.rootViewController else {
        throw AccountError("Return to the app to sign in.")
      }
      while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
      let form = AccountLoginViewController(accounts: accounts)
      do { try form.open() } catch { accounts.nativeFailed(-1); return }
      top.present(form.container, animated: true)
    }
  }
}
#endif
