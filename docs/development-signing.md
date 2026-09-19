> 🤖🔧 ai generated

# Stable macOS development signing

Use `npm run desktop:build:dev` from `desktop/` (quit extend.computer first). This builds the native helper and a debug app with `dev-identity`, then signs its libraries, helper, and outer bundle using one persistent local certificate. `npm run desktop:dev` also opens the built app. The installed development bundle is `target/development/extend.computer.app`.

Development identifiers are `computer.extend.desktop.development` and `computer.extend.prototype.cursor.development`. Production configuration uses separate identifiers and signing settings. The dev build wrapper accepts no release flags; the existing Rust `dev-identity` guard rejects release profiles. Raw `tauri dev` does not use this signed bundle workflow.

`npm run desktop:signing:setup` performs one-time setup. A private directory at `~/Library/Application Support/extend.computer Development Signing` holds a dedicated signing keychain, its random password (mode0600), and public certificate. Exported private-key files are deleted after import; imported key is non-extractable. The dedicated keychain is temporarily added to the user search list only while signing, then the prior list is restored. Certificate trust is user-scoped and restricted to code signing; no TLS trust or system-wide settings are changed.

Each designated requirement pins the exact certificate fingerprint plus bundle identifier. It does not accept arbitrary code sharing the app's name or bundle identifier. Keep this development certificate/keychain private. Do not regenerate it between builds: changing the certificate means changing the app's identity.

Deploy the signed bundle to other test Macs; do not copy the signing key or keychain password. The public certificate is embedded in each signature. macOS still requires an initial Accessibility/Input Monitoring grant for the new development identity. Stable requirements and actual permission retention were verified locally: both app and helper hashes changed after rebuild, and both permissions remained Allowed after restart. The remote test Mac also retained its Accessibility grant without re-enabling it. No TCC database edits, SIP changes, or permission-check bypasses are used.

Permission-retention check: grant permissions to the new dev app, rebuild and replace it at the same path with the same certificate, reopen and verify the permissions remain allowed. Confirm that app code hashes changed while designated requirements remained identical. The app's `extend.computer Development` data profile and existing paired identities are unchanged.

Run `python3 scripts/test-dev-signing.py` to verify stable requirements across different binaries, rejection of an impostor signature, and rejection of production bundle identifiers.
