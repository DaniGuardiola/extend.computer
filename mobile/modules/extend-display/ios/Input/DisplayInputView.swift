import UIKit

/// Display-scoped input for a received display: the input half of Android's
/// ExtendDisplayView (input lease polling, direct touch, remote keyboard,
/// editing shortcuts). The renderer adds this transparent view above its video
/// layer, filling its bounds, and reports the stream through `stream*` calls
/// and `videoFrame`.
final class DisplayInputView: RemoteKeyInputView {
  /// Engine entry points (NativeEngine.displayInputState / setDisplayInputEnabled /
  /// displayInputCommand).
  struct Engine {
    var inputState: (_ id: String) throws -> String
    var setInputEnabled: (_ id: String, _ enabled: Bool) throws -> Void
    var inputCommand: (_ id: String, _ revision: Int64, _ json: String) throws -> Void
  }

  /// state, reason, shortcutPolicy, shortcutToken (Android onInputStatus).
  var onInputStatus: (([String: Any]) -> Void)?
  /// requested, visible, error (Android onKeyboardStatus).
  var onKeyboardStatus: (([String: Any]) -> Void)?

  // Android ViewConfiguration defaults, in points / milliseconds.
  static let touchSlop = 8.0
  static let doubleTapSlop = 100.0
  static let holdTimeout: Int64 = 400
  static let doubleTapTimeout: Int64 = 300

  private let engine: Engine
  private var trustedId: String?
  private var stream = DisplayStreamState()
  private var touch = false
  private var keyboardOn = false
  private var imeVisible = false
  private var imeGeneration = 0
  private var keyboardBinding = ""
  private var keyboardError = ""
  private var textSupported = false
  private var shortcutPolicy: KeyboardShortcutPolicy?
  private var requestedRevision: Int64 = -1
  private var grantRevision: Int64 = -1
  private var grantBinding = ""
  private var grant: DisplayInputGrant?
  private var inputStatus = "disabled"
  private var lastInputStatus = ""
  private var pollWork: DispatchWorkItem?
  private var holdWork: DispatchWorkItem?
  private var pointerIds: [ObjectIdentifier: Int] = [:]
  private var nextPointer = 0
  private var rejectedGesture = false
  private lazy var gestures = DisplayTouch(
    slop: Self.touchSlop, doubleSlop: Self.doubleTapSlop,
    holdTimeout: Self.holdTimeout, doubleTimeout: Self.doubleTapTimeout
  ) { [weak self] command in self?.sendTouch(command) }
  private lazy var keyboard = DisplayKeyboard(textAllowed: { [weak self] in self?.textSupported ?? false }) { [weak self] command in
    self?.sendKeyboard(command) ?? false
  }

  init(engine: Engine) {
    self.engine = engine
    super.init(frame: .zero)
    isMultipleTouchEnabled = true
    isOpaque = false
    backgroundColor = .clear
    isAccessibilityElement = true
    accessibilityLabel = "Remote display input"
    let center = NotificationCenter.default
    center.addObserver(self, selector: #selector(focusChanged), name: UIApplication.willResignActiveNotification, object: nil)
    center.addObserver(self, selector: #selector(focusChanged), name: UIApplication.didBecomeActiveNotification, object: nil)
    center.addObserver(self, selector: #selector(focusChanged), name: UIWindow.didResignKeyNotification, object: nil)
    center.addObserver(self, selector: #selector(focusChanged), name: UIWindow.didBecomeKeyNotification, object: nil)
    center.addObserver(self, selector: #selector(keyboardShown), name: UIResponder.keyboardWillShowNotification, object: nil)
    center.addObserver(self, selector: #selector(keyboardHidden), name: UIResponder.keyboardWillHideNotification, object: nil)
  }

  deinit {
    pollWork?.cancel()
    holdWork?.cancel()
  }

  // MARK: Renderer interface

  /// The trusted display session id (nil = view-only/anonymous stream).
  var sessionId: String? {
    get { trustedId }
    set {
      if trustedId == newValue { return }
      streamStopped()
      trustedId = newValue
      restartPoll()
    }
  }

  /// The fitted video rect in this view's coordinates; refreshes the touch mapping.
  var videoFrame: CGRect = .zero {
    didSet { if videoFrame != oldValue { updateTouchGeometry() } }
  }

  /// A new decoder configuration (Android DisplaySession.Listener.configured):
  /// the old lease no longer matches the picture.
  func streamConfigured(revision: Int64, sourceWidth: Int, sourceHeight: Int, logicalWidth: Int, logicalHeight: Int) {
    suspendInput()
    stream = DisplayStreamState(revision: revision, frameSeen: false, sourceWidth: sourceWidth, sourceHeight: sourceHeight,
                                logicalWidth: logicalWidth, logicalHeight: logicalHeight)
  }

  /// First frame of `revision` is on screen.
  func streamRendered(revision: Int64) {
    if revision == stream.revision { stream.frameSeen = true }
  }

  /// The stream ended, failed or the surface went away.
  func streamStopped() {
    suspendInput()
    stream.frameSeen = false
    stream.revision = -1
  }

  // MARK: Props

  var touchEnabled: Bool {
    get { touch }
    set {
      if touch == newValue { return }
      touch = newValue
      if keyboardOn { cancelGestures() } else { suspendInput() }
      restartPoll()
    }
  }

  var keyboardEnabled: Bool {
    get { keyboardOn }
    set {
      if keyboardOn == newValue { return }
      keyboard.release()
      keyboardOn = newValue
      keyboardError = ""
      hideKeyboard()
      if !newValue && !touch { suspendInput() }
      publishKeyboard()
      restartPoll()
    }
  }

  func editingShortcut(token: String, action: String) throws -> Bool {
    guard let shortcut = EditingShortcut.fromWire(action) else { throw InputError("Unknown shortcut") }
    guard token == shortcutToken(), let policy = shortcutPolicy else { throw InputError("Keyboard input changed. Try again.") }
    guard keyboard.canShortcut() else { throw InputError("Release physical keys before using shortcuts.") }
    if keyboard.shortcut(shortcut, policy: policy) { return true }
    suspendInput()
    return false
  }

  // MARK: Lease

  private var windowFocused: Bool { UIApplication.shared.applicationState == .active && window?.isKeyWindow == true }
  private func canInput() -> Bool {
    stream.frameSeen && stream.revision >= 0 && inputStatus == "granted" && grantRevision == stream.revision &&
      grant != nil && trustedId != nil && isShownInWindow && windowFocused
  }
  private func canTouch() -> Bool { touch && canInput() }
  private func canKeyboard() -> Bool { keyboardOn && canInput() && isFirstResponder }
  private func shortcutToken() -> String? {
    guard keyboardOn, canInput(), shortcutPolicy != nil, let trustedId else { return nil }
    return "\(trustedId):\(grantBinding):\(imeGeneration)"
  }

  private func publishKeyboard() {
    onKeyboardStatus?(["requested": keyboardOn, "visible": imeVisible, "error": keyboardError])
  }

  private func publishInput(_ state: String, _ reason: String = "") {
    let key = "\(state):\(reason):\(shortcutPolicy?.wire ?? "null"):\(shortcutToken() ?? "null")"
    if lastInputStatus == key { return }
    lastInputStatus = key
    onInputStatus?(["state": state, "reason": reason,
                    "shortcutPolicy": shortcutPolicy?.wire ?? "", "shortcutToken": shortcutToken() ?? ""])
  }

  private func hideKeyboard() {
    imeGeneration += 1
    keyboardBinding = ""
    if isFirstResponder { _ = resignFirstResponder() } else { resetComposition() }
  }

  private func showKeyboard() {
    guard keyboardOn, canInput(), keyboardBinding != grantBinding else { return }
    keyboardBinding = grantBinding
    imeGeneration += 1
    if becomeFirstResponder() { reloadInputViews() }
    publishKeyboard()
  }

  private func cancelGestures() {
    holdWork?.cancel()
    holdWork = nil
    gestures.cancel()
  }

  private func suspendInput() {
    keyboard.release()
    hideKeyboard()
    cancelGestures()
    if let id = trustedId {
      try? engine.inputCommand(id, stream.revision, DisplayWire.stop)
      try? engine.setInputEnabled(id, false)
    }
    requestedRevision = -1
    grantRevision = -1
    grantBinding = ""
    grant = nil
    shortcutPolicy = nil
    gestures.geometry = nil
    inputStatus = touch || keyboardOn ? "suspended" : "disabled"
    publishInput(inputStatus)
  }

  private func restartPoll() {
    pollWork?.cancel()
    pollWork = nil
    if window != nil { schedulePoll(after: 0) }
  }

  private func schedulePoll(after delay: TimeInterval) {
    let work = DispatchWorkItem { [weak self] in self?.pollInput() }
    pollWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
  }

  private func pollInput() {
    guard window != nil else { return }
    do {
      if trustedId == nil {
        inputStatus = "unsupported"
        publishInput("unsupported", "View-only display")
      } else if !touch && !keyboardOn {
        inputStatus = "disabled"
        publishInput("disabled")
      } else if let id = trustedId, stream.frameSeen, stream.revision >= 0, isShownInWindow, windowFocused {
        if requestedRevision != stream.revision {
          // Mark before requesting: unsupported/denied input must not turn
          // into a permission request every poll.
          requestedRevision = stream.revision
          try? engine.setInputEnabled(id, true)
        }
        let next = try DisplayInputGrant.parse(try engine.inputState(id), stream: stream)
        textSupported = next.textSupported
        if !next.valid || next.bindingKey != grantBinding {
          cancelGestures()
          // The previous binding was revoked remotely. Never replay its key
          // releases into the replacement lease; Stop releases the old lease.
          keyboard.reset()
          hideKeyboard()
        }
        inputStatus = next.state
        grantRevision = next.valid ? next.revision : -1
        grantBinding = next.valid ? next.bindingKey : ""
        grant = next.valid ? next : nil
        shortcutPolicy = next.valid ? next.shortcutPolicy : nil
        if next.valid { updateTouchGeometry() } else { gestures.geometry = nil }
        if next.valid { showKeyboard() }
        publishInput(inputStatus, next.reason)
      }
    } catch {
      keyboard.reset()
      hideKeyboard()
      cancelGestures()
      grantRevision = -1
      grant = nil
      gestures.geometry = nil
      shortcutPolicy = nil
      inputStatus = "suspended"
      publishInput(inputStatus, error.localizedDescription.isEmpty ? "Touch paused" : error.localizedDescription)
    }
    schedulePoll(after: 0.25)
  }

  private func updateTouchGeometry() {
    guard let grant else { return }
    gestures.geometry = grant.touchGeometry(videoFrame: videoFrame)
  }

  // MARK: Sending

  private func sendTouch(_ command: TouchCommand) {
    guard let id = trustedId else { return }
    if command != .release && !canTouch() { return }
    if command == .release { keyboard.reset() }
    do {
      try engine.inputCommand(id, stream.revision, command.wire)
    } catch {
      inputStatus = "suspended"
      grantRevision = -1
      cancelGestures()
      publishInput("suspended", "Touch paused")
    }
  }

  private func sendKeyboard(_ command: KeyboardCommand) -> Bool {
    guard let id = trustedId, canInput() else { return false }
    do {
      try engine.inputCommand(id, stream.revision, DisplayWire.keyboard(command))
      return true
    } catch {
      keyboard.reset()
      try? engine.inputCommand(id, stream.revision, DisplayWire.stop)
      grantRevision = -1
      inputStatus = "suspended"
      hideKeyboard()
      publishInput("suspended", "Keyboard paused")
      return false
    }
  }

  // MARK: Focus and keyboard visibility

  @objc private func focusChanged(_ note: Notification) {
    // willResignActive arrives while the app still reports .active.
    if note.name == UIApplication.willResignActiveNotification || !windowFocused { suspendInput() }
  }

  @objc private func keyboardShown() { setImeVisible(isFirstResponder && showsSoftKeyboard) }
  @objc private func keyboardHidden() { setImeVisible(false) }
  private func setImeVisible(_ visible: Bool) {
    if imeVisible != visible { imeVisible = visible; publishKeyboard() }
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window == nil {
      pollWork?.cancel()
      pollWork = nil
      suspendInput()
    } else {
      restartPoll()
    }
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    updateTouchGeometry()
  }

  // MARK: Keyboard hooks

  override var canBecomeFirstResponder: Bool { keyboardOn && canInput() }
  override var showsSoftKeyboard: Bool { keyboardOn }
  override func acceptsText() -> Bool { canKeyboard() }
  override func acceptsHardwareKeys() -> Bool { canKeyboard() }
  override func commitText(_ text: String) -> Bool {
    guard keyboard.commit(text) else { return false }
    keyboardError = ""
    publishKeyboard()
    return true
  }
  override func deleteRemoteBackward() -> Bool { keyboard.delete(before: 1, after: 0) }
  override func hardwareKey(code: Int?, mask: Int, down: Bool, repeat isRepeat: Bool) -> Bool {
    keyboard.hardware(code: code, modifierMask: mask, down: down, repeat: isRepeat)
  }
  override func textRejected() {
    keyboardError = "Text requires a compatible peer, up to 64 characters, and released physical keys."
    publishKeyboard()
  }
  override func keyboardFocusLost() {
    keyboard.release()
    imeGeneration += 1
    setImeVisible(false)
  }

  // MARK: Touch

  /// Touches pass through to the views below unless touch input is live.
  override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
    canTouch() && super.point(inside: point, with: event)
  }

  private func pointer(_ touch: UITouch) -> Int {
    let key = ObjectIdentifier(touch)
    if let id = pointerIds[key] { return id }
    nextPointer += 1
    pointerIds[key] = nextPointer
    return nextPointer
  }

  private func touchPoint(_ touch: UITouch) -> TouchPoint {
    let point = touch.location(in: self)
    return TouchPoint(x: Double(point.x), y: Double(point.y))
  }

  private func points() -> [Int: TouchPoint] {
    var result: [Int: TouchPoint] = [:]
    for (key, id) in pointerIds {
      if let touch = activeTouches[key] { result[id] = touchPoint(touch) }
    }
    return result
  }

  private var activeTouches: [ObjectIdentifier: UITouch] = [:]

  private static func millis(_ timestamp: TimeInterval) -> Int64 { Int64((timestamp * 1000).rounded()) }

  private func scheduleHold() {
    holdWork?.cancel()
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.canTouch() else { return }
      self.gestures.hold(time: Self.millis(ProcessInfo.processInfo.systemUptime))
    }
    holdWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + Double(Self.holdTimeout) / 1000, execute: work)
  }

  private func clearTouches() {
    activeTouches.removeAll()
    pointerIds.removeAll()
    rejectedGesture = false
  }

  override func touchesBegan(_ set: Set<UITouch>, with event: UIEvent?) {
    guard canTouch() else { cancelGestures(); clearTouches(); return }
    for touch in set.sorted(by: { $0.timestamp < $1.timestamp }) {
      let id = pointer(touch)
      let wasEmpty = activeTouches.isEmpty
      activeTouches[ObjectIdentifier(touch)] = touch
      if rejectedGesture { continue }
      if wasEmpty {
        if !gestures.down(id: id, point: touchPoint(touch), time: Self.millis(touch.timestamp)) {
          rejectedGesture = true
          continue
        }
        if keyboardOn && !isFirstResponder { _ = becomeFirstResponder() }
        scheduleHold()
      } else {
        holdWork?.cancel()
        gestures.additional(id: id, points: points(), time: Self.millis(touch.timestamp))
      }
    }
  }

  override func touchesMoved(_ set: Set<UITouch>, with event: UIEvent?) {
    guard !rejectedGesture else { return }
    guard canTouch() else { cancelGestures(); return }
    gestures.move(points: points())
  }

  override func touchesEnded(_ set: Set<UITouch>, with event: UIEvent?) {
    let live = !rejectedGesture && canTouch()
    if !rejectedGesture && !live { cancelGestures() }
    for touch in set {
      let key = ObjectIdentifier(touch)
      if live, let id = pointerIds[key] {
        holdWork?.cancel()
        gestures.move(points: points())
        gestures.up(id: id, point: touchPoint(touch), time: Self.millis(touch.timestamp))
      }
      activeTouches.removeValue(forKey: key)
      pointerIds.removeValue(forKey: key)
    }
    if activeTouches.isEmpty {
      if live { gestures.finish() }
      clearTouches()
    }
  }

  override func touchesCancelled(_ set: Set<UITouch>, with event: UIEvent?) {
    cancelGestures()
    for touch in set {
      activeTouches.removeValue(forKey: ObjectIdentifier(touch))
      pointerIds.removeValue(forKey: ObjectIdentifier(touch))
    }
    if activeTouches.isEmpty { clearTouches() }
  }
}
