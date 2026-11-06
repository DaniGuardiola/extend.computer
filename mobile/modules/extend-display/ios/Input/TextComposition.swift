import Foundation

/// The local document behind the remote keyboard's UITextInput. It only ever
/// holds marked (IME composition) text: composition stays on the phone, and
/// the final committed string leaves exactly once. Offsets are UTF-16, like
/// UITextInput positions.
struct TextComposition: Equatable {
  enum Outcome: Equatable {
    case none
    /// Final text to send once (DisplayKeyboard.commit).
    case commit(String)
    /// Backspace with no local composition: delete one remote character.
    case deleteBackward
    case rejected
  }

  private(set) var text = ""
  /// The marked range; nil when nothing is being composed.
  private(set) var marked: NSRange?
  private(set) var selection = NSRange(location: 0, length: 0)

  var length: Int { (text as NSString).length }
  var isComposing: Bool { marked != nil }

  mutating func reset() { self = TextComposition() }

  mutating func setMarked(_ value: String?, selected: NSRange) -> Outcome {
    let value = value ?? ""
    if value.isEmpty { reset(); return .none }
    if value.utf16.count > DisplayKeyboard.maxText * 2 { reset(); return .rejected }
    text = value
    marked = NSRange(location: 0, length: length)
    selection = clamp(NSRange(location: selected.location, length: selected.length))
    return .none
  }

  /// The IME accepted its composition as typed.
  mutating func unmark() -> Outcome {
    guard marked != nil else { return .none }
    let value = text
    reset()
    return value.isEmpty ? .none : .commit(value)
  }

  /// insertText replaces any composition with the final string.
  mutating func insert(_ value: String) -> Outcome {
    reset()
    return value.isEmpty ? .none : .commit(value)
  }

  /// replace(_:withText:): edits inside a composition stay local; with no
  /// composition the document is empty, so the replacement is new text.
  mutating func replace(_ range: NSRange, with value: String) -> Outcome {
    guard marked != nil else { return insert(value) }
    let range = clamp(range)
    let next = (text as NSString).replacingCharacters(in: range, with: value)
    if next.isEmpty { reset(); return .none }
    if next.utf16.count > DisplayKeyboard.maxText * 2 { reset(); return .rejected }
    text = next
    marked = NSRange(location: 0, length: length)
    selection = NSRange(location: range.location + (value as NSString).length, length: 0)
    return .none
  }

  mutating func deleteBackward() -> Outcome {
    guard marked != nil, length > 0 else { reset(); return .deleteBackward }
    var range = selection
    if range.length == 0 {
      guard range.location > 0 else { return .none }
      range = (text as NSString).rangeOfComposedCharacterSequence(at: range.location - 1)
    }
    return replace(range, with: "")
  }

  mutating func select(_ range: NSRange) { selection = clamp(range) }

  func clamp(_ range: NSRange) -> NSRange {
    let start = min(max(range.location, 0), length)
    let end = min(max(start + min(max(range.length, 0), length), start), length)
    return NSRange(location: start, length: end - start)
  }
}
