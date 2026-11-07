import Foundation
import UIKit

/// Module-facing screen sharing calls (Android: the ScreenSharing* functions in
/// ExtendDisplayModule.kt). Capture itself runs in the broadcast extension;
/// BroadcastController talks to it through the App Group.
enum ScreenSharing {
  /// The computer's screen receiver port (Android sends to the same port).
  static let receiverPort = 48179
  static let activeStates: Set<String> = ["consent", "connecting", "streaming"]

  static var active: Bool { activeStates.contains(BroadcastController.shared.state()["state"] as? String ?? "") }

  /// Blocks on the network (device check); call off the main thread.
  static func start(peer: String, preferences raw: [String: Any]) -> [String: Any] {
    do {
      let preferences = try SharingPreferences(raw)
      try AppFocus.require("Return to the app to start screen sharing")
      let address = try address(peer)
      try DispatchQueue.main.sync {
        try MainActor.assumeIsolated {
          // Fail early on settings the screen can't be captured at (Android
          // validates the encoder config before asking for consent).
          let screen = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.screen }.first
          let size = screen?.nativeBounds.size ?? CGSize(width: 1170, height: 2532)
          _ = try SharingConfig(preferences, screenWidth: Int(size.width), screenHeight: Int(size.height))
          try BroadcastController.shared.start(peer: peer, address: address, preferences: preferences)
        }
      }
      return ["started": true, "error": ""]
    } catch let error as SharingError {
      return ["started": false, "error": error.description]
    } catch {
      return ["started": false, "error": "Couldn't start sharing. Check the other device is available and try again."]
    }
  }

  /// The extension runs its own engine instance. Over USB it takes the USB
  /// port over from the app (`UsbHandoff`) and the desktop reconnects to it;
  /// otherwise it uses the saved LAN address. The account relay isn't reachable
  /// from the extension.
  private static func address(_ peer: String) throws -> String {
    let checked = try Devices.check(peer)
    switch checked["route"] as? String {
    case "usb": return UsbHandoff.address
    case "internet": throw SharingError("Screen sharing needs a local connection")
    default: break
    }
    guard let saved = LocalStore.peer(peer)?["address"] as? String, let colon = saved.range(of: ":", options: .backwards),
          colon.lowerBound > saved.startIndex else {
      throw SharingError("Device address unavailable")
    }
    return "\(saved[..<colon.lowerBound]):\(receiverPort)"
  }

  static func state() -> String { JSONText.encode(BroadcastController.shared.state()) }

  /// `{applied, error, preferences}`; the caller saves `preferences` when applied.
  static func update(peer: String, value: String) async -> [String: Any] {
    let preferences: SharingPreferences
    do { preferences = try SharingPreferences(json: value) } catch {
      return ["applied": false, "error": "\(error)", "preferences": ""]
    }
    return await BroadcastController.shared.update(peer: peer, preferences: preferences)
  }

  static func stop() { BroadcastController.shared.stop() }
}
