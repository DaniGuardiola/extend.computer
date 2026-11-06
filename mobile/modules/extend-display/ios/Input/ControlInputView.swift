import UIKit

struct InputError: LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}

/// Port of android/.../ExtendControlView.kt: a relative trackpad plus remote
/// keyboard for the dedicated control session. Touch and physical keys stay
/// native; the host (the `Control` Expo view) receives status maps only.
final class ControlInputView: RemoteKeyInputView {
  /// Engine entry points (NativeEngine.controlState / controlEvent / stopControl).
  struct Engine {
    var controlState: () throws -> String
    var controlEvent: (_ id: String, _ json: String) throws -> Void
    var stopControl: (_ id: String) -> Void
  }

  /// Same maps as Android's onStatus: state, error, shortcutPolicy,
  /// shortcutToken; or state "connected" + inputError for rejected text.
  var onStatus: (([String: Any]) -> Void)?
  /// True while connected (Android keepScreenOn).
  var onKeepAwake: ((Bool) -> Void)?

  private let engine: Engine
  private var id: String?
  private var connected = false
  private var lastPhase = ""
  private var lastSent: Int64 = -1
  private var keyboard = false
  private var imeGeneration = 0
  private var textSupported = false
  private var shortcutPolicy: KeyboardShortcutPolicy?
  private var pollWork: DispatchWorkItem?
  private var touches: [UITouch] = []
  private lazy var gesture = TrackpadGesture(slop: 8 * pixelScale) { [weak self] command in
    _ = self?.send(command.wire)
  }
  private lazy var keyboardState = DisplayKeyboard(textAllowed: { [weak self] in self?.textSupported ?? false }) { [weak self] command in
    self?.send(ControlWire.keyboard(command)) ?? false
  }

  init(engine: Engine) {
    self.engine = engine
    super.init(frame: .zero)
    isMultipleTouchEnabled = true
    isOpaque = false
    backgroundColor = .clear
    contentMode = .redraw
    isAccessibilityElement = true
    accessibilityLabel = "Remote trackpad"
    let center = NotificationCenter.default
    center.addObserver(self, selector: #selector(appResigned), name: UIApplication.willResignActiveNotification, object: nil)
    center.addObserver(self, selector: #selector(appActivated), name: UIApplication.didBecomeActiveNotification, object: nil)
    center.addObserver(self, selector: #selector(windowFocusChanged), name: UIWindow.didResignKeyNotification, object: nil)
    center.addObserver(self, selector: #selector(windowFocusChanged), name: UIWindow.didBecomeKeyNotification, object: nil)
  }

  deinit {
    pollWork?.cancel()
    if let id { engine.stopControl(id) }
  }

  // MARK: Props

  var sessionId: String? {
    get { id }
    set {
      if id == newValue { return }
      stop()
      id = newValue
      schedulePoll(after: 0)
    }
  }

  var keyboardEnabled: Bool {
    get { keyboard }
    set {
      imeGeneration += 1
      if !newValue { keyboardState.release() }
      keyboard = newValue
      resetComposition()
      if newValue {
        if !isFirstResponder { _ = becomeFirstResponder() }
      }
      // Swap between the software keyboard and hardware-only input.
      reloadInputViews()
    }
  }

  func editingShortcut(token: String, action: String) throws -> Bool {
    guard let shortcut = EditingShortcut.fromWire(action) else { throw InputError("Unknown shortcut") }
    guard token == shortcutToken() else { throw InputError("Keyboard input changed. Try again.") }
    guard let policy = shortcutPolicy else { throw InputError("Keyboard input changed. Try again.") }
    guard keyboardState.canShortcut() else { throw InputError("Release physical keys before using shortcuts.") }
    if keyboardState.shortcut(shortcut, policy: policy) { return true }
    // A partial chord must close this exact session so the receiver releases it.
    if let id { engine.stopControl(id) }
    connected = false
    shortcutPolicy = nil
    return false
  }

  // MARK: State

  private var pixelScale: Double {
    let scale = window?.screen.scale ?? traitCollection.displayScale
    return Double(scale > 0 ? scale : 2)
  }

  private var windowFocused: Bool { UIApplication.shared.applicationState == .active && window?.isKeyWindow == true }
  private func canTouch() -> Bool { connected && isShownInWindow && windowFocused }
  private func shortcutToken() -> String? {
    guard keyboard, canTouch(), shortcutPolicy != nil, let id else { return nil }
    return "\(id):\(imeGeneration)"
  }

  private func status(_ values: [String: Any]) { onStatus?(values) }

  @discardableResult
  private func send(_ json: String) -> Bool {
    guard connected, let id else { return false }
    do {
      try engine.controlEvent(id, json)
      return true
    } catch {
      connected = false
      status(["state": "failed", "error": error.localizedDescription.isEmpty ? "Control ended" : error.localizedDescription])
      return false
    }
  }

  private func schedulePoll(after delay: TimeInterval) {
    pollWork?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.poll() }
    pollWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
  }

  private func poll() {
    guard let session = id else { return }
    do {
      guard let data = try engine.controlState().data(using: .utf8),
            let state = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let phase = state["state"] as? String else { throw InputError("Control ended") }
      if (state["id"] as? String) != session {
        connected = false
        status(["state": "closed", "error": "Control session replaced"])
        return
      }
      let wasConnected = connected
      connected = phase == "connected"
      if connected && !wasConnected { recoverKeyboard() }
      onKeepAwake?(connected)
      let sent = (state["sent"] as? NSNumber)?.int64Value ?? 0
      if phase != lastPhase || (sent > 0 && sent / 32 != lastSent / 32) {
        NSLog("ExtendControl: Control stats: %@", String(describing: state))
        lastPhase = phase
        lastSent = sent
      }
      textSupported = connected && (state["text_supported"] as? Bool ?? false)
      shortcutPolicy = connected ? KeyboardShortcutPolicy.fromWire(state["shortcut_policy"] as? String) : nil
      if connected {
        gesture.remoteWidth = (state["width"] as? NSNumber)?.doubleValue ?? gesture.remoteWidth
        gesture.remoteHeight = (state["height"] as? NSNumber)?.doubleValue ?? gesture.remoteHeight
      }
      status(["state": phase, "error": state["error"] as? String ?? "",
              "shortcutPolicy": shortcutPolicy?.wire ?? "", "shortcutToken": shortcutToken() ?? ""])
      if phase == "connecting" || phase == "connected" { schedulePoll(after: 0.25) }
    } catch {
      connected = false
      status(["state": "failed", "error": error.localizedDescription.isEmpty ? "Control ended" : error.localizedDescription])
    }
  }

  private func cancelTouch() {
    touches.removeAll()
    gesture.cancel()
  }

  private func stop() {
    cancelTouch()
    imeGeneration += 1
    keyboardState.reset()
    resetComposition()
    textSupported = false
    shortcutPolicy = nil
    onKeepAwake?(false)
    pollWork?.cancel()
    pollWork = nil
    if let id { engine.stopControl(id) }
    connected = false
  }

  private func focusLost() {
    cancelTouch()
    imeGeneration += 1
    keyboardState.release()
  }

  /// After focus returns, bring the keyboard back once; never replay a draft.
  private func recoverKeyboard() {
    guard keyboard, connected, isShownInWindow, windowFocused, !isFirstResponder else { return }
    if becomeFirstResponder() { reloadInputViews() }
  }

  @objc private func appResigned() {
    focusLost()
    stopRepeat()
  }

  @objc private func appActivated() {
    recoverKeyboard()
  }

  @objc private func windowFocusChanged() {
    if windowFocused { recoverKeyboard() } else { focusLost(); stopRepeat() }
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window == nil { stop() } else if id != nil && pollWork == nil { schedulePoll(after: 0) }
  }

  // MARK: Keyboard hooks

  override var canBecomeFirstResponder: Bool { id != nil }
  override var showsSoftKeyboard: Bool { keyboard }
  override func acceptsText() -> Bool {
    keyboard && connected && isShownInWindow && windowFocused && isFirstResponder
  }
  override func acceptsHardwareKeys() -> Bool { connected }
  override func commitText(_ text: String) -> Bool { keyboardState.commit(text) }
  override func deleteRemoteBackward() -> Bool { keyboardState.delete(before: 1, after: 0) }
  override func hardwareKey(code: Int?, mask: Int, down: Bool, repeat isRepeat: Bool) -> Bool {
    keyboardState.hardware(code: code, modifierMask: mask, down: down, repeat: isRepeat)
  }
  override func textRejected() {
    status(["state": "connected", "inputError": "Text requires a compatible peer, up to 64 characters, and released physical keys."])
  }
  override func keyboardFocusLost() { focusLost() }

  // MARK: Touch

  private func centroid() -> (x: Double, y: Double) {
    guard !touches.isEmpty else { return (0, 0) }
    let scale = pixelScale
    var x = 0.0, y = 0.0
    for touch in touches {
      let point = touch.location(in: self)
      x += Double(point.x) * scale
      y += Double(point.y) * scale
    }
    return (x / Double(touches.count), y / Double(touches.count))
  }

  override func touchesBegan(_ set: Set<UITouch>, with event: UIEvent?) {
    guard canTouch() else { cancelTouch(); return }
    if !isFirstResponder { _ = becomeFirstResponder() }
    for touch in set {
      if touches.isEmpty {
        touches.append(touch)
        guard let serial = gesture.down(centroid(), eligible: canTouch()) else { touches.removeAll(); return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
          guard let self else { return }
          self.gesture.hold(serial: serial, eligible: self.canTouch())
        }
      } else {
        touches.append(touch)
        gesture.pointerDown(centroid(), eligible: canTouch())
      }
    }
  }

  override func touchesMoved(_ set: Set<UITouch>, with event: UIEvent?) {
    guard canTouch() else { cancelTouch(); return }
    guard !touches.isEmpty else { return }
    gesture.move(centroid(), pointers: touches.count, eligible: canTouch())
  }

  override func touchesEnded(_ set: Set<UITouch>, with event: UIEvent?) {
    guard canTouch() else { cancelTouch(); return }
    guard !touches.isEmpty else { return }
    touches.removeAll { set.contains($0) }
    // A lifted second finger leaves the gesture suppressed until the last one ends.
    if touches.isEmpty { gesture.up(eligible: canTouch()) }
  }

  override func touchesCancelled(_ set: Set<UITouch>, with event: UIEvent?) {
    cancelTouch()
  }

  // MARK: Drawing (the faint 32pt grid of the Android pad)

  override func draw(_ rect: CGRect) {
    guard let context = UIGraphicsGetCurrentContext() else { return }
    context.setStrokeColor(UIColor(white: 0.5, alpha: 36.0 / 255.0).cgColor)
    context.setLineWidth(1 / CGFloat(pixelScale))
    var at: CGFloat = 32
    while at < bounds.width { context.move(to: CGPoint(x: at, y: 0)); context.addLine(to: CGPoint(x: at, y: bounds.height)); at += 32 }
    at = 32
    while at < bounds.height { context.move(to: CGPoint(x: 0, y: at)); context.addLine(to: CGPoint(x: bounds.width, y: at)); at += 32 }
    context.strokePath()
  }
}
