package computer.extend.android

sealed class KeyboardCommand {
  data class Key(val code: Int, val down: Boolean, val repeat: Boolean = false) : KeyboardCommand()
  data class Modifiers(val mask: Int) : KeyboardCommand()
  data class Text(val value: String) : KeyboardCommand()
}

enum class KeyboardShortcutPolicy(val modifier: Int, val wire: String) {
  COMMAND(8, "command"), CONTROL(2, "control");
  companion object {
    fun fromWire(value: String): KeyboardShortcutPolicy? = entries.firstOrNull { it.wire == value }
  }
}

enum class EditingShortcut(val code: Int, val wire: String) {
  SELECT_ALL(0, "select_all"), COPY(8, "copy"), PASTE(9, "paste"), UNDO(6, "undo");
  companion object {
    fun fromWire(value: String): EditingShortcut? = entries.firstOrNull { it.wire == value }
  }
}

/** Physical shortcut positions stay distinct from final layout-independent IME commits. */
class DisplayKeyboard(private val textAllowed: () -> Boolean = { false }, private val emit: (KeyboardCommand) -> Boolean) {
  private val held = mutableSetOf<Int>()
  private var modifiers = 0

  private fun mask(value: Int): Boolean {
    if (modifiers == value) return true
    if (!emit(KeyboardCommand.Modifiers(value))) { reset(); return false }
    modifiers = value
    return true
  }
  fun hardware(code: Int?, modifierMask: Int, down: Boolean, repeat: Boolean): Boolean {
    if (!mask(modifierMask)) return false
    if (code == null || (!down && code !in held)) return true
    if (!emit(KeyboardCommand.Key(code, down, down && repeat && code in held))) { reset(); return false }
    if (down) held.add(code) else held.remove(code)
    return true
  }
  fun canShortcut() = held.isEmpty() && modifiers == 0
  fun shortcut(action: EditingShortcut, policy: KeyboardShortcutPolicy): Boolean {
    // A toolbar action must never inherit or release a user's held keys.
    if (!canShortcut()) return false
    return hardware(action.code, policy.modifier, true, false) &&
      hardware(action.code, policy.modifier, false, false) && mask(0)
  }
  fun commit(value: String): Boolean {
    // Validate the entire commit before injecting any character or modifier.
    if (value.isEmpty()) return true
    if (value.length > MAX_TEXT * 2 || held.isNotEmpty() || modifiers != 0) return false
    if (value == "\n" || value == "\t") {
      val code = if (value == "\n") 36 else 48
      return emit(KeyboardCommand.Key(code, true)) && emit(KeyboardCommand.Key(code, false))
    }
    if (!textAllowed() || !validText(value)) return false
    return emit(KeyboardCommand.Text(value))
  }
  fun delete(before: Int, after: Int): Boolean {
    if (before !in 0..16 || after !in 0..16 || before + after > 16 || held.isNotEmpty() || modifiers != 0) return false
    for (code in List(before) { 51 } + List(after) { 117 }) {
      if (!emit(KeyboardCommand.Key(code, true)) || !emit(KeyboardCommand.Key(code, false))) {
        reset(); return false
      }
    }
    return true
  }
  fun release() {
    val keys = held.toList()
    held.clear()
    for (code in keys) if (!emit(KeyboardCommand.Key(code, false))) break
    if (modifiers != 0) emit(KeyboardCommand.Modifiers(0))
    reset()
  }
  fun reset() { held.clear(); modifiers = 0 }

  companion object {
    // A text commit occupies one bounded message; deletion stays below the 64-message queue.
    const val MAX_TEXT = 64
    fun validText(value: String): Boolean {
      if (value.isEmpty() || value.length > MAX_TEXT * 2 || value.codePointCount(0, value.length) > MAX_TEXT) return false
      var index = 0
      while (index < value.length) {
        val ch = value[index]
        if (Character.isHighSurrogate(ch)) {
          if (index + 1 >= value.length || !Character.isLowSurrogate(value[index + 1])) return false
          index += 2
        } else {
          if (Character.isLowSurrogate(ch) || Character.isISOControl(ch)) return false
          index++
        }
      }
      return value.toByteArray(Charsets.UTF_8).size <= 256
    }
    private val letters = intArrayOf(0,11,8,2,14,3,5,4,34,38,40,37,46,45,31,35,12,15,1,17,32,9,13,7,16,6)
    private val numbers = intArrayOf(29,18,19,20,21,23,22,26,28,25)
    private val functions = intArrayOf(122,120,99,118,96,97,98,100,101,109,103,111)
    fun physical(androidCode: Int): Int? = when (androidCode) {
      in 29..54 -> letters[androidCode - 29]
      in 7..16 -> numbers[androidCode - 7]
      in 131..142 -> functions[androidCode - 131]
      else -> mapOf(66 to 36,67 to 51,112 to 117,61 to 48,62 to 49,111 to 53,
        19 to 126,20 to 125,21 to 123,22 to 124,122 to 115,123 to 119,92 to 116,93 to 121,
        69 to 27,70 to 24,71 to 33,72 to 30,73 to 42,74 to 41,75 to 39,55 to 43,
        56 to 47,76 to 44,68 to 50)[androidCode]
    }
    fun isModifier(androidCode: Int) = androidCode in setOf(59,60,113,114,57,58,117,118)
    fun ascii(character: Char): Pair<Int, Boolean>? {
      if (character in 'a'..'z') return letters[character - 'a'] to false
      if (character in 'A'..'Z') return letters[character - 'A'] to true
      if (character in '0'..'9') return numbers[character - '0'] to false
      val plain = "-=[]\\;',./`"
      val shifted = "_+{}|:\"<>?~"
      val codes = intArrayOf(27,24,33,30,42,41,39,43,47,44,50)
      plain.indexOf(character).takeIf { it >= 0 }?.let { return codes[it] to false }
      shifted.indexOf(character).takeIf { it >= 0 }?.let { return codes[it] to true }
      "!@#$%^&*()".indexOf(character).takeIf { it >= 0 }?.let { return numbers[(it + 1) % 10] to true }
      return when (character) { ' ' -> 49 to false; '\n' -> 36 to false; '\t' -> 48 to false; else -> null }
    }
  }
}

/** A retained IME cannot commit after its owner changes session, lease or visibility. */
class KeyboardCommitSession(private val keyboard: DisplayKeyboard, private val current: () -> Boolean) {
  private var closed = false
  fun close() { closed = true }
  fun commit(value: String): Boolean = !closed && current() && keyboard.commit(value)
}
