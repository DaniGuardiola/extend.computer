import Foundation

/// Ordered JSON for the engine's input boundaries. Key order and number
/// formatting follow Android's org.json output so both platforms send the same
/// wire text (`controlEvent`, `displayInputCommand`).
indirect enum InputJSON: Equatable {
  case string(String)
  case int(Int)
  case double(Double)
  case bool(Bool)
  case object([(String, InputJSON)])

  static func == (lhs: InputJSON, rhs: InputJSON) -> Bool { lhs.text == rhs.text }

  var text: String {
    var out = ""
    write(into: &out)
    return out
  }

  private func write(into out: inout String) {
    switch self {
    case .string(let value): InputJSON.quote(value, into: &out)
    case .int(let value): out += String(value)
    case .bool(let value): out += value ? "true" : "false"
    case .double(let value): out += InputJSON.number(value)
    case .object(let fields):
      out += "{"
      for (index, field) in fields.enumerated() {
        if index > 0 { out += "," }
        InputJSON.quote(field.0, into: &out)
        out += ":"
        field.1.write(into: &out)
      }
      out += "}"
    }
  }

  /// org.json writes integral doubles without a fraction (0.0 -> 0, 1.0 -> 1).
  static func number(_ value: Double) -> String {
    precondition(value.isFinite, "JSON numbers must be finite")
    if value == value.rounded(), abs(value) < 1e15 { return String(Int64(value)) }
    return "\(value)"
  }

  static func quote(_ value: String, into out: inout String) {
    out += "\""
    for scalar in value.unicodeScalars {
      switch scalar {
      case "\"": out += "\\\""
      case "\\": out += "\\\\"
      case "\n": out += "\\n"
      case "\r": out += "\\r"
      case "\t": out += "\\t"
      case "\u{08}": out += "\\b"
      case "\u{0C}": out += "\\f"
      default:
        if scalar.value < 0x20 {
          out += String(format: "\\u%04x", scalar.value)
        } else {
          out.unicodeScalars.append(scalar)
        }
      }
    }
    out += "\""
  }
}

extension KeyboardCommand {
  /// The InputEvent object shared by control and display input.
  var json: InputJSON {
    switch self {
    case .key(let code, let down, let isRepeat):
      return .object([("kind", .string("mac_key")), ("code", .int(code)), ("down", .bool(down)), ("repeat", .bool(isRepeat))])
    case .modifiers(let mask):
      return .object([("kind", .string("modifiers")), ("mask", .int(mask))])
    case .text(let value):
      return .object([("kind", .string("text")), ("value", .string(value))])
    }
  }
}

/// Events for the dedicated control session (`controlEvent(id, json)`).
enum ControlWire {
  static func move(x: Double, y: Double) -> String {
    InputJSON.object([("kind", .string("move")), ("x", .double(x)), ("y", .double(y))]).text
  }
  static func button(_ button: Int, down: Bool) -> String {
    InputJSON.object([("kind", .string("button")), ("button", .int(button)), ("down", .bool(down)), ("clicks", .int(1))]).text
  }
  static func scroll(dx: Int, dy: Int) -> String {
    InputJSON.object([("kind", .string("scroll")), ("dx", .int(dx)), ("dy", .int(dy))]).text
  }
  static func keyboard(_ command: KeyboardCommand) -> String { command.json.text }
}

/// Commands for display-scoped input (`displayInputCommand(id, revision, json)`).
enum DisplayWire {
  static let stop = InputJSON.object([("kind", .string("stop"))]).text

  private static func point(_ p: TouchPoint) -> InputJSON {
    .object([("x", .double(p.x)), ("y", .double(p.y))])
  }

  static func touch(_ command: TouchCommand) -> String {
    switch command {
    case .move(let p):
      return InputJSON.object([("kind", .string("move")), ("point", point(p))]).text
    case .button(let p, let button, let down, let clicks):
      return InputJSON.object([("kind", .string("input")), ("point", point(p)), ("event", .object([
        ("kind", .string("button")), ("button", .int(button)), ("down", .bool(down)), ("clicks", .int(clicks)),
      ]))]).text
    case .scroll(let p, let dx, let dy):
      return InputJSON.object([("kind", .string("input")), ("point", point(p)), ("event", .object([
        ("kind", .string("scroll")), ("dx", .int(dx)), ("dy", .int(dy)),
      ]))]).text
    case .release:
      return InputJSON.object([("kind", .string("input")), ("event", .object([("kind", .string("release"))]))]).text
    }
  }

  static func keyboard(_ command: KeyboardCommand) -> String {
    InputJSON.object([("kind", .string("input")), ("event", command.json)]).text
  }
}

/// Kotlin's roundToInt (Math.round): halves round toward positive infinity.
@inline(__always) func kotlinRound(_ value: Double) -> Int {
  Int((value + 0.5).rounded(.down))
}
