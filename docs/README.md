# Technical documentation

Start here to understand how extend.computer connects devices to share input and displays across platforms. The documentation covers the whole product: input sharing, display extension, mirroring, and remote desktop.

> [!NOTE]
> Missing modes and platform adapters are marked **Not supported yet** on their pages. General architecture describes shared responsibilities; implementation details identify the supported platform and mode. A documented capability is not a claim that it already ships.

Detailed proposals and historical experiments live in the [archive](archived/README.md).

## Read in order

| Chapter | What it explains |
| --- | --- |
| 1. [System overview](overview.md) | Components and the complete connection flow |
| 2. [Code map](architecture/code-map.md) | Where each part lives in the repository |
| 3. [Sharing modes](architecture/sharing-modes.md) | Input sharing, extension, mirroring, and remote desktop |
| 4. [Platforms and adapters](architecture/platforms.md) | Device roles, platform boundaries, and support status |
| 5. [Pairing](architecture/pairing.md) | Verification, pinned identities, and removing trust |
| 6. [Accounts and routing](architecture/accounts-and-routing.md) | Account consent, local connections, and internet relay |
| 7. [Input control](architecture/input-control.md) | Capture, handoff, ordering, heartbeats, and recovery |
| 8. [macOS permissions](architecture/macos-permissions.md) | Current macOS permission and optimization adapter |
| 9. [Desktop development](development/desktop.md) | Build profiles, local workflow, and validation |
| 10. [Development signing](development/signing.md) | Stable development identity and permission retention |
| 11. [CLI operation](operations/cli.md) | Terminal control sessions and diagnostic commands |
| 12. [Releases and updates](operations/releases.md) | Current macOS CI, signing, channels, and updates |

Each chapter has previous/next links at the top and bottom. Paths in prose are relative to the repository root unless stated otherwise.

## Component references

- [Account server](../server/README.md): standalone server setup and API reference.
- [Website](../web/README.md): hosted account service and website deployment.
- [Contributing](../CONTRIBUTING.md): contribution checks and release workflow.
- [Desktop changelog](../desktop/CHANGELOG.md): published release history.

## Scope

Devices take roles within a session: supplying input, receiving control, supplying a display image, or presenting that image. Which roles a device can perform depends on its platform adapter, hardware, and OS permissions. Connection availability, cryptographic trust, user consent, capabilities, and transport selection remain separate checks across all modes.

Older documents retain the conclusions and verification limits from their own milestone. Consult the archive for research context, not current setup instructions.

---

[System overview →](overview.md)
