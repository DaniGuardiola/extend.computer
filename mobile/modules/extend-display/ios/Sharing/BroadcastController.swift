import Foundation
import ReplayKit
import UIKit

/// App side of iPhone screen sharing (Android: ScreenSharingService's static API).
///
/// State machine reported by `state()`:
///   idle → consent (picker shown, request written) → connecting (extension
///   started, engine negotiating) → streaming → closed | failed.
/// `consent` falls back to `closed` with an error when no broadcast starts
/// within `consentTimeout` (the picker can't report being dismissed).
/// The extension owns capture; this class only writes requests to the App
/// Group, presents the system picker and reads back what the extension publishes.
final class BroadcastController {
  static let shared = BroadcastController()

  static let consentTimeout: TimeInterval = 30
  static let updateTimeout: TimeInterval = 20

  private let lock = NSLock()
  private var session: String?
  private var peer = ""
  private var consentDeadline: Date?
  /// Terminal result kept after the extension's state goes stale or is replaced.
  private var ended: (state: String, error: String)?
  private var picker: RPSystemBroadcastPickerView?

  /// Called (on an arbitrary thread) whenever the extension publishes a change.
  var onChange: (() -> Void)?

  private init() {
    BroadcastGroup.observe(.state) { [weak self] in self?.onChange?() }
  }

  /// Whether the App Group and extension were configured by the config plugin.
  var available: Bool { BroadcastGroup.defaults != nil && BroadcastGroup.extensionIdentifier != nil }

  /// Writes the request and presents the system broadcast picker preselected to
  /// our extension. Must be called on the main thread. `address` is the engine
  /// route string the extension passes to `start_display_sender`.
  func start(peer: String, address: String, preferences: SharingPreferences) throws {
    dispatchPrecondition(condition: .onQueue(.main))
    guard available, let extensionIdentifier = BroadcastGroup.extensionIdentifier else {
      throw SharingError("Screen sharing isn't set up in this build")
    }
    let current = state()
    guard !["consent", "connecting", "streaming"].contains(current["state"] as? String ?? "") else {
      throw SharingError("Screen sharing is already active")
    }
    let request = BroadcastRequest(peer: peer, address: address, preferences: preferences)
    BroadcastGroup.write(BroadcastGroup.requestKey, request.dictionary)
    lock.lock()
    session = request.session; self.peer = peer; ended = nil
    consentDeadline = Date().addingTimeInterval(Self.consentTimeout)
    lock.unlock()
    do { try presentPicker(extensionIdentifier) } catch {
      lock.lock(); ended = ("failed", "Couldn't open screen sharing consent"); consentDeadline = nil; lock.unlock()
      BroadcastGroup.write(BroadcastGroup.requestKey, nil)
      throw error
    }
  }

  /// `{state, error, peer[, encoded, resizes, transport]}` like Android's
  /// ScreenSharingService.state().
  func state() -> [String: Any] {
    lock.lock(); defer { lock.unlock() }
    let remote = BroadcastGroup.read(BroadcastGroup.stateKey)
    let remoteState = remote?["state"] as? String ?? ""
    let fresh = (remote?["updated"] as? NSNumber).map { Date().timeIntervalSince1970 - $0.doubleValue < BroadcastGroup.staleAfter } ?? false
    let ours = session != nil && remote?["session"] as? String == session

    if let remote, fresh, ["connecting", "streaming"].contains(remoteState) {
      // A running extension wins, even one started before the app relaunched.
      consentDeadline = nil
      if ours || session == nil { session = remote["session"] as? String; peer = remote["peer"] as? String ?? peer }
      dismissPicker()
      return snapshot(remote, state: remoteState, error: "")
    }
    if let remote, ours, ["closed", "failed"].contains(remoteState) {
      consentDeadline = nil
      ended = (remoteState, remote["error"] as? String ?? "")
      dismissPicker()
      return snapshot(remote, state: remoteState, error: ended!.error)
    }
    if ours, ["connecting", "streaming"].contains(remoteState) {
      // Our extension stopped publishing without saying goodbye (killed, crashed).
      consentDeadline = nil
      ended = ("failed", "Screen sharing ended unexpectedly")
    }
    if let deadline = consentDeadline {
      if Date() < deadline { return ["state": "consent", "error": "", "peer": peer] }
      consentDeadline = nil
      ended = ("closed", "Screen sharing wasn't started")
      BroadcastGroup.write(BroadcastGroup.requestKey, nil)
      dismissPicker()
    }
    if let ended { return ["state": ended.state, "error": ended.error, "peer": peer] }
    return ["state": "idle", "error": "", "peer": ""]
  }

  /// Applies new preferences to a running broadcast. Resolves with the
  /// extension's answer: `{applied, error, preferences}` (Android: update()).
  /// The caller persists the preferences on success.
  func update(peer: String, preferences: SharingPreferences) async -> [String: Any] {
    let current = state()
    guard current["state"] as? String == "streaming", current["peer"] as? String == peer else {
      return ["applied": false, "error": "No active sharing session for this device", "preferences": ""]
    }
    let id = UUID().uuidString
    BroadcastGroup.write(BroadcastGroup.updateKey, ["id": id, "preferences": preferences.dictionary])
    BroadcastGroup.post(.update)
    let deadline = Date().addingTimeInterval(Self.updateTimeout)
    while Date() < deadline {
      if let result = BroadcastGroup.read(BroadcastGroup.updateResultKey), result["id"] as? String == id {
        let applied = result["applied"] as? Bool ?? false
        let prefs = (result["preferences"] as? [String: Any]).flatMap { try? JSONSerialization.data(withJSONObject: $0) }
          .flatMap { String(data: $0, encoding: .utf8) } ?? ""
        return ["applied": applied, "error": result["error"] as? String ?? "", "preferences": applied ? prefs : ""]
      }
      if ["closed", "failed", "idle"].contains(state()["state"] as? String ?? "") { break }
      try? await Task.sleep(nanoseconds: 50_000_000)
    }
    return ["applied": false, "error": "Screen sharing didn't apply the settings", "preferences": ""]
  }

  /// Asks the extension to finish the broadcast; cancels a pending consent.
  func stop() {
    lock.lock()
    if consentDeadline != nil { consentDeadline = nil; ended = ("closed", "Screen sharing wasn't started") }
    lock.unlock()
    BroadcastGroup.write(BroadcastGroup.requestKey, nil)
    BroadcastGroup.post(.stop)
    DispatchQueue.main.async { self.lock.lock(); self.dismissPicker(); self.lock.unlock() }
  }

  // MARK: Picker

  private func presentPicker(_ extensionIdentifier: String) throws {
    guard let window = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
      .first(where: { $0.activationState == .foregroundActive })?.keyWindow else {
      throw SharingError("Open extend.computer to start sharing")
    }
    lock.lock(); dismissPicker(); lock.unlock()
    let picker = RPSystemBroadcastPickerView(frame: CGRect(x: -100, y: -100, width: 44, height: 44))
    picker.preferredExtension = extensionIdentifier
    picker.showsMicrophoneButton = false
    picker.isUserInteractionEnabled = false
    picker.alpha = 0.01
    window.addSubview(picker)
    // The picker has no API to open itself; its only control is a button.
    guard let button = picker.subviews.compactMap({ $0 as? UIButton }).first else {
      picker.removeFromSuperview()
      throw SharingError("Couldn't open screen sharing consent")
    }
    button.sendActions(for: .touchUpInside)
    lock.lock(); self.picker = picker; lock.unlock()
  }

  /// Call with `lock` held.
  private func dismissPicker() {
    guard let picker else { return }
    self.picker = nil
    if Thread.isMainThread { picker.removeFromSuperview() } else { DispatchQueue.main.async { picker.removeFromSuperview() } }
  }

  private func snapshot(_ remote: [String: Any], state: String, error: String) -> [String: Any] {
    var result: [String: Any] = ["state": state, "error": error, "peer": remote["peer"] as? String ?? peer]
    for key in ["encoded", "resizes", "transport"] { if let value = remote[key] { result[key] = value } }
    return result
  }
}
