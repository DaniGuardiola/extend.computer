> 🤖🔧 ai generated

# extend.computer

[extend.computer](https://extend.computer) shares keyboard and mouse control between computers. It starts with macOS to macOS, with a shared Rust engine intended for other platforms. External-display and screen-mirroring modes are planned, not implemented.

## Current capabilities

- Encrypted local pairing, pinned device identities, pairing-authorized control in the desktop app, and revocation.
- Persistent keyboard/mouse sharing across the left or right screen edge, with heartbeat recovery and Control-Option-Escape to stop.
- A first-class CLI with optional reconnect, plus a Tauri GUI that pairs devices and starts/stops sessions directly.
- React, Tailwind CSS 4, Ariakit controls, Lucide icons, and system/light/dark appearance.
- Native macOS permission guidance through PermissionFlow. The GUI reports effective input access; Accessibility may satisfy both input checks without a separate Input Monitoring grant.

Native full-input sharing has been tested across two Macs through the CLI harness. GUI pairing, permission setup, and GUI input sharing have been user-verified. Discovery-first visual pairing awaits a live two-Mac comparison. Desktop accounts support password/2FA login, browser passkeys/security keys, device registration and presence, and secure saved sessions. Signup and account security management open the website. The account directory links already-paired devices; new devices still require local pairing. Automatic account-based pairing and internet connections remain future work. A standalone account API is available in `server/`. Screen extension, visual arrangement, and backup providers remain future work. Licensed under [MIT](LICENSE); vendored dependencies retain their own licenses.

## Development and use

- [Public launch checklist](docs/public-launch-checklist.md): history reset, migration cleanup, distribution, and public CI verification.
- [Desktop guide](docs/desktop-preview.md): GUI setup, building, permissions, and current limits.
- [Persistent control](docs/persistent-control.md): CLI commands, reconnect, and remembered consent.
- CLI executable: `extend-computer`.
- [Prototype guide](docs/prototype.md): diagnostic commands and trust model.
- [Code map](docs/code-map.md): source organization and verification commands.
- [Official website](web/README.md): Cloudflare-hosted landing page, password/passkey accounts, and device management.
- [Account server](server/README.md): self-hostable account and device registry, API, and desktop account integration.

### AWDL setting

Periodic Wi-Fi cursor stalls were isolated to AWDL activity through before/intervention/restored trials. The development helper can temporarily pause AWDL during authorized sessions on eligible routes.

- `--awdl` / `--awdl=true`: leave AWDL unchanged.
- `--no-awdl` / `--awdl=false`: allow session-scoped pauses; current default.

The previous interface state is restored when the lease ends. Contradictory flags are rejected. Configuration-file loading is not implemented. See [session results](docs/session-low-jitter.md) and the [helper guide](native/macos/LowJitter/README.md).

## Research and direction

- [Product direction](docs/product-direction.md): progressive CLI/GUI experience and optional backup providers.
- [Local-first design](docs/local-first-design.md): discovery, consent, trust, and cross-platform architecture.
- [Duet investigation](docs/duet-investigation.md): observations, measurements, and their limits.
- [Verification history](docs/verification.md), [cursor milestone](docs/cursor-prototype.md), and [full-input milestone](docs/input-prototype.md): diagnostic evidence and regression history.

Development GUI builds: `cd desktop && npm run desktop:dev` (or `npm run desktop:build:dev`). These explicitly enable debug-only file identities in a separate extend.computer Development profile, avoiding Keychain prompts across rebuilds. Normal builds retain Keychain; release builds reject `dev-identity`. See [desktop development details](docs/desktop-preview.md#development-identity).
