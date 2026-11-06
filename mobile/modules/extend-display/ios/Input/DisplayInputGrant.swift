import CoreGraphics
import Foundation

/// What the receiver needs from the current decoded stream to validate an
/// input lease. Supplied by the display renderer.
struct DisplayStreamState: Equatable {
  var revision: Int64 = -1
  var frameSeen = false
  var sourceWidth = 0
  var sourceHeight = 0
  var logicalWidth = 0
  var logicalHeight = 0
}

/// Parsed `displayInputState(id)` snapshot, validated against the stream that
/// is on screen (ExtendDisplayView.pollInput on Android).
struct DisplayInputGrant: Equatable {
  var phase: String
  var reason: String
  var textSupported: Bool
  /// Granted for exactly the decoded revision, raster and canvas.
  var valid: Bool
  var revision: Int64
  /// Canonical binding identity; a change revokes keyboard and gesture state.
  var bindingKey: String
  var contentX = 0.0, contentY = 0.0, contentWidth = 0.0, contentHeight = 0.0
  var sourceWidth = 0.0, sourceHeight = 0.0
  var shortcutPolicy: KeyboardShortcutPolicy?

  /// Status state to publish: a stale grant reads as suspended.
  var state: String { phase == "granted" && !valid ? "suspended" : phase }

  static func parse(_ json: String, stream: DisplayStreamState) throws -> DisplayInputGrant {
    guard let data = json.data(using: .utf8),
          let snapshot = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw NSError(domain: "ExtendInput", code: 1, userInfo: [NSLocalizedDescriptionKey: "Invalid input state"])
    }
    func object(_ value: Any?) -> [String: Any]? { value as? [String: Any] }
    func int(_ value: Any?) -> Int64? { (value as? NSNumber).map { $0.int64Value } }
    func double(_ value: Any?) -> Double { (value as? NSNumber)?.doubleValue ?? .nan }

    let phase = snapshot["state"] as? String ?? "suspended"
    let binding = object(snapshot["binding"])
    let revision = int(snapshot["revision"]) ?? -1
    let geometry = object(snapshot["geometry"])
    let raster = object(geometry?["raster"])
    let canvas = object(geometry?["canvas"])
    let valid = phase == "granted" && revision == stream.revision &&
      int(binding?["revision"]) == stream.revision &&
      int(raster?["width"]) == Int64(stream.sourceWidth) && int(raster?["height"]) == Int64(stream.sourceHeight) &&
      int(canvas?["width"]) == Int64(stream.logicalWidth) && int(canvas?["height"]) == Int64(stream.logicalHeight)
    var bindingKey = ""
    if let binding, let encoded = try? JSONSerialization.data(withJSONObject: binding, options: [.sortedKeys]) {
      bindingKey = String(decoding: encoded, as: UTF8.self)
    }
    var grant = DisplayInputGrant(
      phase: phase, reason: snapshot["reason"] as? String ?? "",
      textSupported: snapshot["text_supported"] as? Bool ?? false,
      valid: valid, revision: revision, bindingKey: bindingKey)
    if valid {
      grant.shortcutPolicy = KeyboardShortcutPolicy.fromWire(snapshot["shortcut_policy"] as? String)
      let content = object(geometry?["content"])
      let source = object(geometry?["source_size"])
      grant.contentX = double(content?["x"]); grant.contentY = double(content?["y"])
      grant.contentWidth = double(content?["width"]); grant.contentHeight = double(content?["height"])
      grant.sourceWidth = double(source?["width"]); grant.sourceHeight = double(source?["height"])
    }
    return grant
  }

  /// Touch mapping for the fitted video frame (input-view coordinates).
  func touchGeometry(videoFrame frame: CGRect) -> TouchGeometry? {
    guard valid else { return nil }
    return TouchGeometry(left: Double(frame.minX), top: Double(frame.minY), width: Double(frame.width), height: Double(frame.height),
                         contentX: contentX, contentY: contentY, contentWidth: contentWidth, contentHeight: contentHeight,
                         sourceWidth: sourceWidth, sourceHeight: sourceHeight)
  }
}
