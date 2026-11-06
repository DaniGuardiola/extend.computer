import Foundation

/// Port of android/.../DisplayTouch.kt: direct touch on a received display.
struct TouchPoint: Equatable {
  var x: Double
  var y: Double
}

/// Maps view coordinates to the normalized video-view position the engine
/// expects (`point` in displayInputCommand), restricted to the granted content
/// rect. left/top/width/height are the fitted video frame in the input view's
/// coordinates; content* and source* come from the input grant geometry.
struct TouchGeometry: Equatable {
  var left, top, width, height: Double
  var contentX, contentY, contentWidth, contentHeight: Double
  var sourceWidth, sourceHeight: Double

  func map(_ point: TouchPoint) -> TouchPoint? {
    let all = [left, top, width, height, contentX, contentY, contentWidth, contentHeight, sourceWidth, sourceHeight]
    if !all.allSatisfy(\.isFinite) || width <= 0 || height <= 0 || contentWidth <= 0 || contentHeight <= 0 ||
      sourceWidth <= 0 || sourceHeight <= 0 || contentX < 0 || contentY < 0 ||
      contentX + contentWidth > 1.000001 || contentY + contentHeight > 1.000001 { return nil }
    let x = (point.x - left) / width
    let y = (point.y - top) / height
    guard x.isFinite, y.isFinite, x >= contentX, x <= contentX + contentWidth, y >= contentY, y <= contentY + contentHeight else { return nil }
    return TouchPoint(x: x, y: y)
  }

  func scroll(_ delta: TouchPoint) -> TouchPoint {
    TouchPoint(x: delta.x * sourceWidth / (width * contentWidth), y: delta.y * sourceHeight / (height * contentHeight))
  }
}

enum TouchCommand: Equatable {
  case move(TouchPoint)
  case button(TouchPoint, button: Int, down: Bool, clicks: Int)
  case scroll(TouchPoint, dx: Int, dy: Int)
  case release

  var wire: String { DisplayWire.touch(self) }
}

/// Deterministic pointer-ID controller. The view owns scheduling and lease
/// checks. Tap = left click (repeated taps raise the click count up to 3, so a
/// double tap is a double click), hold = left-button drag, two-finger tap =
/// right click, two-finger pan = scroll.
final class DisplayTouch {
  private let slop: Double
  private let doubleSlop: Double
  private let holdTimeout: Int64
  private let doubleTimeout: Int64
  private let emit: (TouchCommand) -> Void

  init(slop: Double, doubleSlop: Double, holdTimeout: Int64, doubleTimeout: Int64, emit: @escaping (TouchCommand) -> Void) {
    self.slop = slop
    self.doubleSlop = doubleSlop
    self.holdTimeout = holdTimeout
    self.doubleTimeout = doubleTimeout
    self.emit = emit
  }

  private var mapping: TouchGeometry?
  /// A changed mapping cancels the gesture (releasing remote state) first.
  var geometry: TouchGeometry? {
    get { mapping }
    set { if mapping != newValue { cancel() }; mapping = newValue }
  }

  private enum Mode { case none, single, pair, suppress }
  private var mode = Mode.none
  private var primary = -1
  private var secondary = -1
  private var origin = TouchPoint(x: 0, y: 0)
  private var secondOrigin = TouchPoint(x: 0, y: 0)
  private var current = TouchPoint(x: 0, y: 0)
  private var target = TouchPoint(x: 0, y: 0)
  private var centroid = TouchPoint(x: 0, y: 0)
  private var residual = TouchPoint(x: 0, y: 0)
  private var began: Int64 = 0
  private var moved = false
  private var dragging = false
  private var previousTap: TouchPoint?
  private var previousTapTime = Int64.min
  private var previousClicks = 0

  private func distance(_ a: TouchPoint, _ b: TouchPoint) -> Double { hypot(a.x - b.x, a.y - b.y) }
  private func mapped(_ point: TouchPoint) -> TouchPoint? { mapping?.map(point) }

  @discardableResult
  func down(id: Int, point: TouchPoint, time: Int64) -> Bool {
    cancelGesture(release: false)
    guard let position = mapped(point) else { return false }
    primary = id; origin = point; current = point; target = position; began = time
    moved = false; dragging = false; mode = .single
    emit(.move(position))
    return true
  }

  func additional(id: Int, points: [Int: TouchPoint], time: Int64) {
    if mode != .single || points.count != 2 || id == primary { cancel(); mode = .suppress; return }
    guard let first = points[primary], let second = points[id], let position = mapped(first) else { return cancel() }
    if mapped(second) == nil { cancel(); mode = .suppress; return }
    if dragging { emit(.button(target, button: 0, down: false, clicks: 1)) }
    dragging = false; previousTap = nil; secondary = id
    origin = first; secondOrigin = second; target = position; began = time
    centroid = TouchPoint(x: (first.x + second.x) / 2, y: (first.y + second.y) / 2)
    residual = TouchPoint(x: 0, y: 0); moved = false; mode = .pair
  }

  func move(points: [Int: TouchPoint]) {
    switch mode {
    case .single:
      guard let point = points[primary], let position = mapped(point) else { return cancel() }
      current = point; target = position
      if distance(point, origin) > slop { moved = true }
      emit(.move(position))
    case .pair:
      if points.count != 2 { cancel(); mode = .suppress; return }
      guard let first = points[primary], let second = points[secondary] else { return cancel() }
      if mapped(first) == nil || mapped(second) == nil { cancel(); mode = .suppress; return }
      let next = TouchPoint(x: (first.x + second.x) / 2, y: (first.y + second.y) / 2)
      if !moved && max(distance(first, origin), distance(second, secondOrigin)) > slop { moved = true }
      if moved, let geometry = mapping {
        let delta = geometry.scroll(TouchPoint(x: next.x - centroid.x, y: next.y - centroid.y))
        residual = TouchPoint(x: min(max(residual.x + delta.x, -4096), 4096), y: min(max(residual.y + delta.y, -4096), 4096))
        let dx = min(max(kotlinRound(residual.x), -4096), 4096)
        let dy = min(max(kotlinRound(residual.y), -4096), 4096)
        residual = TouchPoint(x: residual.x - Double(dx), y: residual.y - Double(dy))
        if dx != 0 || dy != 0 { emit(.scroll(target, dx: dx, dy: dy)) }
        centroid = next
      }
    default:
      break
    }
  }

  func hold(time: Int64) {
    if mode != .single || moved || dragging || time - began < holdTimeout || mapped(current) == nil { return }
    previousTap = nil; dragging = true
    emit(.button(target, button: 0, down: true, clicks: 1))
  }

  func up(id: Int, point: TouchPoint, time: Int64) {
    if mode == .single && id == primary {
      let position = mapped(point)
      if dragging {
        dragging = false
        emit(.button(position ?? target, button: 0, down: false, clicks: 1))
      } else if !moved, let position, distance(point, origin) <= slop, time - began < holdTimeout {
        var clicks = 1
        if let last = previousTap, (0...doubleTimeout).contains(time - previousTapTime), distance(last, point) <= doubleSlop {
          clicks = min(previousClicks + 1, 3)
        }
        emit(.button(position, button: 0, down: true, clicks: clicks))
        emit(.button(position, button: 0, down: false, clicks: clicks))
        previousTap = point; previousTapTime = time; previousClicks = clicks
      } else {
        previousTap = nil
      }
      cancelGesture(release: false)
    } else if mode == .pair && (id == primary || id == secondary) {
      if !moved && mapped(point) != nil && time - began < holdTimeout {
        emit(.button(target, button: 1, down: true, clicks: 1))
        emit(.button(target, button: 1, down: false, clicks: 1))
      }
      mode = .suppress; primary = -1; secondary = -1
    }
  }

  func finish() { cancelGesture(release: false) }
  func cancel() { cancelGesture(release: true); previousTap = nil }

  private func cancelGesture(release: Bool) {
    let wasActive = mode != .none
    let wasDragging = dragging
    mode = .none; dragging = false; primary = -1; secondary = -1; moved = false
    if release && wasActive { emit(.release) } else if wasDragging { emit(.button(target, button: 0, down: false, clicks: 1)) }
  }
}
