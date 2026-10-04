> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

> 🤖🔧 ai generated

# Progressive product experience

extend.computer must work equally well from a terminal or a graphical interface. The CLI is a supported product surface for precise commands, scripting, and automation; the GUI makes the same operations discoverable for people who do not use a terminal. Neither interface owns a separate session engine or permission database.

## Shared foundations

A shared Rust engine owns device identity, discovery, pairing, explicit capabilities, active sessions, reconnect policy, layout, and configuration. Native adapters own operating-system input, display capture, encoding, permission prompts, and presentation. The GUI and CLI call the same operations and read the same versioned configuration and trust records.

Start with terminal commands for individual operations. Add an optional local service when simultaneous GUI/CLI clients need to manage a running session; use authenticated local IPC rather than separate engines opening conflicting event taps. Provide structured command output for scripts alongside concise human-readable status. The future graphical display arrangement edits the same layout settings used by CLI flags.

## Local first, optional backup

No account or cloud connection is required for local discovery, pairing, or control. An optional account-like experience can provide encrypted backup, recovery, and synchronization through user-selected storage providers. Proton Drive is a desired future provider, alongside other cloud or local backup destinations; no provider integration is selected or implemented yet.

Separate portable preferences and layouts from device credentials. Device private keys remain in the platform credential store by default. Restoring a backup must not silently clone a device identity or grant remote control to a new device. Revalidate device identity and require explicit consent for control permissions after recovery. Backup encryption and recovery-key design must be settled before storing sensitive trust material with a provider.

## Immediate sequence

Build usable CLI session lifetimes and explicit remembered-control permissions with trusted reconnect, then a small Mac menu-bar interface, then visual display arrangement. Keep the architecture ready for cross-platform adapters and eventual virtual-display/video mode. Account and cloud-backup UI follow later without changing account-free local operation.
