package computer.extend.android

fun main() {
  val gate = ControlTouchGate()
  val buttons = mutableListOf<Boolean>()
  fun hold(focused: Boolean) { if (gate.hold(focused)) buttons.add(true) }
  fun cancel() { if (gate.finish()) buttons.add(false) }

  // Losing focus before the 500ms timer retires it without a remote press.
  check(gate.begin(true))
  cancel()
  hold(false)
  hold(true) // An already queued timer cannot press after focus returns.
  check(!gate.accepts(true)) // Resumed MOVE/UP cannot click or move.
  check(buttons.isEmpty())

  // A held contact releases once across window blur, pad blur, CANCEL and stop.
  check(gate.begin(true))
  hold(true)
  hold(true)
  cancel(); cancel(); cancel()
  check(buttons == listOf(true, false))
  check(!gate.accepts(true))
  check(!gate.hold(true))

  // A fresh focused DOWN restores normal drag and exactly one balanced UP.
  check(!gate.begin(false))
  check(!gate.accepts(false))
  check(gate.begin(true))
  check(gate.accepts(true))
  hold(true)
  check(gate.finish())
  buttons.add(false)
  check(!gate.finish())
  check(buttons == listOf(true, false, true, false))

  // Adding a second pointer ends the drag but retains the scroll gesture.
  check(gate.begin(true))
  check(gate.hold(true))
  check(gate.releaseHeld())
  check(!gate.releaseHeld())
  check(gate.accepts(true))
  check(!gate.finish())
  check(!gate.accepts(true))
  println("Control touch focus: pending hold, balanced blur, stale resume, fresh drag and second pointer passed")
}
