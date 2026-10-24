package computer.extend.android

fun main() {
  for (policy in KeyboardShortcutPolicy.entries) {
    for (action in EditingShortcut.entries) {
      val events = mutableListOf<KeyboardCommand>()
      val keyboard = DisplayKeyboard { events.add(it); true }
      check(keyboard.shortcut(action, policy))
      check(events == listOf(KeyboardCommand.Modifiers(policy.modifier),
        KeyboardCommand.Key(action.code, true), KeyboardCommand.Key(action.code, false),
        KeyboardCommand.Modifiers(0)))
      check(keyboard.canShortcut())
    }
  }
  val events = mutableListOf<KeyboardCommand>()
  val held = DisplayKeyboard { events.add(it); true }
  check(held.hardware(0, 8, true, false))
  events.clear()
  check(!held.shortcut(EditingShortcut.PASTE, KeyboardShortcutPolicy.COMMAND))
  check(events.isEmpty())
  held.release()
  check(events == listOf(KeyboardCommand.Key(0, false), KeyboardCommand.Modifiers(0)))
  for (failureAt in 1..4) {
    var sent = 0
    val partial = mutableListOf<KeyboardCommand>()
    val keyboard = DisplayKeyboard { partial.add(it); ++sent != failureAt }
    check(!keyboard.shortcut(EditingShortcut.COPY, KeyboardShortcutPolicy.CONTROL))
    check(partial.size == failureAt) // Owner must close the exact session/lease.
    check(keyboard.canShortcut())
  }
  check(KeyboardShortcutPolicy.fromWire("windows") == null)
  check(EditingShortcut.fromWire("cut") == null)
  println("editing shortcut checks passed")
}
