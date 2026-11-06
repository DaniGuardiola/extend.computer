import Foundation

/// Port of android/.../DisplayKeyboard.kt. Physical shortcut positions stay
/// distinct from final layout-independent IME commits.
enum KeyboardCommand: Equatable {
  case key(code: Int, down: Bool, repeat: Bool = false)
  case modifiers(mask: Int)
  case text(String)
}

enum KeyboardShortcutPolicy: String {
  case command, control
  var modifier: Int { self == .command ? 8 : 2 }
  var wire: String { rawValue }
  static func fromWire(_ value: String?) -> KeyboardShortcutPolicy? { value.flatMap(KeyboardShortcutPolicy.init(rawValue:)) }
}

enum EditingShortcut: String {
  case selectAll = "select_all", copy, paste, undo
  var code: Int {
    switch self {
    case .selectAll: return 0
    case .copy: return 8
    case .paste: return 9
    case .undo: return 6
    }
  }
  static func fromWire(_ value: String) -> EditingShortcut? { EditingShortcut(rawValue: value) }
}

final class DisplayKeyboard {
  /// A text commit occupies one bounded message; deletion stays below the 64-message queue.
  static let maxText = 64

  private let textAllowed: () -> Bool
  private let emit: (KeyboardCommand) -> Bool
  private var held = Set<Int>()
  private var modifiers = 0

  init(textAllowed: @escaping () -> Bool = { false }, emit: @escaping (KeyboardCommand) -> Bool) {
    self.textAllowed = textAllowed
    self.emit = emit
  }

  var isHolding: Bool { !held.isEmpty || modifiers != 0 }

  private func mask(_ value: Int) -> Bool {
    if modifiers == value { return true }
    if !emit(.modifiers(mask: value)) { reset(); return false }
    modifiers = value
    return true
  }

  @discardableResult
  func hardware(code: Int?, modifierMask: Int, down: Bool, repeat isRepeat: Bool) -> Bool {
    if !mask(modifierMask) { return false }
    guard let code, down || held.contains(code) else { return true }
    if !emit(.key(code: code, down: down, repeat: down && isRepeat && held.contains(code))) { reset(); return false }
    if down { held.insert(code) } else { held.remove(code) }
    return true
  }

  func canShortcut() -> Bool { held.isEmpty && modifiers == 0 }

  func shortcut(_ action: EditingShortcut, policy: KeyboardShortcutPolicy) -> Bool {
    // A toolbar action must never inherit or release a user's held keys.
    guard canShortcut() else { return false }
    return hardware(code: action.code, modifierMask: policy.modifier, down: true, repeat: false) &&
      hardware(code: action.code, modifierMask: policy.modifier, down: false, repeat: false) && mask(0)
  }

  func commit(_ value: String) -> Bool {
    // Validate the entire commit before injecting any character or modifier.
    if value.isEmpty { return true }
    if value.utf16.count > Self.maxText * 2 || !held.isEmpty || modifiers != 0 { return false }
    if value == "\n" || value == "\t" {
      let code = value == "\n" ? 36 : 48
      return emit(.key(code: code, down: true)) && emit(.key(code: code, down: false))
    }
    if !textAllowed() || !Self.validText(value) { return false }
    return emit(.text(value))
  }

  func delete(before: Int, after: Int) -> Bool {
    guard (0...16).contains(before), (0...16).contains(after), before + after <= 16, held.isEmpty, modifiers == 0 else { return false }
    for code in Array(repeating: 51, count: before) + Array(repeating: 117, count: after) {
      if !emit(.key(code: code, down: true)) || !emit(.key(code: code, down: false)) {
        reset(); return false
      }
    }
    return true
  }

  func release() {
    let keys = Array(held)
    held.removeAll()
    for code in keys where !emit(.key(code: code, down: false)) { break }
    if modifiers != 0 { _ = emit(.modifiers(mask: 0)) }
    reset()
  }

  func reset() { held.removeAll(); modifiers = 0 }

  static func validText(_ value: String) -> Bool {
    // Swift strings never hold unpaired surrogates, so the Kotlin surrogate
    // walk reduces to the UTF-16, scalar, control and UTF-8 bounds.
    if value.isEmpty || value.utf16.count > maxText * 2 || value.unicodeScalars.count > maxText { return false }
    for scalar in value.unicodeScalars where scalar.value < 0x20 || (0x7F...0x9F).contains(scalar.value) { return false }
    return value.utf8.count <= 256
  }
}

/// USB HID keyboard usages (UIKeyboardHIDUsage raw values) to macOS virtual key
/// codes, covering exactly the keys Android's DisplayKeyboard.physical maps.
enum HardwareKeys {
  private static let letters = [0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16, 6]
  /// Mac codes for digits 0...9.
  private static let numbers = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25]
  private static let functions = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111]
  private static let others: [Int: Int] = [
    0x28: 36, // return
    0x2A: 51, // backspace
    0x4C: 117, // forward delete
    0x2B: 48, // tab
    0x2C: 49, // space
    0x29: 53, // escape
    0x52: 126, 0x51: 125, 0x50: 123, 0x4F: 124, // up, down, left, right
    0x4A: 115, 0x4D: 119, 0x4B: 116, 0x4E: 121, // home, end, page up, page down
    0x2D: 27, 0x2E: 24, 0x2F: 33, 0x30: 30, 0x31: 42, // - = [ ] backslash
    0x33: 41, 0x34: 39, 0x36: 43, 0x37: 47, 0x38: 44, // ; ' , . /
    0x35: 50, // grave
  ]

  static func macCode(usage: Int) -> Int? {
    switch usage {
    case 0x04...0x1D: return letters[usage - 0x04]
    case 0x1E...0x26: return numbers[usage - 0x1E + 1]
    case 0x27: return numbers[0]
    case 0x3A...0x45: return functions[usage - 0x3A]
    default: return others[usage]
    }
  }

  /// Left/right control, shift, option and command usages (0xE0...0xE7).
  static func modifierBit(usage: Int) -> Int? {
    switch usage {
    case 0xE1, 0xE5: return 1 // shift
    case 0xE0, 0xE4: return 2 // control
    case 0xE2, 0xE6: return 4 // option
    case 0xE3, 0xE7: return 8 // command
    default: return nil
    }
  }
}

/// Held physical modifiers, tracked per key so left+right overlap and missed
/// key-up flags can never leave a stale mask.
struct ModifierTracker {
  private var held = Set<Int>()
  mutating func update(usage: Int, down: Bool) {
    guard HardwareKeys.modifierBit(usage: usage) != nil else { return }
    if down { held.insert(usage) } else { held.remove(usage) }
  }
  var mask: Int { held.reduce(0) { $0 | (HardwareKeys.modifierBit(usage: $1) ?? 0) } }
  mutating func reset() { held.removeAll() }
}
