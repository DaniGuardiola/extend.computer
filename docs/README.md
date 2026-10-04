# Technical documentation

Start here to understand how extend.computer connects two Macs and shares keyboard and mouse input. These pages describe the current implementation; proposals and historical experiments live in the [archive](archived/README.md).

## Read in order

| Chapter | What it explains |
| --- | --- |
| 1. [System overview](overview.md) | Components and the complete connection flow |
| 2. [Code map](architecture/code-map.md) | Where each part lives in the repository |
| 3. [Pairing and device identity](architecture/pairing.md) | Verification, pinned identities, and removing trust |
| 4. [Accounts and connection routes](architecture/accounts-and-routing.md) | Account consent, local connections, and internet relay |
| 5. [Keyboard and mouse sessions](architecture/input-control.md) | Capture, handoff, ordering, heartbeats, and recovery |
| 6. [macOS permissions and Wi-Fi optimization](architecture/macos-permissions.md) | Permission guidance, native helper approval, and session leases |
| 7. [Desktop development](development/desktop.md) | Build profiles, local workflow, and validation |
| 8. [Development signing](development/signing.md) | Stable development identity and permission retention |
| 9. [CLI operation](operations/cli.md) | Terminal control sessions and diagnostic commands |
| 10. [Releases and updates](operations/releases.md) | CI, signing, notarization, channels, and compatibility |

Each chapter has previous/next links at the top and bottom. Paths in prose are relative to the repository root unless stated otherwise.

## Component references

- [Account server](../server/README.md): standalone server setup and API reference.
- [Website](../web/README.md): hosted account service and website deployment.
- [Contributing](../CONTRIBUTING.md): contribution checks and release workflow.
- [Desktop changelog](../desktop/CHANGELOG.md): published release history.

## Scope

The implemented sharing mode is keyboard and mouse control between Macs. Screen extension, mirroring, and remote desktop display streaming are unavailable. Connection availability, cryptographic trust, user consent, OS permissions, and transport selection are separate checks.

Older documents retain the conclusions and verification limits from their own milestone. Consult the archive for research context, not current setup instructions.

---

[System overview →](overview.md)
