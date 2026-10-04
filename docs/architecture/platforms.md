# Platforms and adapters

[← Sharing modes](sharing-modes.md) · [Index](../README.md) · [Pairing →](pairing.md)

The shared engine manages sessions across computers and mobile devices. Platform adapters provide the input, display, permission, and credential-storage capabilities available on each operating system.

> [!NOTE]
> macOS only. Other platforms **not supported yet**.

## Shared responsibilities

Identity verification, trust decisions, account membership, encrypted session authorization, event ordering, and connection lifecycle live in the shared engine and runtime. A device name, platform label, or common account cannot substitute for capability checks or permission.

## Platform responsibilities

| Boundary | Platform-dependent work | Current implementation |
| --- | --- | --- |
| Identity storage | Protect device keys using the platform's credential facilities | macOS Keychain; separate development file identity |
| Input | Capture and inject supported events; map physical keys and modifiers | macOS native helpers |
| Displays | Expose supported capture, virtual-display, and presentation capabilities | Not supported yet |
| Permissions | Determine effective access and guide users through OS approval | macOS permission bridge |
| Network optimization | Apply supported optimizations and restore resources on exit | macOS Wi-Fi/AWDL broker |
| Distribution | Package, sign, install, and update applications | macOS Developer ID and Sparkle pipeline |

## Capability differences

Support is per mode and role, not simply per platform name. A device might present an image without being able to create a virtual display, or accept control without being able to capture global input.

The protocol negotiates implemented capabilities. Hardware, OS restrictions, and permissions determine the available operations.

---

[← Sharing modes](sharing-modes.md) · [Index](../README.md) · [Pairing →](pairing.md)
