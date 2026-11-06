import Foundation

/// Non-2xx account response. Port of Android's AccountHttp.Failure.
public struct AccountHTTPFailure: LocalizedError, CustomStringConvertible {
  public let status: Int
  public var message: String {
    status == 401 ? "Your login or code expired. Sign in again."
      : status == 429 ? "Too many requests. Wait and try again." : "Account request failed. Try again."
  }
  public var errorDescription: String? { message }
  public var description: String { message }
}

/// Blocking JSON client for the account API (port of AccountHttp.java).
/// TLS uses platform trust; bearer credentials never follow a redirect; no
/// cookies or cache; 12 s deadline; 256 KiB response cap.
final class AccountHTTP {
  static let limit = 256 * 1024
  static let timeout: TimeInterval = 12

  func request(_ origin: String, _ path: String, _ method: String, _ body: [String: Any]?, _ token: String?) throws -> [String: Any] {
    guard (try? AccountProtocol.origin(origin)) == origin, path.hasPrefix("/"), !path.contains(".."),
          token == nil || AccountProtocol.hex(token), let url = URL(string: origin + "/v1" + path)
    else { throw AccountError("Invalid account request.") }
    var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalAndRemoteCacheData, timeoutInterval: Self.timeout)
    request.httpMethod = method
    request.httpShouldHandleCookies = false
    request.setValue(origin, forHTTPHeaderField: "Origin")
    request.setValue("desktop", forHTTPHeaderField: "X-Extend-Client")
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
    if let body {
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.httpBody = try JSONSerialization.data(withJSONObject: body)
    }
    let call = Call()
    let configuration = URLSessionConfiguration.ephemeral
    configuration.urlCache = nil
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.urlCredentialStorage = nil
    configuration.timeoutIntervalForRequest = Self.timeout
    configuration.timeoutIntervalForResource = Self.timeout
    let session = URLSession(configuration: configuration, delegate: call, delegateQueue: nil)
    defer { session.invalidateAndCancel() }
    session.dataTask(with: request).resume()
    guard call.done.wait(timeout: .now() + Self.timeout + 1) == .success else {
      throw AccountError("Couldn't reach the account server. Check your connection and try again.")
    }
    let (status, data, failed) = call.result()
    if let status, !(200..<300).contains(status) { throw AccountHTTPFailure(status: status) }
    guard !failed, status != nil else {
      throw AccountError("Couldn't reach the account server. Check your connection and try again.")
    }
    if data.isEmpty { return [:] }
    guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
      throw AccountError("Couldn't reach the account server. Check your connection and try again.")
    }
    return object
  }

  private final class Call: NSObject, URLSessionDataDelegate {
    let done = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var status: Int?
    private var data = Data()
    private var failed = false
    func result() -> (Int?, Data, Bool) { lock.lock(); defer { lock.unlock() }; return (status, data, failed) }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
      completionHandler(nil) // A redirect is returned as its 3xx status, never followed.
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
      lock.lock(); status = (response as? HTTPURLResponse)?.statusCode; lock.unlock()
      completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive chunk: Data) {
      lock.lock()
      if data.count + chunk.count > AccountHTTP.limit { failed = true; lock.unlock(); dataTask.cancel(); return }
      data.append(chunk)
      lock.unlock()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
      lock.lock(); if error != nil { failed = true }; lock.unlock()
      done.signal()
    }
  }
}
