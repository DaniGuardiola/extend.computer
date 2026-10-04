# Code map

[← System overview](../overview.md) · [Contents](../README.md) · [Sharing modes →](sharing-modes.md)

## Shared engine

`src/` contains the Rust library and CLI. `session.rs` owns the encrypted protocol and authorization; `control.rs` forwards ordered input; `cursor.rs` manages the native subprocess. Identity, trust, reconnect, discovery, wire framing, input events, and AWDL leases each have separate modules. `verification.rs` implements visual-pairing commitments and bilateral confirmation; see [pairing](pairing.md) for its protocol and review limits. `error.rs` defines failures that callers classify; diagnostic text is not used as a machine-readable error code. CLI diagnostic modes and `examples/` remain useful for reproducing timing and recovery failures; they are not abandoned GUI scaffolding.

## Desktop

`desktop/src/` contains React UI and the typed Tauri bridge. `usePermissions.ts` owns startup/focus checks, coalesces concurrent requests, polls while permission setup is open, and checks fresh access before connecting. `PermissionsDialog.tsx` presents effective input capabilities; Accessibility can also satisfy read access without a separate Input Monitoring grant. The native guide retains its own close-on-grant observer. `desktop/src-tauri/src/main.rs` handles application startup and IPC. The `runtime` module owns shared job state and snapshots:

- `runtime/incoming.rs`: receiver lifecycle and the native input consent adapter.
- `runtime/unpair.rs`: local access removal, pinned peer notification, and persistent explicit retry.
- `runtime/outgoing.rs`: initiated pairing, code normalization, and pinned control connections.
- `runtime/state.rs`: typed session kinds/phases and allowed transitions, serialized with stable IPC strings.
- `runtime/errors.rs`: user-facing translation of typed engine errors and transport failures.
- `runtime/tests.rs`: encrypted pairing/control lifecycle tests with a fake native input helper.

`approval.rs` owns pending consent, `peers.rs` device metadata, `native.rs` helper permission preflight, and `permission_flow.rs` main-thread access to native permission guidance. Device metadata never grants trust. GUI IPC exposes implemented UI actions; GUI unpair removes local trust and tracks removal metadata; CLI revocation remains a separate persistent block.

`accounts.rs` owns account sessions and browser sign-in; `relay.rs` bridges account WebSockets to the existing encrypted engine. `runtime/presence.rs` checks peer reachability and availability independently of control sessions. See [accounts and routing](accounts-and-routing.md).

## macOS

`native/macos/` holds the Swift input adapter and small C compatibility boundary. `Permissions/` contains the PermissionFlow bridge and MIT-licensed upstream modules. `LowJitter/` contains the app-owned Wi-Fi optimization broker, service registration support, and standalone development tools.

The warp-suppression adjustment, background cursor visibility fallback, session AWDL leases, and blocking accepted sockets each address observed failures. Do not remove them as cosmetic compatibility code without reproducing their original cases. Diagnostic notes live in the milestone documents.

## Account server and website

- [Account server](../../server/README.md): standalone server setup and API.
- [Website](../../web/README.md): hosted account service and deployment.

## Checks

With Rust on PATH, run from the repository root:

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml --target aarch64-apple-darwin
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --target aarch64-apple-darwin --all-targets -- -D warnings
npm --prefix desktop run build
bash scripts/build-permission-flow.sh
```

The desktop integration test uses an isolated ephemeral listening port and does not interrupt a running app. It uses temporary trust stores and identities, and does not capture input. Native GUI permission and edge-handoff behavior still require live testing. Build outputs are ignored. Generated mobile/Windows Store icons were removed because those app targets are not configured; regenerate from `desktop/public/extend-computer.svg` when adding those targets.

To preserve existing development permission grants, source-only cleanup need not replace an already running `.app`. Ad-hoc rebuilds can require macOS approval again; the [stable development signing workflow](../development/signing.md) uses a persistent certificate to retain designated requirements.

---

[← System overview](../overview.md) · [Contents](../README.md) · [Sharing modes →](sharing-modes.md)
