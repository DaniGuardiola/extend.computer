# Platforms and adapters

[← Sharing modes](sharing-modes.md) · [Index](../README.md) · [Pairing →](pairing.md)

The shared engine manages sessions across computers and mobile devices. Platform adapters provide the input, display, permission, and credential-storage capabilities available on each operating system.

> [!NOTE]
> macOS is the shipping platform. Windows support is in development: it pairs, mirrors its screen to another device, and accepts keyboard and mouse control from another device. Other platforms are **not supported yet**.

## Shared responsibilities

Identity verification, trust decisions, account membership, encrypted session authorization, event ordering, and connection lifecycle live in the shared engine and runtime. A device name, platform label, or common account cannot substitute for capability checks or permission.

## Platform responsibilities

| Boundary | Platform-dependent work | Current implementation |
| --- | --- | --- |
| Identity storage | Protect device keys using the platform's credential facilities | macOS Keychain; Windows Credential Manager; separate development file identity |
| Input | Capture and inject supported events; map physical keys and modifiers | macOS native helpers; Windows SendInput injection (`native/windows/input/`) |
| Displays | Expose supported capture, virtual-display, and presentation capabilities | Experimental macOS LAN streaming: ScreenCaptureKit, VideoToolbox, Metal, and a runtime virtual-display adapter. Windows mirroring source: DXGI Desktop Duplication and Media Foundation H.264 (`native/windows/display/`) |
| Permissions | Determine effective access and guide users through OS approval | macOS permission bridge |
| Network optimization | Apply supported optimizations and restore resources on exit | macOS Wi-Fi/AWDL broker |
| Distribution | Package, sign, install, and update applications | macOS Developer ID and Sparkle pipeline |

## Source boundaries

`engine/src/platform/mod.rs` selects platform implementations at compile time. macOS Keychain and Wi-Fi broker adapters live in `engine/src/platform/macos/`, and the Windows Credential Manager adapter in `engine/src/platform/windows/`; shared identity and session code call them through stable engine interfaces. Other operating systems currently use explicit fallback adapters, not macOS subprocesses. Unix file permissions remain alongside the shared storage they protect.

Native code and platform-specific tooling use matching directories: `native/macos/`, `native/windows/` and `scripts/macos/`. Keep repository-wide release/version tools in `scripts/` and shared native implementations in `native/shared/`. Add new operating systems in sibling directories, with their own checks and explicit capabilities; do not place Linux or Windows implementations in macOS directories.

CI tests the shared engine on Linux, the shipping desktop on macOS, and the engine, Windows display helper and desktop build on Windows. macOS builds run for shared engine/build changes and macOS implementation changes. Changes confined to future `native/linux/`, `scripts/linux/`, or `engine/src/platform/linux/` directories do not trigger macOS builds; changing the shared platform dispatcher does.

## Capability differences

Support is per mode and role, not simply per platform name. A device might present an image without being able to create a virtual display, or accept control without being able to capture global input.

The protocol negotiates implemented capabilities. Hardware, OS restrictions, and permissions determine the available operations.

---

[← Sharing modes](sharing-modes.md) · [Index](../README.md) · [Pairing →](pairing.md)
