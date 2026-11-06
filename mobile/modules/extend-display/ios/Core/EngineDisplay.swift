import ExtendEngine
import Foundation

// Display-receive and display-input calls (Android NativeEngine.attachDisplay,
// nextDisplayFrame, displayInputState, ...). Byte results come back as
// ExtBuffers: a NULL ptr with no error means nothing was queued in the
// engine's ~20 ms wait; an error means the session ended.

extension Engine {
  static func attachDisplay(_ id: String) throws { try call(id) { ext_attach_display($0) } }
  static func displayConfigured(_ id: String, _ revision: Int64) throws {
    try call(id) { ext_display_configured($0, revision) }
  }
  static func displayRendered(_ id: String, _ revision: Int64, _ presentationUs: Int64, _ deviceNs: Int64, _ callbackNs: Int64) throws {
    try call(id) { ext_display_rendered($0, revision, presentationUs, deviceNs, callbackNs) }
  }
  static func nextDisplayFrame(_ id: String) throws -> Data? {
    try buffer(id) { ext_next_display_frame($0, $1) }
  }
  static func nextDisplayCursor(_ id: String) throws -> Data? {
    try buffer(id) { ext_next_display_cursor($0, $1) }
  }
  static func displayStateText() throws -> String {
    JSONText.encode(try call { ext_display_state() })
  }
  static func displayInputState(_ id: String) throws -> String {
    JSONText.encode(try call(id) { ext_display_input_state($0) })
  }
  static func setDisplayInputEnabled(_ id: String, _ enabled: Bool) throws {
    try call(id) { ext_set_display_input_enabled($0, enabled) }
  }
  static func displayInputCommand(_ id: String, _ revision: Int64, _ command: String) throws {
    try call(id, command) { ext_display_input_command($0, revision, $1) }
  }
  static func controlStateText() throws -> String {
    JSONText.encode(try call { ext_control_state() })
  }

  private static func buffer(_ id: String,
                             _ body: (UnsafePointer<CChar>, UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> ExtBuffer) throws -> Data? {
    var error: UnsafeMutablePointer<CChar>?
    let result = id.withCString { body($0, &error) }
    if let error {
      let message = String(cString: error)
      ext_string_free(error)
      if result.ptr != nil { ext_buffer_free(result) }
      throw EngineError(message.isEmpty ? "Display session ended" : message)
    }
    guard let ptr = result.ptr else { return nil }
    defer { ext_buffer_free(result) }
    return Data(bytes: ptr, count: Int(result.len))
  }
}
