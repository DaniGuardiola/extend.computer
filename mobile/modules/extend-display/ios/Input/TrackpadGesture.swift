import Foundation

/// A focus boundary retires the complete gesture, including a delayed hold.
/// Port of android/.../ControlTouchGate.kt.
final class ControlTouchGate {
  private var active = false
  private var held = false

  func begin(_ eligible: Bool) -> Bool {
    active = eligible
    return active
  }
  func accepts(_ eligible: Bool) -> Bool { active && eligible }
  func hold(_ eligible: Bool) -> Bool {
    if !accepts(eligible) || held { return false }
    held = true
    return true
  }
  func releaseHeld() -> Bool {
    let released = held
    held = false
    return released
  }
  func finish() -> Bool {
    active = false
    return releaseHeld()
  }
}

enum TrackpadCommand: Equatable {
  /// Absolute normalized cursor position.
  case move(x: Double, y: Double)
  case button(Int, down: Bool)
  case scroll(dx: Int, dy: Int)

  var wire: String {
    switch self {
    case .move(let x, let y): return ControlWire.move(x: x, y: y)
    case .button(let button, let down): return ControlWire.button(button, down: down)
    case .scroll(let dx, let dy): return ControlWire.scroll(dx: dx, dy: dy)
    }
  }
}

/// The relative trackpad of ExtendControlView.Pad.onTouchEvent, without UIKit.
/// Points are centroids in device pixels (Android feeds raw pixels; the iOS view
/// scales points by the screen scale), so one phone pixel moves one remote pixel.
/// One finger moves, tap = left click, two-finger tap = right click,
/// two fingers scroll, and a still hold presses the left button for dragging.
final class TrackpadGesture {
  let slop: Double
  var remoteWidth = 1920.0
  var remoteHeight = 1080.0
  private(set) var x = 0.5
  private(set) var y = 0.5
  private var previous = (x: 0.0, y: 0.0)
  private var travel = 0.0
  private var twoFinger = false
  private let gate = ControlTouchGate()
  /// The armed hold, by gesture serial; nil when canceled.
  private var armedHold: Int?
  private var serial = 0
  private let emit: (TrackpadCommand) -> Void

  init(slop: Double, emit: @escaping (TrackpadCommand) -> Void) {
    self.slop = slop
    self.emit = emit
  }

  /// First finger down. Returns the serial to pass to `hold` after the hold
  /// timeout, or nil when the gesture was not admitted.
  func down(_ centroid: (x: Double, y: Double), eligible: Bool) -> Int? {
    cancel()
    guard gate.begin(eligible) else { return nil }
    previous = centroid
    travel = 0
    twoFinger = false
    serial += 1
    armedHold = serial
    return serial
  }

  func pointerDown(_ centroid: (x: Double, y: Double), eligible: Bool) {
    guard gate.accepts(eligible) else { return }
    armedHold = nil
    if gate.releaseHeld() { emit(.button(0, down: false)) }
    twoFinger = true
    previous = centroid
  }

  func move(_ centroid: (x: Double, y: Double), pointers: Int, eligible: Bool) {
    guard gate.accepts(eligible) else { return }
    let dx = centroid.x - previous.x
    let dy = centroid.y - previous.y
    travel += abs(dx) + abs(dy)
    if travel >= slop * 2 { armedHold = nil }
    if pointers >= 2 {
      emit(.scroll(dx: min(max(kotlinRound(dx), -4096), 4096), dy: min(max(kotlinRound(dy), -4096), 4096)))
    } else if !twoFinger {
      x = min(max(x + dx / remoteWidth, 0), 1)
      y = min(max(y + dy / remoteHeight, 0), 1)
      emit(.move(x: x, y: y))
    }
    previous = centroid
  }

  /// Last finger up.
  func up(eligible: Bool) {
    guard gate.accepts(eligible) else { return }
    armedHold = nil
    if gate.finish() {
      emit(.button(0, down: false))
    } else if travel < slop * 2 {
      let button = twoFinger ? 1 : 0
      emit(.button(button, down: true))
      emit(.button(button, down: false))
    }
  }

  func hold(serial: Int, eligible: Bool) {
    guard armedHold == serial else { return }
    armedHold = nil
    if !twoFinger && travel < slop * 2 && gate.hold(eligible) { emit(.button(0, down: true)) }
  }

  /// Focus loss, cancellation or a new gesture: release a held drag.
  func cancel() {
    armedHold = nil
    if gate.finish() { emit(.button(0, down: false)) }
  }
}
