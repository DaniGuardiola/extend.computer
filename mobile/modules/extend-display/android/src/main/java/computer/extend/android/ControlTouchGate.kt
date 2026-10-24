package computer.extend.android

/** A focus boundary retires the complete gesture, including a delayed hold. */
class ControlTouchGate {
  private var active = false
  private var held = false

  fun begin(eligible: Boolean): Boolean {
    active = eligible
    return active
  }
  fun accepts(eligible: Boolean) = active && eligible
  fun hold(eligible: Boolean): Boolean {
    if (!accepts(eligible) || held) return false
    held = true
    return true
  }
  fun releaseHeld(): Boolean {
    val released = held
    held = false
    return released
  }
  fun finish(): Boolean {
    active = false
    return releaseHeld()
  }
}
