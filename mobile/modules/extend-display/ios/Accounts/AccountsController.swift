import Foundation

/// Engine calls the account owner needs, as closures so tests can fake them and
/// the module can wire them to the `ext_*` C ABI (see `AccountEngine.live`).
public struct AccountEngine {
  /// `{"fingerprint": <hex>, "public_key": <hex>}` of this device's X25519 identity.
  public var publicIdentity: () throws -> [String: Any]
  /// Name registered with the account (the local device name).
  public var name: () -> String
  /// `ext_account_proof(server_key, challenge)`.
  public var proof: (_ serverKey: String, _ challenge: String) throws -> String
  /// `ext_replace_account_peers(scope, directory JSON)`.
  public var replacePeers: (_ scope: String, _ directory: String) throws -> Void
  /// `ext_clear_account_peers()`.
  public var clearPeers: () throws -> Void
  /// `ext_configure_account_relay(server, scope, device, token, fingerprint)`.
  public var configureRelay: (_ server: String, _ scope: String, _ device: String, _ token: String, _ fingerprint: String) throws -> Void

  public init(publicIdentity: @escaping () throws -> [String: Any], name: @escaping () -> String,
              proof: @escaping (String, String) throws -> String, replacePeers: @escaping (String, String) throws -> Void,
              clearPeers: @escaping () throws -> Void,
              configureRelay: @escaping (String, String, String, String, String) throws -> Void = { _, _, _, _, _ in }) {
    self.publicIdentity = publicIdentity
    self.name = name
    self.proof = proof
    self.replacePeers = replacePeers
    self.clearPeers = clearPeers
    self.configureRelay = configureRelay
  }
}

/// UI the account owner drives. `UIKitAccountPresenter` is the app implementation.
public protocol AccountPresenter: AnyObject {
  /// Shows the browser sign-in page. `closed` is called once when the user (or
  /// `dismiss`) closes it. Returns `dismiss`, which closes it without side effects.
  func openSignIn(_ url: URL, closed: @escaping () -> Void) throws -> () -> Void
  /// Opens an account website page (signup, account, recover) in the browser.
  func openPage(_ url: URL) throws
  /// Shows the native password/authenticator/recovery-code form. Throws only if
  /// the app can't present now (not in the foreground).
  func presentLogin(_ accounts: AccountsController) throws
}

/// Native account owner (port of Android's Accounts.java). React Native receives
/// only `state()`, a whitelist, never a credential. Each method matches the JS
/// function in `mobile/modules/extend-display/index.tsx`:
///
/// getAccountState → state(), resumeAccount → resume(), refreshAccount → refresh(),
/// startAccountBrowser → startBrowser(), startAccountPassword → startPassword(),
/// cancelAccountSignIn → cancel(), logoutAccount → logout(),
/// configureAccount → configure(_:), removeAccountDevice → removeDevice(_:),
/// openAccountPage → openPage(_:).
///
/// Methods block on the network (up to 12 s per request); call them off the main thread.
public final class AccountsController {
  public static let defaultServer = "https://extend.computer"
  static let serverKey = "computer.extend.account.server"
  static let platform = "ios"

  /// Called with the new `state()` JSON whenever it changes; any thread.
  public var onChange: ((String) -> Void)? {
    get { lock.locked { changeHandler } }
    set { lock.locked { changeHandler = newValue } }
  }

  private let lock = NSRecursiveLock(), operation = NSRecursiveLock()
  private let store: AccountCredentialStore
  private let engine: AccountEngine
  private let presenter: AccountPresenter
  private let http: AccountHTTP
  private let defaults: UserDefaults
  private let signInTimeout: TimeInterval
  private let worker = DispatchQueue(label: "computer.extend.accounts")
  private var refreshQueued = false
  private var origin: String
  private var error = "", email = "", deviceId = ""
  private var credential: AccountCredential?
  private var devices: [[String: Any]] = []
  private var loaded = false, storageError = false, busy = false, nativePending = false, closed = false
  private var generation: Int64 = 0
  private var updatedAt: UInt64 = 0, signInDeadline: UInt64 = 0
  private var browser: AccountBrowser?
  private var dismissBrowser: (() -> Void)?
  private var nativeTimeout: DispatchWorkItem?
  private var dismissNative: (() -> Void)?
  private var heartbeat: DispatchSourceTimer?
  private var changeHandler: ((String) -> Void)?
  private var lastEmitted: String?

  /// `server` overrides the saved server (tests); otherwise the server saved by
  /// `configure(_:)` in `defaults`, or extend.computer.
  public init(engine: AccountEngine, presenter: AccountPresenter,
              store: AccountCredentialStore = KeychainAccountCredentials(),
              defaults: UserDefaults = .standard, server: String? = nil, signInTimeout: TimeInterval = 300) throws {
    guard signInTimeout > 0, signInTimeout <= 300 else { throw AccountError("Invalid sign-in timeout.") }
    origin = try AccountProtocol.origin(server ?? defaults.string(forKey: Self.serverKey) ?? Self.defaultServer)
    self.engine = engine
    self.presenter = presenter
    self.store = store
    self.defaults = defaults
    self.signInTimeout = signInTimeout
    http = AccountHTTP()
  }
  deinit { heartbeat?.cancel() }

  /// Refreshes now and then every `interval` seconds (heartbeat + directory),
  /// like Android's 30 s schedule. Call `resume()` again when the app returns to
  /// the foreground; iOS suspends the timer in the background.
  public func start(heartbeatInterval: TimeInterval = 30) {
    lock.locked {
      guard !closed, heartbeat == nil else { return }
      let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
      timer.schedule(deadline: .now() + heartbeatInterval, repeating: heartbeatInterval, leeway: .seconds(1))
      timer.setEventHandler { [weak self] in self?.resume() }
      heartbeat = timer
      timer.resume()
    }
    resume()
  }

  private static func now() -> UInt64 { clock_gettime_nsec_np(CLOCK_MONOTONIC) }

  // MARK: State

  private func load() throws {
    try lock.locked {
      if closed { throw AccountError("Account service is closed.") }
      if loaded { if storageError { throw AccountError(error) }; return }
      do {
        let restored = try store.load(origin: origin)
        if let restored {
          guard AccountProtocol.hex(restored.token),
                (try? AccountProtocol.text(["email": restored.email], "email", 320)) != nil
          else { throw AccountError("Invalid saved login.") }
        }
        credential = restored
        email = restored?.email ?? ""
        deviceId = restored?.deviceId ?? ""
        loaded = true
        storageError = false
      } catch {
        storageError = true
        self.error = "Couldn't read saved login. Sign out locally and try again."
        throw AccountError(self.error)
      }
    }
  }

  /// `AccountState` JSON (same shape as Android).
  public func state() -> String {
    lock.locked {
      let fresh = updatedAt != 0 && Self.now() - updatedAt < 90_000_000_000
      let safe = devices.map { row -> [String: Any] in var row = row; if !fresh { row["online"] = false }; return row }
      return AccountProtocol.encode([
        "server": origin, "email": email.isEmpty ? NSNull() : email, "device_id": deviceId.isEmpty ? NSNull() : deviceId,
        "devices": safe, "browser_pending": browser != nil, "native_pending": nativePending, "busy": busy, "loaded": loaded,
        "storage_error": storageError, "directory_fresh": fresh, "error": error.isEmpty ? NSNull() : error,
      ] as [String: Any])
    }
  }

  private func notify() {
    let (next, handler): (String, ((String) -> Void)?) = lock.locked {
      let next = state()
      guard next != lastEmitted else { return (next, nil) }
      lastEmitted = next
      return (next, changeHandler)
    }
    handler?(next)
  }

  /// Queues a background refresh (heartbeat + directory) unless one is queued.
  public func resume() {
    lock.locked {
      guard !closed, !refreshQueued else { return }
      refreshQueued = true
      worker.async { [self] in
        do { _ = try refresh() } catch {
          lock.locked { if !closed && !storageError { self.error = "Couldn't refresh the account. Check your connection and retry." } }
        }
        lock.locked { refreshQueued = false }
        notify()
      }
    }
  }

  func prepare() throws { try operation.locked { try load() } }

  func nativeFailed(_ ticket: Int64) {
    defer { notify() }
    lock.locked {
      guard !closed, generation == ticket else { return }
      if nativePending { generation += 1; closeNativeLocked() }
      error = "Couldn't open native sign-in. Return to the app and retry."
    }
  }

  private func current(_ ticket: Int64) throws {
    if generation == ticket, signInDeadline != 0, Self.now() >= signInDeadline, browser != nil || nativePending {
      generation += 1
      closeBrowserLocked()
      closeNativeLocked()
      error = "Sign-in timed out. Start again."
    }
    if closed || generation != ticket { throw AccountError("Account operation canceled.") }
  }

  private func commit(_ ticket: Int64, _ next: AccountCredential) throws {
    try lock.locked {
      try current(ticket)
      try store.save(origin: origin, next)
      credential = next
      email = next.email
      deviceId = next.deviceId ?? ""
    }
  }

  private func accept(_ ticket: Int64, _ server: String, _ response: [String: Any]) throws {
    let token = try AccountProtocol.id(response, "token")
    do {
      guard let account = response["account"] as? [String: Any] else { throw AccountError("Invalid account response.") }
      let email = try AccountProtocol.text(account, "email", 320)
      try lock.locked {
        try commit(ticket, AccountCredential(token: token, email: email, keyVerified: false))
        signInDeadline = 0
      }
    } catch {
      revoke(server, token)
      throw error
    }
  }

  private func revoke(_ server: String, _ token: String) {
    _ = try? http.request(server, "/auth/logout", "POST", [:], token)
  }

  // MARK: Refresh

  /// Registers this device if needed, proves key possession once, heartbeats and
  /// reloads the account directory into the engine.
  @discardableResult
  public func refresh() throws -> String {
    defer { notify() }
    return try operation.locked {
      try load()
      let claimed: (Int64, AccountCredential?, String)? = lock.locked {
        if busy { return nil }
        busy = true
        return (generation, credential, origin)
      }
      guard let claimed else { return state() }
      let (ticket, saved, server) = claimed
      do {
        guard var next = saved else { try engine.clearPeers(); throw SignedOut() }
        let local = try engine.publicIdentity()
        let fingerprint = try AccountProtocol.id(local, "fingerprint")
        let user = try http.request(server, "/account", "GET", nil, next.token)
        let account = try AccountProtocol.id(user, "id")
        next.accountId = account
        next.email = try AccountProtocol.text(user, "email", 320)
        let name = engine.name()
        if next.deviceToken == nil || name != next.registeredName || fingerprint != next.registeredFingerprint {
          let registered = try http.request(server, "/devices", "POST", [
            "public_key": try AccountProtocol.id(local, "public_key"), "name": name, "platform": Self.platform,
          ], next.token)
          guard try AccountProtocol.id(registered, "fingerprint") == fingerprint else {
            throw AccountError("Device registration identity mismatch.")
          }
          next.deviceId = try AccountProtocol.id(registered, "id")
          next.deviceToken = try AccountProtocol.id(registered, "device_token")
          next.registeredName = name
          next.registeredFingerprint = fingerprint
          next.keyVerified = false
          try commit(ticket, next)
        }
        guard let device = next.deviceId, AccountProtocol.hex(device), let deviceToken = next.deviceToken else {
          throw AccountError("Invalid account device identifier.")
        }
        let prefix = "/devices/\(device)"
        if !next.keyVerified {
          let options = try http.request(server, prefix + "/proof/options", "POST", [:], deviceToken)
          let challenge = try AccountProtocol.id(options, "challenge"), key = try AccountProtocol.id(options, "server_key")
          let proof = try engine.proof(key, challenge)
          _ = try http.request(server, prefix + "/proof/verify", "POST", ["challenge": challenge, "proof": proof], deviceToken)
          next.keyVerified = true
          try commit(ticket, next)
        }
        _ = try http.request(server, prefix + "/heartbeat", "POST", [:], deviceToken)
        let directory = try AccountProtocol.directory(http.request(server, "/devices", "GET", nil, next.token))
        try lock.locked {
          try current(ticket)
          try commit(ticket, next)
          let scope = "\(server)/\(account)"
          try engine.replacePeers(scope, AccountProtocol.encode(directory))
          try engine.configureRelay(server, scope, device, deviceToken, fingerprint)
          devices = directory
          updatedAt = Self.now()
          error = ""
        }
      } catch let failure as AccountHTTPFailure {
        try lock.locked {
          guard generation == ticket else { return }
          if failure.status == 401 {
            do { try clearLocked() } catch { busy = false; throw error }
          }
          error = failure.message
        }
      } catch is SignedOut {
      } catch {
        lock.locked { if generation == ticket { self.error = "Couldn't refresh the account. Check your connection and retry." } }
      }
      lock.locked { busy = false }
      return state()
    }
  }

  // MARK: Browser sign-in

  /// Starts browser sign-in (passkeys, security keys, any website login) through
  /// the loopback PKCE handoff. Returns the state with `browser_pending`.
  @discardableResult
  public func startBrowser() throws -> String {
    defer { notify() }
    return try operation.locked {
      try load()
      let (ticket, server): (Int64, String) = try lock.locked {
        if credential != nil || browser != nil || nativePending || busy {
          throw AccountError("Sign out or cancel the current sign-in first.")
        }
        generation += 1
        busy = true
        error = ""
        return (generation, origin)
      }
      defer { lock.locked { busy = false } }
      var next: AccountBrowser?
      do {
        let info = try http.request(server, "/server", "GET", nil, nil)
        guard info["desktop_browser_login"] as? Bool == true else {
          throw BrowserUnsupported()
        }
        let pending = try AccountBrowser(origin: server, timeout: signInTimeout)
        next = pending
        try lock.locked {
          try current(ticket)
          browser = pending
          signInDeadline = Self.now() + UInt64(signInTimeout * 1e9)
        }
        let dismiss = try presenter.openSignIn(pending.url) { [weak self] in self?.browserClosed(ticket, pending) }
        let stale: Bool = lock.locked {
          guard generation == ticket, browser === pending else { return true }
          dismissBrowser = dismiss
          return false
        }
        if stale { dismiss() }
        pending.start(exchange: { [weak self] code, verifier in
          guard let self else { throw AccountError("Account service is closed.") }
          let value = try self.http.request(server, "/auth/desktop/exchange", "POST", ["code": code, "verifier": verifier], nil)
          try self.operation.locked {
            try self.accept(ticket, server, value)
            try self.lock.locked {
              try self.current(ticket)
              if self.browser === pending { self.browser = nil; self.dismissBrowser?(); self.dismissBrowser = nil }
              self.signInDeadline = 0
              self.error = ""
            }
          }
          self.notify()
          self.resume()
        }, failed: { [weak self] message in
          guard let self else { return }
          self.lock.locked {
            guard self.generation == ticket else { return }
            if self.browser === pending { self.closeBrowserLocked() }
            self.signInDeadline = 0
            self.error = message
          }
          self.notify()
        })
      } catch {
        next?.close()
        let message: String = lock.locked {
          if generation == ticket {
            if browser === next, next != nil { closeBrowserLocked() }
            signInDeadline = 0
            self.error = error is BrowserUnsupported ? BrowserUnsupported.message
              : (error as? AccountError)?.message == "Return to the app to sign in." ? "Return to the app to sign in."
              : "Couldn't open browser sign-in. Retry."
          }
          return self.error
        }
        throw AccountError(message)
      }
      lock.locked { busy = false }
      return state()
    }
  }

  private struct SignedOut: Error {}
  private struct BrowserUnsupported: Error {
    static let message = "This server provides an account API. Use Sign in with password."
  }

  /// The sign-in sheet closed. Cancels the flow only if it is still this one.
  private func browserClosed(_ ticket: Int64, _ pending: AccountBrowser) {
    let changed: Bool = lock.locked {
      guard generation == ticket, browser === pending else { return false }
      generation += 1
      dismissBrowser = nil
      closeBrowserLocked()
      signInDeadline = 0
      error = ""
      return true
    }
    if changed { notify() }
  }

  private func closeBrowserLocked() {
    browser?.close()
    browser = nil
    let dismiss = dismissBrowser
    dismissBrowser = nil
    dismiss?()
  }

  private func closeNativeLocked() {
    nativePending = false
    signInDeadline = 0
    nativeTimeout?.cancel()
    nativeTimeout = nil
    let dismiss = dismissNative
    dismissNative = nil
    dismiss?()
  }

  @discardableResult
  public func cancel() -> String {
    defer { notify() }
    return lock.locked {
      generation += 1
      closeBrowserLocked()
      closeNativeLocked()
      error = ""
      return state()
    }
  }

  // MARK: Sign-out and devices

  private func clearLocked() throws {
    credential = nil; email = ""; deviceId = ""; devices = []; updatedAt = 0; loaded = true
    var failure: Error?
    do { try engine.clearPeers() } catch { failure = error; self.error = "Couldn't close account connections. Retry signing out locally." }
    do { try store.clear(origin: origin) } catch { failure = error; self.error = "Couldn't remove saved login. Retry signing out locally." }
    storageError = failure != nil
    if failure != nil { throw AccountError(error) }
  }

  /// Signs out locally first, then revokes the session on the server. Manual
  /// pairings are untouched; only account-trusted peers are cleared.
  @discardableResult
  public func logout() throws -> String { defer { notify() }; return try logout(expected: nil) }

  private func logout(expected: Int64?) throws -> String {
    let (previous, server, logoutGeneration, storageFailure): (AccountCredential?, String, Int64, Error?) = try lock.locked {
      if let expected { try current(expected) }
      generation += 1
      closeBrowserLocked()
      closeNativeLocked()
      let previous = credential
      var failure: Error?
      do { try clearLocked(); error = "" } catch { failure = error }
      return (previous, origin, generation, failure)
    }
    if let previous {
      do { _ = try http.request(server, "/auth/logout", "POST", [:], previous.token) } catch {
        lock.locked {
          if storageFailure == nil, !closed, generation == logoutGeneration, origin == server, credential == nil {
            self.error = "Signed out locally. Server was unreachable; revoke the session in account settings."
          }
        }
      }
    }
    if let storageFailure { throw storageFailure }
    return state()
  }

  /// Changes the account server (signed out only). HTTPS, or HTTP on loopback.
  @discardableResult
  public func configure(_ raw: String) throws -> String {
    defer { notify() }
    let server = try AccountProtocol.origin(raw)
    try operation.locked {
      try lock.locked {
        try load()
        if credential != nil || browser != nil || nativePending || busy {
          throw AccountError("Sign out or cancel sign-in before changing servers.")
        }
        defaults.set(server, forKey: Self.serverKey)
        generation += 1
        try engine.clearPeers()
        origin = server
        loaded = false
        error = ""
        devices = []
        updatedAt = 0
        try load()
      }
    }
    resume()
    return state()
  }

  /// Removes an account device. Removing this device signs out.
  @discardableResult
  public func removeDevice(_ id: String) throws -> String {
    defer { notify() }
    guard AccountProtocol.hex(id) else { throw AccountError("Invalid account device.") }
    return try operation.locked {
      let (saved, ticket, server): (AccountCredential, Int64, String) = try lock.locked {
        try current(generation)
        guard let credential, !busy else { throw AccountError("Sign in first.") }
        return (credential, generation, origin)
      }
      do { _ = try http.request(server, "/devices/\(id)", "DELETE", nil, saved.token) } catch let failure as AccountHTTPFailure {
        try lock.locked { if generation == ticket && failure.status == 401 { try clearLocked(); error = failure.message } }
        throw failure
      }
      if id == saved.deviceId { return try logout(expected: ticket) }
      try lock.locked {
        try current(ticket)
        let kept = devices.filter { $0["id"] as? String != id }
        try engine.replacePeers("\(server)/\(saved.accountId ?? "")", AccountProtocol.encode(kept))
        devices = kept
      }
      return try refresh()
    }
  }

  /// Opens `signup`, `account` or `recover` on the account website.
  public func openPage(_ page: String) throws {
    guard ["signup", "account", "recover"].contains(page) else { throw AccountError("Unknown account page.") }
    let server: String = try lock.locked { try current(generation); return origin }
    guard try http.request(server, "/server", "GET", nil, nil)["desktop_browser_login"] as? Bool == true else {
      throw AccountError("This server provides only an API. Use its administrator's account website.")
    }
    try presenter.openPage(URL(string: "\(server)/\(page)")!)
  }

  // MARK: Native sign-in (password, authenticator or recovery code)

  /// Shows the native sign-in form. Passwords, MFA tickets and codes stay native.
  public func startPassword() throws {
    try prepare()
    try presenter.presentLogin(self)
  }

  func beginNative(dismiss: @escaping () -> Void) throws -> Int64 {
    try load()
    defer { notify() }
    return try lock.locked {
      if storageError { throw AccountError("Sign out locally before signing in.") }
      if credential != nil || browser != nil || nativePending || busy { throw AccountError("Cancel the current sign-in first.") }
      nativePending = true
      dismissNative = dismiss
      error = ""
      generation += 1
      let ticket = generation
      signInDeadline = Self.now() + UInt64(signInTimeout * 1e9)
      let timeout = DispatchWorkItem { [weak self] in
        guard let self else { return }
        let fired: Bool = self.lock.locked {
          guard self.generation == ticket, self.nativePending else { return false }
          self.generation += 1
          self.closeNativeLocked()
          self.error = "Sign-in timed out. Start again."
          return true
        }
        if fired { self.notify() }
      }
      nativeTimeout = timeout
      worker.asyncAfter(deadline: .now() + signInTimeout, execute: timeout)
      return ticket
    }
  }

  /// Password step. Returns `[:]` when signed in, or the server's
  /// `{"mfa_required": true, "ticket": .., "totp": .., "recovery": ..}`.
  func password(_ ticket: Int64, email: String, password: String) throws -> [String: Any] {
    try operation.locked {
      let server: String = try lock.locked { try current(ticket); return origin }
      let response = try http.request(server, "/auth/login", "POST", ["email": email, "password": password], nil)
      return try nativeResult(ticket, server, response)
    }
  }

  /// Second-factor step with the MFA ticket from `password`.
  func verify(_ ticket: Int64, secondFactor: String, code: String) throws -> [String: Any] {
    try operation.locked {
      let server: String = try lock.locked { try current(ticket); return origin }
      let response = try http.request(server, "/auth/mfa/verify", "POST", ["ticket": secondFactor, "code": code], nil)
      return try nativeResult(ticket, server, response)
    }
  }

  private func nativeResult(_ ticket: Int64, _ server: String, _ response: [String: Any]) throws -> [String: Any] {
    if response["mfa_required"] as? Bool == true {
      try lock.locked { try current(ticket) }
      _ = try AccountProtocol.id(response, "ticket")
      return response
    }
    try accept(ticket, server, response)
    try lock.locked {
      try current(ticket)
      nativePending = false
      dismissNative = nil
      signInDeadline = 0
      nativeTimeout?.cancel()
      nativeTimeout = nil
      error = ""
    }
    notify()
    resume()
    return [:]
  }

  /// The native form closed (cancel, swipe-down or success).
  func nativeClosed(_ ticket: Int64) {
    let changed: Bool = lock.locked {
      guard generation == ticket, nativePending else { return false }
      generation += 1
      dismissNative = nil
      closeNativeLocked()
      return true
    }
    if changed { notify() }
  }

  /// Runs native form work on the account worker (serialized with refreshes).
  func submitNative(_ task: @escaping () -> Void) throws {
    try lock.locked {
      if closed { throw AccountError("Account service is closed.") }
      worker.async(execute: task)
    }
  }

  /// Stops timers, ends any sign-in and clears account peers from the engine.
  /// The saved login stays for the next launch.
  public func close() {
    lock.locked {
      guard !closed else { return }
      closed = true
      generation += 1
      heartbeat?.cancel()
      heartbeat = nil
      closeBrowserLocked()
      closeNativeLocked()
      try? engine.clearPeers()
    }
  }
}

fileprivate extension NSRecursiveLock {
  func locked<T>(_ body: () throws -> T) rethrows -> T {
    lock()
    defer { unlock() }
    return try body()
  }
}
