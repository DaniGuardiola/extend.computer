// Move an external benchmark display without changing the built-in display.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 4,
      let display = UInt32(CommandLine.arguments[1]),
      let x = Int32(CommandLine.arguments[2]), let y = Int32(CommandLine.arguments[3]),
      CGDisplayIsActive(display) != 0, CGDisplayIsBuiltin(display) == 0,
      abs(Int64(x)) <= 32000, abs(Int64(y)) <= 32000 else {
    fputs("Usage: display-position.swift EXTERNAL_DISPLAY_ID X Y\n", stderr)
    exit(2)
}
var configuration: CGDisplayConfigRef?
guard CGBeginDisplayConfiguration(&configuration) == .success,
      let configuration else { exit(1) }
guard CGConfigureDisplayOrigin(configuration, display, x, y) == .success else {
    CGCancelDisplayConfiguration(configuration)
    exit(1)
}
guard CGCompleteDisplayConfiguration(configuration, .forSession) == .success else { exit(1) }
