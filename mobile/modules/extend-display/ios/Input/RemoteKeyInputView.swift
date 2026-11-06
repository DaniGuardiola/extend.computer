import UIKit

/// UTF-16 offset position in the composition document.
final class CompositionPosition: UITextPosition {
  let offset: Int
  init(_ offset: Int) { self.offset = offset }
}

final class CompositionRange: UITextRange {
  let range: NSRange
  init(_ range: NSRange) { self.range = range }
  override var isEmpty: Bool { range.length == 0 }
  override var start: UITextPosition { CompositionPosition(range.location) }
  override var end: UITextPosition { CompositionPosition(range.location + range.length) }
}

/// A responder that forwards a remote keyboard: the software keyboard through
/// UITextInput (marked/IME text stays local in `TextComposition`; only the
/// committed string is forwarded, once) and a hardware keyboard through
/// pressesBegan/Ended as positional macOS key codes plus modifier masks.
/// Subclasses decide what is accepted and where it goes.
class RemoteKeyInputView: UIView, UITextInput {
  // MARK: Subclass hooks

  /// Software-keyboard text may be committed now.
  func acceptsText() -> Bool { false }
  /// Hardware keys may be forwarded now.
  func acceptsHardwareKeys() -> Bool { false }
  /// Final text (including "\n" / "\t"). False rejects it.
  func commitText(_ text: String) -> Bool { false }
  /// One backspace with no local composition. False rejects it.
  func deleteRemoteBackward() -> Bool { false }
  /// Mapped physical key (nil code = modifier-only change). False if not forwarded.
  func hardwareKey(code: Int?, mask: Int, down: Bool, repeat isRepeat: Bool) -> Bool { false }
  /// A commit or delete was rejected (text unsupported, too long, keys held).
  func textRejected() {}
  /// First-responder status was lost; held keys must be released.
  func keyboardFocusLost() {}
  /// Show the software keyboard while first responder; otherwise only
  /// hardware keys reach this view.
  var showsSoftKeyboard: Bool { false }

  // MARK: State

  private(set) var composition = TextComposition()
  private var modifiers = ModifierTracker()
  private var repeatTimer: Timer?
  private var repeatCode: Int?
  private let hiddenKeyboard = UIView(frame: .zero)
  static let keyRepeatDelay: TimeInterval = 0.5
  static let keyRepeatInterval: TimeInterval = 0.083

  override init(frame: CGRect) {
    super.init(frame: frame)
    inputAssistantItem.leadingBarButtonGroups = []
    inputAssistantItem.trailingBarButtonGroups = []
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    // The repeat timer retains this view; never let it outlive the window.
    if window == nil { stopRepeat() }
  }

  override var inputView: UIView? { showsSoftKeyboard ? nil : hiddenKeyboard }

  override func resignFirstResponder() -> Bool {
    let resigned = super.resignFirstResponder()
    if resigned { dropKeyboardState() }
    return resigned
  }

  /// Clears local composition, stops repeat and forgets held modifiers, then
  /// tells the subclass to release remote keys.
  func dropKeyboardState() {
    stopRepeat()
    modifiers.reset()
    resetComposition()
    keyboardFocusLost()
  }

  /// Discard any local composition, telling the text system the document changed.
  func resetComposition() {
    guard composition != TextComposition() else { return }
    inputDelegate?.selectionWillChange(self)
    inputDelegate?.textWillChange(self)
    composition.reset()
    inputDelegate?.textDidChange(self)
    inputDelegate?.selectionDidChange(self)
  }

  private func apply(_ outcome: TextComposition.Outcome) {
    switch outcome {
    case .none: break
    case .commit(let text): if !commitText(text) { textRejected() }
    case .deleteBackward: if !deleteRemoteBackward() { textRejected() }
    case .rejected: textRejected()
    }
  }

  private func edit(_ change: (inout TextComposition) -> TextComposition.Outcome) {
    guard acceptsText() else {
      composition.reset()
      return
    }
    apply(change(&composition))
  }

  // MARK: Hardware keys

  override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    let rest = presses.filter { !handle($0, down: true) }
    if !rest.isEmpty { super.pressesBegan(rest, with: event) }
  }

  override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    let rest = presses.filter { !handle($0, down: false) }
    if !rest.isEmpty { super.pressesEnded(rest, with: event) }
  }

  override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    let rest = presses.filter { !handle($0, down: false) }
    if !rest.isEmpty { super.pressesCancelled(rest, with: event) }
  }

  private func handle(_ press: UIPress, down: Bool) -> Bool {
    guard let key = press.key else { return false }
    let usage = key.keyCode.rawValue
    let code = HardwareKeys.macCode(usage: usage)
    let isModifier = HardwareKeys.modifierBit(usage: usage) != nil
    if code == nil && !isModifier { return false }
    guard acceptsHardwareKeys() else {
      if !down { modifiers.update(usage: usage, down: false) }
      return false
    }
    modifiers.update(usage: usage, down: down)
    if !down, let code, code == repeatCode { stopRepeat() }
    let forwarded = hardwareKey(code: code, mask: modifiers.mask, down: down, repeat: false)
    if forwarded, down, let code { startRepeat(code) } else if down && !isModifier { stopRepeat() }
    // Consume mapped keys while forwarding is allowed so the text system never
    // also inserts them.
    return true
  }

  // UIKit does not redeliver held keys; the Mac does not auto-repeat injected
  // key-downs. Repeat the latest held key like a hardware keyboard would.
  private func startRepeat(_ code: Int) {
    stopRepeat()
    repeatCode = code
    let timer = Timer(timeInterval: Self.keyRepeatInterval, target: self, selector: #selector(repeatKey(_:)), userInfo: nil, repeats: true)
    timer.fireDate = Date(timeIntervalSinceNow: Self.keyRepeatDelay)
    RunLoop.main.add(timer, forMode: .common)
    repeatTimer = timer
  }

  @objc private func repeatKey(_ timer: Timer) {
    guard let code = repeatCode, acceptsHardwareKeys(),
          hardwareKey(code: code, mask: modifiers.mask, down: true, repeat: true) else {
      stopRepeat()
      return
    }
  }

  func stopRepeat() {
    repeatTimer?.invalidate()
    repeatTimer = nil
    repeatCode = nil
  }

  // MARK: UITextInputTraits

  var autocapitalizationType: UITextAutocapitalizationType = .none
  var autocorrectionType: UITextAutocorrectionType = .no
  var spellCheckingType: UITextSpellCheckingType = .no
  var smartQuotesType: UITextSmartQuotesType = .no
  var smartDashesType: UITextSmartDashesType = .no
  var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
  var keyboardType: UIKeyboardType = .default
  var keyboardAppearance: UIKeyboardAppearance = .default
  var returnKeyType: UIReturnKeyType = .default
  var enablesReturnKeyAutomatically = false
  @available(iOS 17.0, *)
  var inlinePredictionType: UITextInlinePredictionType {
    get { .no }
    set {}
  }

  // MARK: UIKeyInput

  /// Always true so the software backspace keeps reaching deleteBackward.
  var hasText: Bool { true }

  func insertText(_ text: String) { edit { $0.insert(text) } }

  func deleteBackward() { edit { $0.deleteBackward() } }

  // MARK: UITextInput

  weak var inputDelegate: UITextInputDelegate?
  lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
  var markedTextStyle: [NSAttributedString.Key: Any]?

  private func offset(_ position: UITextPosition) -> Int { (position as? CompositionPosition)?.offset ?? 0 }
  private func nsRange(_ range: UITextRange) -> NSRange {
    (range as? CompositionRange).map { composition.clamp($0.range) } ?? NSRange(location: 0, length: 0)
  }

  func text(in range: UITextRange) -> String? {
    (composition.text as NSString).substring(with: nsRange(range))
  }

  func replace(_ range: UITextRange, withText text: String) {
    let target = nsRange(range)
    edit { $0.replace(target, with: text) }
  }

  var selectedTextRange: UITextRange? {
    get { CompositionRange(composition.selection) }
    set { if let newValue { composition.select(nsRange(newValue)) } }
  }

  var markedTextRange: UITextRange? { composition.marked.map(CompositionRange.init) }

  func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
    edit { $0.setMarked(markedText, selected: selectedRange) }
  }

  func unmarkText() { edit { $0.unmark() } }

  var beginningOfDocument: UITextPosition { CompositionPosition(0) }
  var endOfDocument: UITextPosition { CompositionPosition(composition.length) }

  func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
    let a = offset(fromPosition), b = offset(toPosition)
    return CompositionRange(NSRange(location: min(a, b), length: abs(a - b)))
  }

  func position(from position: UITextPosition, offset delta: Int) -> UITextPosition? {
    let next = offset(position) + delta
    return (0...composition.length).contains(next) ? CompositionPosition(next) : nil
  }

  func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset delta: Int) -> UITextPosition? {
    switch direction {
    case .left, .up: return self.position(from: position, offset: -delta)
    default: return self.position(from: position, offset: delta)
    }
  }

  func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
    let a = offset(position), b = offset(other)
    return a < b ? .orderedAscending : a > b ? .orderedDescending : .orderedSame
  }

  func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int { offset(toPosition) - offset(from) }

  func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
    switch direction {
    case .left, .up: return range.start
    default: return range.end
    }
  }

  func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
    let at = offset(position)
    switch direction {
    case .left, .up: return CompositionRange(NSRange(location: 0, length: at))
    default: return CompositionRange(NSRange(location: at, length: composition.length - at))
    }
  }

  func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection { .natural }
  func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}

  /// Candidate windows anchor near the bottom center of the input surface.
  private var caret: CGRect { CGRect(x: bounds.midX, y: max(bounds.maxY - 24, 0), width: 2, height: 20) }
  func firstRect(for range: UITextRange) -> CGRect { caret }
  func caretRect(for position: UITextPosition) -> CGRect { caret }
  func selectionRects(for range: UITextRange) -> [UITextSelectionRect] { [] }
  func closestPosition(to point: CGPoint) -> UITextPosition? { CompositionPosition(composition.selection.location) }
  func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? { range.start }
  func characterRange(at point: CGPoint) -> UITextRange? { nil }

  // MARK: Window state

  /// Shown in a window and not hidden by itself or an ancestor.
  var isShownInWindow: Bool {
    guard window != nil else { return false }
    var view: UIView? = self
    while let current = view {
      if current.isHidden || current.alpha <= 0.01 { return false }
      view = current.superview
    }
    return true
  }
}
