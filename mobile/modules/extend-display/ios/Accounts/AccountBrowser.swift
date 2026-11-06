import Darwin
import Foundation

/// Ephemeral IPv4 loopback callback, identical to the desktop/Android PKCE
/// contract (port of AccountBrowser.java). The sign-in page sends the browser to
/// `http://127.0.0.1:<port>/callback?state=..&code=..`; only a request with the
/// random state is accepted, the listener closes after the first valid one, and
/// the code is useless without the verifier, which never leaves this object.
final class AccountBrowser {
  let state = AccountProtocol.randomHex()
  let port: UInt16
  private let verifier = AccountProtocol.verifier()
  private let origin: String
  private let timeout: TimeInterval
  private let lock = NSLock()
  private var listener: Int32
  private var client: Int32 = -1
  private var running = false
  private var closed = false

  init(origin: String, timeout: TimeInterval = 300) throws {
    guard timeout > 0, timeout <= 300 else { throw AccountError("Invalid sign-in timeout.") }
    self.origin = try AccountProtocol.origin(origin)
    self.timeout = timeout
    let fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
    guard fd >= 0 else { throw AccountError("Couldn't open browser sign-in. Retry.") }
    var address = sockaddr_in()
    address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    address.sin_family = sa_family_t(AF_INET)
    address.sin_port = 0
    address.sin_addr.s_addr = inet_addr("127.0.0.1")
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    let bound = withUnsafeMutablePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        bind(fd, $0, length) == 0 && listen(fd, 4) == 0 && getsockname(fd, $0, &length) == 0
      }
    }
    guard bound else { Darwin.close(fd); throw AccountError("Couldn't open browser sign-in. Retry.") }
    listener = fd
    port = UInt16(bigEndian: address.sin_port)
  }
  deinit { if listener >= 0 { Darwin.close(listener) } }

  var url: URL {
    URL(string: "\(origin)/desktop/connect?port=\(port)&state=\(state)&challenge=\(AccountProtocol.challenge(verifier))")!
  }

  /// `exchange` runs on the listener thread with the code and verifier; when it
  /// throws, the browser gets a 503 and `failed` is reported. `failed` is not
  /// called after `close()`.
  func start(exchange: @escaping (_ code: String, _ verifier: String) throws -> Void, failed: @escaping (String) -> Void) {
    lock.lock(); running = !closed; let go = running; lock.unlock()
    guard go else { return }
    let thread = Thread { [self] in run(exchange: exchange, failed: failed) }
    thread.name = "extend-account-browser"
    thread.start()
  }

  private var isClosed: Bool { lock.lock(); defer { lock.unlock() }; return closed }
  private static func now() -> UInt64 { clock_gettime_nsec_np(CLOCK_MONOTONIC) }

  private func run(exchange: (String, String) throws -> Void, failed: (String) -> Void) {
    let until = Self.now() + UInt64(timeout * 1e9)
    defer { finish() }
    while !isClosed, Self.now() < until {
      var poller = pollfd(fd: listener, events: Int16(POLLIN), revents: 0)
      guard poll(&poller, 1, 500) > 0 else { continue }
      let accepted = accept(listener, nil, nil)
      guard accepted >= 0 else { if isClosed { return }; continue }
      var one: Int32 = 1
      setsockopt(accepted, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
      lock.lock(); client = accepted; let stop = closed; lock.unlock()
      defer { lock.lock(); client = -1; lock.unlock(); Darwin.close(accepted) }
      if stop { return }
      // An unrelated local socket (timeout, garbage) can't consume the sign-in.
      guard let line = Self.requestLine(accepted, deadline: min(until, Self.now() + 2_000_000_000)) else { continue }
      guard let code = AccountProtocol.callback(line, state: state) else {
        Self.respond(accepted, "400 Bad Request", "Invalid sign-in callback.")
        continue
      }
      lock.lock(); Darwin.close(listener); listener = -1; lock.unlock() // No replay after the first valid callback.
      do {
        try exchange(code, verifier)
      } catch {
        Self.respond(accepted, "503 Service Unavailable", "Couldn't finish sign-in. Return to the app and retry.")
        if !isClosed { failed("Couldn't finish browser sign-in. Return to the app and retry.") }
        return
      }
      // The credential is already saved; a closed tab can't undo success.
      Self.write(accepted, "HTTP/1.1 303 See Other\r\nLocation: \(origin)/desktop/connected#close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
      return
    }
    if !isClosed { failed("Browser sign-in timed out. Start again.") }
  }

  private func finish() {
    lock.lock(); closed = true; running = false
    if listener >= 0 { Darwin.close(listener); listener = -1 }
    lock.unlock()
  }

  /// First line of the request head; nil on timeout, EOF or a head over 8 KiB.
  static func requestLine(_ fd: Int32, deadline: UInt64) -> String? {
    var bytes: [UInt8] = []
    var ending: UInt32 = 0
    while true {
      let now = now()
      guard now < deadline else { return nil }
      var poller = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
      let wait = Int32(min(2000, max(1, (deadline - now + 999_999) / 1_000_000)))
      guard poll(&poller, 1, wait) > 0 else { continue }
      var value: UInt8 = 0
      guard recv(fd, &value, 1, 0) == 1, bytes.count < 8192 else { return nil }
      bytes.append(value)
      ending = (ending << 8) | UInt32(value)
      if ending == 0x0d0a_0d0a {
        let head = String(decoding: bytes, as: UTF8.self)
        return head.range(of: "\r\n").map { String(head[..<$0.lowerBound]) } ?? ""
      }
    }
  }

  private static func respond(_ fd: Int32, _ status: String, _ text: String) {
    let body = Array(text.utf8)
    write(fd, "HTTP/1.1 \(status)\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n\(text)")
  }
  private static func write(_ fd: Int32, _ text: String) {
    let bytes = Array(text.utf8)
    var sent = 0
    while sent < bytes.count {
      let count = bytes[sent...].withUnsafeBytes { send(fd, $0.baseAddress, $0.count, 0) }
      if count <= 0 { return }
      sent += count
    }
  }

  /// Stops listening and interrupts a connected callback. Idempotent.
  func close() {
    lock.lock(); defer { lock.unlock() }
    closed = true
    if client >= 0 { shutdown(client, SHUT_RDWR) }
    // A running listener thread owns the descriptor and closes it within 500 ms;
    // closing it here could let the number be reused under its poll().
    if running { if listener >= 0 { shutdown(listener, SHUT_RDWR) } } else if listener >= 0 { Darwin.close(listener); listener = -1 }
  }
}
