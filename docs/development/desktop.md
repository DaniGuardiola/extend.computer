# Desktop development

[← macOS permissions](../architecture/macos-permissions.md) · [Contents](../README.md) · [Development signing →](signing.md)

The desktop uses Tauri 2, React, Tailwind CSS, Ariakit controls, and native macOS helpers. Production and development have separate application identities and data profiles.

## Local workflow

Install the repository's configured Rust and Node toolchains, plus macOS Command Line Tools. From the repository root:

```sh
npm install
npm install --prefix desktop
npm run desktop:build:dev --prefix desktop
npm run desktop:dev --prefix desktop
```

Quit the existing app and stop input sharing before rebuilding. `desktop:build:dev` builds and signs the development bundle; `desktop:dev` builds and opens it. The bundle lives at `target/development/extend.computer.app`.

These commands explicitly enable the debug-only `dev-identity` feature. The development app uses `~/Library/Application Support/extend.computer Development` and a file identity protected by filesystem permissions. Production uses the `extend.computer` profile and Keychain identities. Production pairings and identities are not copied into development.

Raw `tauri dev` is not the stable signed development-bundle workflow. Release builds reject `dev-identity`. Read the next chapter for certificate setup and permission retention.

## Desktop behavior

Local pairing opens a discoverable window on both devices and verifies ordered symbols. Manual code entry is available as a fallback. Account sign-in uses the account service and browser handoff; account-only peers still require receiver control consent. The device row starts and stops keyboard/mouse sessions and chooses the handoff edge.

Permission guidance runs inside the app through native bridges. Effective helper checks determine readiness. The floating guidance offers the outer application bundle for System Settings; it does not grant permissions itself.

## Checks

See [Contributing](../../CONTRIBUTING.md) for the required checks and toolchain setup. The [code map](../architecture/code-map.md#checks) links checks to implementation areas.

Engine and desktop tests use temporary identities and fake input helpers for protocol and lifecycle verification. They do not prove physical movement, OS permission approval, or update installation. Those flows require live testing in signed app bundles.

## Production builds

Production distribution uses the CI workflow documented in [Releases and updates](../operations/releases.md). Developer ID signing, hardened runtime, notarization, and updater verification belong to that pipeline. A locally signed test build is a separate validation artifact and is not a newly published release.

---

[← macOS permissions](../architecture/macos-permissions.md) · [Contents](../README.md) · [Development signing →](signing.md)
