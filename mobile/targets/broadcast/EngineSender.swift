import Foundation

/// Engine snapshot (`display_sender_state` JSON) the handler needs.
struct EngineSenderState {
  /// Session id from `start`; a different id means another capture replaced ours.
  var id: String
  /// `connecting` → `negotiated` → `streaming` ⇄ `configuring`, then `closed`/`failed`.
  var state: String
  var error: String
  var route: String
  /// Negotiated config; valid once `state == "streaming"`.
  var config: [String: Any]
  /// The receiver lost a frame: the next pushed frame must be a keyframe.
  var keyframe: Bool
  var raw: [String: Any]

  init(_ value: [String: Any]) {
    raw = value
    id = value["id"] as? String ?? ""
    state = value["state"] as? String ?? "idle"
    error = value["error"] as? String ?? ""
    route = value["route"] as? String ?? ""
    config = value["config"] as? [String: Any] ?? [:]
    keyframe = value["keyframe"] as? Bool ?? false
  }
}

/// The outgoing display path of the Rust engine (native/android/engine/src/sender.rs
/// via the `ext_*_display_sender` C functions); see NativeEngineSender.
///
/// Frame contract (same as Android's `pushDisplayFrame`): one complete access
/// unit per call, Annex-B with 4-byte start codes, keyframes prefixed with the
/// parameter sets (VPS/SPS/PPS for HEVC, SPS/PPS for H.264), strictly increasing
/// positive nanosecond timestamps, at most 8 MiB.
protocol EngineSender: AnyObject {
  /// Opens the encrypted display session to `peer` at `address` (LAN
  /// `host:port`, or `usb:<carrier id>` from `usbCarrier`).
  func start(peer: String, address: String, config: SharingConfig) throws
  /// Listens on the USB port (retrying while the app still holds it) so the
  /// desktop's usbmux reconnect attaches a carrier to this engine.
  func listenUsb() throws
  /// Carrier id once the desktop attached over USB for `peer`.
  func usbCarrier(peer: String) -> String?
  func stopUsb()
  /// Renegotiates size/rate (same codec). State goes `configuring` → `streaming`.
  func configure(_ config: SharingConfig) throws
  func push(frame: Data, timestamp: UInt64, keyframe: Bool) throws
  func state() -> EngineSenderState
  func stop()
}

enum BroadcastEngine {
  /// Injection point for tests; the extension uses the linked Rust engine.
  static var make: () throws -> EngineSender = { try NativeEngineSender() }
}
