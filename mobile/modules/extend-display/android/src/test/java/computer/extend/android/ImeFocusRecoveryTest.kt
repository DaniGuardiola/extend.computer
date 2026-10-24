package computer.extend.android

fun main() {
  val recovery = ImeFocusRecovery()
  var generation = 1
  var focused = true
  val events = mutableListOf<KeyboardCommand>()
  val keyboard = DisplayKeyboard({ true }) { events.add(it); true }
  val old = KeyboardCommitSession(keyboard) { generation == 1 && focused }
  check(old.commit("before"))
  focused = false
  generation = 2
  recovery.invalidate(generation)
  check(!old.commit("draft"))
  check(!recovery.claim(generation, focused))
  check(recovery.needed(generation))
  focused = true
  check(recovery.claim(generation, focused))
  check(!recovery.claim(generation, focused)) // Exactly one restart per blur.
  val fresh = KeyboardCommitSession(keyboard) { generation == 2 && focused }
  check(fresh.commit("漢字🙂"))
  check(!old.commit("late"))
  check(events == listOf(KeyboardCommand.Text("before"), KeyboardCommand.Text("漢字🙂")))
  recovery.invalidate(3)
  check(!recovery.claim(2, true)) // A queued old-session refresh cannot consume a new one.
  check(recovery.needed(3))
  check(recovery.claim(3, true)) // Coalesced latest focus transition remains recoverable.
  recovery.invalidate(3)
  recovery.clear() // Framework created a fresh editor, toggle Off, or session stopped.
  check(!recovery.claim(3, true))
  recovery.invalidate(4)
  check(!recovery.claim(4, false)) // Detached/disconnected/unfocused gates retain no permission.
  println("IME recovery: one-shot refresh, stale commits, fresh Unicode, replacement and framework refresh passed")
}
