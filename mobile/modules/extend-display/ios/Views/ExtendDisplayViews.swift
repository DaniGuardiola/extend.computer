import ExpoModulesCore
import UIKit
import os

// Native views behind `NativeDisplay` (ExtendDisplay) and `NativeControl`
// (Control) in ../index.tsx. Same props, events and status maps as Android's
// ExtendDisplayView.kt / ExtendControlView.kt.

/// Screen keep-awake owners (display stream, control session). Immersive mode
/// keeps the screen awake on its own; the idle timer is disabled while either holds.
@MainActor enum KeepAwake {
  private static var owners = Set<ObjectIdentifier>()
  static var active: Bool { !owners.isEmpty }
  static func set(_ owner: AnyObject, _ on: Bool) {
    let key = ObjectIdentifier(owner)
    if on { owners.insert(key) } else { owners.remove(key) }
    UIApplication.shared.isIdleTimerDisabled = active || Immersive.enabled
  }
}

private let displayLog = Logger(subsystem: "computer.extend.ios", category: "display")

func errorMessage(_ error: Error?, _ fallback: String) -> String {
  guard let error else { return fallback }
  let text: String
  if let local = error as? LocalizedError, let description = local.errorDescription { text = description }
  else if let printable = error as? CustomStringConvertible { text = printable.description }
  else { text = error.localizedDescription }
  return text.isEmpty ? fallback : text
}

/// Received display: Metal video + cursor overlay (DisplayRenderer), the
/// engine frame pump (DisplayReceiver) and display-scoped touch/keyboard
/// input (DisplayInputView) above it.
class ExtendDisplayView: ExpoView {
  let onStatus = EventDispatcher()
  let onInputStatus = EventDispatcher()
  let onKeyboardStatus = EventDispatcher()

  private let renderer: DisplayRenderer?
  private let input: DisplayInputView
  private var receiver: DisplayReceiver?
  private var trustedId: String?
  private var generation = 0
  private var streamingRevision: Int64 = -1
  private var stats: Timer?

  required init(appContext: AppContext? = nil) {
    renderer = try? DisplayRenderer.make()
    input = DisplayInputView(engine: DisplayInputView.Engine(
      inputState: { try Engine.displayInputState($0) },
      setInputEnabled: { try Engine.setDisplayInputEnabled($0, $1) },
      inputCommand: { try Engine.displayInputCommand($0, $1, $2) }))
    super.init(appContext: appContext)
    backgroundColor = .black
    clipsToBounds = true
    if let renderer { addSubview(renderer) }
    addSubview(input)
    input.onInputStatus = { [weak self] in self?.onInputStatus($0) }
    input.onKeyboardStatus = { [weak self] in self?.onKeyboardStatus($0) }
  }

  // MARK: Props

  var sessionId: String? {
    get { trustedId }
    set {
      if trustedId == newValue { return }
      stop()
      trustedId = newValue
      input.sessionId = newValue
      connect()
    }
  }
  var touchEnabled: Bool {
    get { input.touchEnabled }
    set { input.touchEnabled = newValue }
  }
  var keyboardEnabled: Bool {
    get { input.keyboardEnabled }
    set { input.keyboardEnabled = newValue }
  }

  func editingShortcut(token: String, action: String) throws -> Bool {
    try input.editingShortcut(token: token, action: action)
  }

  // MARK: Layout

  override func layoutSubviews() {
    super.layoutSubviews()
    renderer?.frame = bounds
    renderer?.layoutIfNeeded()
    input.frame = bounds
    input.videoFrame = renderer?.videoRect ?? .zero
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window == nil { stop() } else { connect() }
  }

  // MARK: Session

  private func status(_ values: [String: Any]) { onStatus(values) }

  private func connect() {
    guard receiver == nil, window != nil, let id = trustedId else { return }
    generation += 1
    let session = generation
    status(["state": "connecting", "message": ""])
    guard let renderer else {
      ended(session, DisplayFailure("Metal isn't available on this device."))
      return
    }
    let config: DisplayStreamConfig
    do {
      let snapshot = Engine.displayState()
      guard snapshot["id"] as? String == id else { throw DisplayFailure("Display session replaced") }
      guard let object = snapshot["config"] as? [String: Any] else { throw DisplayFailure("Invalid display configuration") }
      config = try DisplayStreamConfig(object)
    } catch {
      Engine.cancelDisplay(id)
      ended(session, error)
      return
    }
    let calls = DisplayEngineCalls(
      attach: { try Engine.attachDisplay(id) },
      state: { Data(try Engine.displayStateText().utf8) },
      configured: { try Engine.displayConfigured(id, $0) },
      rendered: { try Engine.displayRendered(id, $0, $1, $2, $3) },
      nextFrame: { try Engine.nextDisplayFrame(id) },
      nextCursor: { try Engine.nextDisplayCursor(id) },
      cancel: { Engine.cancelDisplay(id) })
    let next = DisplayReceiver(id: id, engine: calls, renderer: renderer)
    next.onConfigured = { [weak self] config, revision in
      guard let self, self.generation == session else { return }
      self.streamingRevision = -1
      self.input.streamConfigured(revision: revision, sourceWidth: config.rasterWidth, sourceHeight: config.rasterHeight,
                                  logicalWidth: config.width, logicalHeight: config.height)
      self.setNeedsLayout()
      self.status(["state": "connecting", "width": config.rasterWidth, "height": config.rasterHeight])
    }
    next.onFirstFrame = { [weak self] revision in
      guard let self, self.generation == session, self.streamingRevision != revision else { return }
      self.streamingRevision = revision
      self.input.streamRendered(revision: revision)
      KeepAwake.set(self, true)
      self.status(["state": "streaming"])
    }
    next.onEnded = { [weak self] error in
      guard let self, self.generation == session else { return }
      self.receiver = nil
      self.ended(session, error)
    }
    receiver = next
    next.start(config: config)
    // Diagnostics like Android's frame logs: engine receive/render counters.
    stats?.invalidate()
    stats = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in
      let state = Engine.displayState()
      let keys = ["state", "revision", "received", "rendered", "queued_frames", "cursor_received", "cursor_delivered", "route"]
      let stats = JSONText.encode(state.filter { keys.contains($0.key) })
      let config = JSONText.encode(state["config"] ?? NSNull())
      displayLog.notice("ExtendDisplay stats \(stats, privacy: .public) config \(config, privacy: .public)")
    }
  }

  private func ended(_ session: Int, _ error: Error?) {
    guard session == generation else { return }
    stats?.invalidate(); stats = nil
    if let error { displayLog.error("ExtendDisplay ended: \(errorMessage(error, ""), privacy: .public)") }
    input.streamStopped()
    streamingRevision = -1
    KeepAwake.set(self, false)
    status(["state": "disconnected", "message": errorMessage(error, "Display session ended.")])
  }

  private func stop() {
    generation += 1
    stats?.invalidate(); stats = nil
    receiver?.stop()
    receiver = nil
    input.streamStopped()
    streamingRevision = -1
    renderer?.reset()
    KeepAwake.set(self, false)
  }
}

/// Dedicated control session: the relative trackpad + remote keyboard.
class ExtendControlView: ExpoView {
  let onStatus = EventDispatcher()
  private let pad: ControlInputView

  required init(appContext: AppContext? = nil) {
    pad = ControlInputView(engine: ControlInputView.Engine(
      controlState: { try Engine.controlStateText() },
      controlEvent: { try Engine.controlEvent($0, $1) },
      stopControl: { Engine.stopControl($0) }))
    super.init(appContext: appContext)
    clipsToBounds = true
    addSubview(pad)
    pad.onStatus = { [weak self] in self?.onStatus($0) }
    pad.onKeepAwake = { [weak self] on in
      guard let self else { return }
      KeepAwake.set(self, on)
    }
  }

  var sessionId: String? {
    get { pad.sessionId }
    set { pad.sessionId = newValue }
  }
  var keyboard: Bool {
    get { pad.keyboardEnabled }
    set { pad.keyboardEnabled = newValue }
  }

  func editingShortcut(token: String, action: String) throws -> Bool {
    try pad.editingShortcut(token: token, action: action)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    if pad.frame != bounds { pad.frame = bounds; pad.setNeedsDisplay() }
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window == nil { KeepAwake.set(self, false) }
  }
}
