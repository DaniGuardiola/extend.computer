package computer.extend.android

/** An invalidated editor needs one refresh; a framework-created editor cancels it. */
class ImeFocusRecovery {
  private var invalidatedGeneration: Int? = null
  fun invalidate(generation: Int) { invalidatedGeneration = generation }
  fun clear() { invalidatedGeneration = null }
  fun needed(generation: Int) = invalidatedGeneration == generation
  fun claim(generation: Int, eligible: Boolean): Boolean {
    if (!eligible || !needed(generation)) return false
    clear()
    return true
  }
}
