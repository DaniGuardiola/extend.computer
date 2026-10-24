package computer.extend.android

fun main() {
  val commands = mutableListOf<KeyboardCommand>()
  var textAllowed = false
  val keyboard = DisplayKeyboard({ textAllowed }) { commands.add(it); true }
  check(keyboard.hardware(DisplayKeyboard.physical(29), 2, true, false))
  check(keyboard.hardware(DisplayKeyboard.physical(29), 2, true, true))
  check(!keyboard.commit("text"))
  check(commands == listOf(KeyboardCommand.Modifiers(2), KeyboardCommand.Key(0, true), KeyboardCommand.Key(0, true, true)))
  keyboard.release()
  check(commands.takeLast(2) == listOf(KeyboardCommand.Key(0, false), KeyboardCommand.Modifiers(0)))
  val released = commands.size
  keyboard.release()
  keyboard.hardware(0, 0, false, false)
  check(commands.size == released) // No duplicate releases or orphan key-ups.

  commands.clear()
  check(!keyboard.commit("ASCII")) // Old peer is never sent the new event.
  check(!keyboard.commit("é🙂"))
  check(commands.isEmpty())
  textAllowed = true
  for (unsupported in listOf("x".repeat(65), "🙂".repeat(65), "\u0000", "\ud800", "\udfff", "A\ud800B")) {
    check(!keyboard.commit(unsupported))
    check(commands.isEmpty()) // Entire commit rejected before any injection.
  }
  for (value in listOf("Aaé", "e\u0301", "🙂", "👩‍💻", "中文", "🙂".repeat(64))) {
    check(keyboard.commit(value))
    check(commands.last() == KeyboardCommand.Text(value))
  }
  check(commands.size == 6) // One bounded event per final commit, no synthesized key bursts.
  commands.clear()
  check(keyboard.commit("\n"))
  check(commands == listOf(KeyboardCommand.Key(36,true), KeyboardCommand.Key(36,false)))
  commands.clear()
  check(keyboard.hardware(0,0,true,false))
  val pending = commands.size
  check(!keyboard.commit("é"))
  check(commands.size == pending)
  keyboard.release()
  for (character in (32..126).map(Int::toChar) + listOf('\n', '\t')) check(DisplayKeyboard.ascii(character) != null)
  for (systemKey in listOf(3,4,24,25,26,164)) {
    check(DisplayKeyboard.physical(systemKey) == null)
    check(!DisplayKeyboard.isModifier(systemKey)) // Home, Back, volume and power stay local.
  }
  commands.clear()
  check(!keyboard.delete(16, 1))
  check(commands.isEmpty())
  check(keyboard.delete(1, 1))
  check(commands == listOf(KeyboardCommand.Key(51, true), KeyboardCommand.Key(51, false),
    KeyboardCommand.Key(117, true), KeyboardCommand.Key(117, false)))
  commands.clear()
  var active = true
  val retained = KeyboardCommitSession(keyboard) { active }
  check(retained.commit("é🙂"))
  val beforeStale = commands.size
  active = false
  check(!retained.commit("late"))
  active = true
  retained.close()
  check(!retained.commit("closed"))
  check(commands.size == beforeStale)
  println("DisplayKeyboard: balanced repeats/releases, atomic Unicode validation/old-peer rejection, bounded commits/deletes and local system keys passed")
}
