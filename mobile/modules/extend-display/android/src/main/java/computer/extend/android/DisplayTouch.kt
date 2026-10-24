package computer.extend.android

import kotlin.math.hypot
import kotlin.math.roundToInt

internal data class TouchPoint(val x: Double, val y: Double)
internal data class TouchGeometry(
  val left: Double, val top: Double, val width: Double, val height: Double,
  val contentX: Double, val contentY: Double, val contentWidth: Double, val contentHeight: Double,
  val sourceWidth: Double, val sourceHeight: Double,
) {
  fun map(point: TouchPoint): TouchPoint? {
    if (!listOf(left, top, width, height, contentX, contentY, contentWidth, contentHeight, sourceWidth, sourceHeight).all { it.isFinite() }
      || width <= 0 || height <= 0 || contentWidth <= 0 || contentHeight <= 0 || sourceWidth <= 0 || sourceHeight <= 0
      || contentX < 0 || contentY < 0 || contentX + contentWidth > 1.000001 || contentY + contentHeight > 1.000001) return null
    val x = (point.x - left) / width
    val y = (point.y - top) / height
    return if (x.isFinite() && y.isFinite() && x in contentX..(contentX + contentWidth) && y in contentY..(contentY + contentHeight)) TouchPoint(x, y) else null
  }
  fun scroll(delta: TouchPoint) = TouchPoint(delta.x * sourceWidth / (width * contentWidth), delta.y * sourceHeight / (height * contentHeight))
}
internal sealed class TouchCommand {
  data class Move(val point: TouchPoint) : TouchCommand()
  data class Button(val point: TouchPoint, val button: Int, val down: Boolean, val clicks: Int = 1) : TouchCommand()
  data class Scroll(val point: TouchPoint, val dx: Int, val dy: Int) : TouchCommand()
  data object Release : TouchCommand()
}

/** Deterministic pointer-ID controller. View owns scheduling and lease checks. */
internal class DisplayTouch(
  private val slop: Double, private val doubleSlop: Double,
  private val holdTimeout: Long, private val doubleTimeout: Long,
  private val emit: (TouchCommand) -> Unit,
) {
  var geometry: TouchGeometry? = null
    set(value) { if (field != value) cancel(); field = value }
  private enum class Mode { NONE, SINGLE, PAIR, SUPPRESS }
  private var mode = Mode.NONE
  private var primary = -1
  private var secondary = -1
  private var origin = TouchPoint(0.0, 0.0)
  private var secondOrigin = origin
  private var current = origin
  private var target = origin
  private var centroid = origin
  private var residual = origin
  private var began = 0L
  private var moved = false
  private var dragging = false
  private var previousTap: TouchPoint? = null
  private var previousTapTime = Long.MIN_VALUE
  private var previousClicks = 0
  private fun distance(a: TouchPoint, b: TouchPoint) = hypot(a.x - b.x, a.y - b.y)
  private fun mapped(point: TouchPoint) = geometry?.map(point)
  fun down(id: Int, point: TouchPoint, time: Long): Boolean {
    cancelGesture(false)
    val position = mapped(point) ?: return false
    primary = id; origin = point; current = point; target = position; began = time
    moved = false; dragging = false; mode = Mode.SINGLE
    emit(TouchCommand.Move(position))
    return true
  }
  fun additional(id: Int, points: Map<Int, TouchPoint>, time: Long) {
    if (mode != Mode.SINGLE || points.size != 2 || id == primary) { cancel(); mode = Mode.SUPPRESS; return }
    val first = points[primary] ?: return cancel()
    val second = points[id] ?: return cancel()
    val position = mapped(first) ?: return cancel()
    if (mapped(second) == null) { cancel(); mode = Mode.SUPPRESS; return }
    if (dragging) emit(TouchCommand.Button(target, 0, false))
    dragging = false; previousTap = null; secondary = id
    origin = first; secondOrigin = second; target = position; began = time
    centroid = TouchPoint((first.x + second.x) / 2, (first.y + second.y) / 2)
    residual = TouchPoint(0.0, 0.0); moved = false; mode = Mode.PAIR
  }
  fun move(points: Map<Int, TouchPoint>) {
    when (mode) {
      Mode.SINGLE -> {
        val point = points[primary] ?: return cancel()
        val position = mapped(point) ?: return cancel()
        current = point; target = position
        if (distance(point, origin) > slop) moved = true
        emit(TouchCommand.Move(position))
      }
      Mode.PAIR -> {
        if (points.size != 2) { cancel(); mode = Mode.SUPPRESS; return }
        val first = points[primary] ?: return cancel()
        val second = points[secondary] ?: return cancel()
        if (mapped(first) == null || mapped(second) == null) { cancel(); mode = Mode.SUPPRESS; return }
        val next = TouchPoint((first.x + second.x) / 2, (first.y + second.y) / 2)
        if (!moved && maxOf(distance(first, origin), distance(second, secondOrigin)) > slop) moved = true
        if (moved) {
          val delta = geometry!!.scroll(TouchPoint(next.x - centroid.x, next.y - centroid.y))
          residual = TouchPoint((residual.x + delta.x).coerceIn(-4096.0, 4096.0), (residual.y + delta.y).coerceIn(-4096.0, 4096.0))
          val dx = residual.x.roundToInt().coerceIn(-4096, 4096)
          val dy = residual.y.roundToInt().coerceIn(-4096, 4096)
          residual = TouchPoint(residual.x - dx, residual.y - dy)
          if (dx != 0 || dy != 0) emit(TouchCommand.Scroll(target, dx, dy))
          centroid = next
        }
      }
      else -> Unit
    }
  }
  fun hold(time: Long) {
    if (mode != Mode.SINGLE || moved || dragging || time - began < holdTimeout || mapped(current) == null) return
    previousTap = null; dragging = true
    emit(TouchCommand.Button(target, 0, true))
  }
  fun up(id: Int, point: TouchPoint, time: Long) {
    if (mode == Mode.SINGLE && id == primary) {
      val position = mapped(point)
      if (dragging) {
        dragging = false
        emit(TouchCommand.Button(position ?: target, 0, false))
      } else if (!moved && position != null && distance(point, origin) <= slop && time - began < holdTimeout) {
        val last = previousTap
        val clicks = if (last != null && time - previousTapTime in 0..doubleTimeout && distance(last, point) <= doubleSlop)
          (previousClicks + 1).coerceAtMost(3) else 1
        emit(TouchCommand.Button(position, 0, true, clicks)); emit(TouchCommand.Button(position, 0, false, clicks))
        previousTap = point; previousTapTime = time; previousClicks = clicks
      } else previousTap = null
      cancelGesture(false)
    } else if (mode == Mode.PAIR && (id == primary || id == secondary)) {
      if (!moved && mapped(point) != null && time - began < holdTimeout) {
        emit(TouchCommand.Button(target, 1, true)); emit(TouchCommand.Button(target, 1, false))
      }
      mode = Mode.SUPPRESS; primary = -1; secondary = -1
    }
  }
  fun finish() { cancelGesture(false) }
  fun cancel() { cancelGesture(true); previousTap = null }
  private fun cancelGesture(release: Boolean) {
    val wasActive = mode != Mode.NONE
    val wasDragging = dragging
    mode = Mode.NONE; dragging = false; primary = -1; secondary = -1; moved = false
    if (release && wasActive) emit(TouchCommand.Release)
    else if (wasDragging) emit(TouchCommand.Button(target, 0, false))
  }
}
