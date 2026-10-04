> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

> 🤖🔧 ai generated

# extend.computer desktop preview

extend.computer uses Tauri 2, React 19, Tailwind CSS 4, Ariakit dialogs/selects, and Lucide React icons. The compact macOS header retains native window controls. Appearance follows the system by default, with persistent Light and Dark overrides. Log in is a disabled placeholder.

## Run

From `desktop`, run `npm install` and `npm run tauri dev`, with Rust and native build tools on PATH. Build an Apple Silicon app with `npm run tauri build -- --debug --target aarch64-apple-darwin`. The bundle includes the existing `target/extend.computer Cursor.app`; prepare that native helper before packaging.

## GUI workflow

Open Pair device on both computers. Choose a nearby device, compare all eight symbols in order, then confirm on both. Opening Pair device advertises this device on the pairing-only discovery service. Its two-minute, one-attempt windows renew automatically while the dialog stays open; closing it cancels pending pairing and withdraws discovery without stopping trusted-peer receiving. Manual name/address/code entry remains under Pair manually; Show my code is inside that fallback. Discovery names are untrusted hints until verification completes.

Paired devices come from authenticated trust records. Select Extend controls, choose Left or Right, and connect. Pairing authorizes keyboard and mouse sharing while receiving is enabled. There is no separate control approval dialog; authenticated paired devices still require native permission readiness. Connection progress and disconnect controls appear in the device card. Disconnect and Control-Option-Escape end control; heartbeat recovery remains active. Receiving can be turned off separately.

Permissions opens the relevant macOS settings and checks native helper access. macOS permission approval remains a user action. The GUI invokes the shared Rust engine directly and never requires copying terminal commands.

State lives in the platform application-data directory under the `extend.computer` profile. Device labels, addresses, and edges use devices.json. Identities remain in Keychain; trust uses the existing TrustStore. Old layout drafts are not silently trusted or migrated. Discovery is unverified until encrypted pairing authenticates the peer. Reconnect pins the paired fingerprint.

## Current limits

Screen extension, arrangement editing, automatic reconnect, account/backup providers, and concurrent control sessions are not implemented. Screen extension is visibly unavailable. Changed addresses can be edited in device settings. The packaged GUI targets macOS first; other platforms still need native input implementations. This is an ad-hoc signed development build, without notarization or hardened runtime. Distribution requires a Developer ID signing configuration and a review of native library loading.

## Verification

The frontend production build and native arm64 bundle pass. All 44 core tests pass. Desktop integration tests exercise encrypted loopback pairing, pairing-authorized control without another prompt, native-helper readiness, disconnect, and cancellation. A separate test rejects stale approval answers. These tests use temporary identities and a fake input helper; they do not prove physical input behavior. Native UI inspection verified the compact header and accessible Ariakit appearance selector. Two-Mac GUI pairing succeeded after correcting unhyphenated-code entry. The user subsequently verified GUI input sharing. The newer visual pairing flow has loopback and browser validation; live two-Mac comparison is still pending.

## macOS permission guidance

Permission buttons use [PermissionFlow](https://github.com/jaywcjlove/PermissionFlow), vendored at revision `2f2a4b76b1eb2ff7ab815b977be8229853f10bf8` under its MIT license. The two upstream modules remain unchanged. `scripts/macos/build-permission-flow.sh` compiles three native dylibs (SystemSettingsKit, PermissionFlow, and the app’s small Swift bridge), signs development binaries ad hoc, and packages localization resources and the license. This avoids the installed SwiftPM manifest/runtime mismatch. Swift 6 and macOS native build tools are required.

Tauri calls the bridge on its main thread. The controller lives in the existing extend.computer process and offers the outer extend.computer.app for dragging into System Settings. It does not request extra permissions to track the Settings window. The existing input helper remains the authority for readiness; the dialog refreshes status while open and when focused, showing an explicit Allowed label without a manual recheck button. The Swift bridge polls the selected permission with public preflight APIs and closes guidance after a grant. Polling stops when its panel is closed, a grant is detected, or five minutes elapse. Guidance does not grant permission or bypass macOS approval.

The floating panel and Settings navigation were verified in the native app; the user confirmed the panel was visible. Pairing input autoformats four-character groups, accepts mixed case and separators, and supports editing across separators. Pairing success is represented by the new device row, without persistent success text.

Packaging validation found that signing only the executable left an invalid app resource seal. Tauri now signs the complete development bundle; `codesign --verify --deep --strict` passes. Existing development permission grants may need refreshing after rebuilding.

To unpair, open a device’s settings and choose Unpair device. Local access is removed immediately. An authenticated, identity-pinned notification attempts to remove the other record; delivery failure needs no retry action. Idle apps check saved peers every 90 seconds; only a pinned, encrypted “not paired” reply removes stale trust. Connection attempts also reconcile this response. Offline devices and network errors remain paired. Receiving must be on for a peer to answer. Only the device receiving the unpair keeps a persistent, muted placeholder in the previous list position until dismissed; a brief in-app notification calls attention to the change. Fresh pairing clears the placeholder. Explicit CLI revocation remains intact.

## Development identity

Run `npm run desktop:dev` or `npm run desktop:build:dev` from `desktop/` to use the explicit `dev-identity` feature. These debug builds show DEV and use `~/Library/Application Support/extend.computer Development`, with a fresh 0600 file identity in a 0700 directory. The key persists across rebuilds without Keychain prompts. Pair development apps once; production identities and pairings are never copied or exported. Development keys are readable by processes running as your user: use them only for development.

Normal builds keep Keychain storage and the `extend.computer` profile. There is no runtime switch or automatic insecure fallback. The feature is disabled by default, and build-time guards reject it outside the debug profile, including release builds with debug assertions enabled.


## Wi-Fi optimization permission

Wi-Fi optimization appears alongside Accessibility in Permissions. Allow registers the bundled helper through app-owned native macOS service approval. Pending approval opens Login Items & Extensions in System Settings. No shell or Python installer is used. See [native setup and signing](../architecture/macos-permissions.md).

Outgoing GUI control checks the actual peer route before connecting. Both endpoints require a successful optimization lease for eligible Wi-Fi routes before input-session readiness. Ethernet, loopback, and other non-Wi-Fi routes do not require this permission. Receiver listening and pairing remain available without it because no control route exists yet. A local helper failure opens the existing permission dialog for the relevant role; peer-side failure closes the connection and must be repaired on that peer.

Readiness probes contact the authenticated service and require protocol 2; an installed file alone is insufficient. Session brokers renew on their own process, without the previous 20-second replacement on the input path. Broker exit ends required sessions on the next heartbeat. The helper verifies AWDL remains down during renewal; external reactivation fails the lease rather than repeatedly forcing the interface down. Existing disconnect/expiry restoration remains in place.

Dev builds package the same setup action and enforce the same gates. Compatible installed helpers survive ordinary GUI rebuilds. Development and production use separate service identities and the same native registration flow. Production distribution requires Developer ID signing and notarization. Automated tests cover broker failure, route classification, permission-dialog routing, lease restoration, and external reactivation. Real two-Mac UI approval and extended-session verification must accompany release validation.
