> 🤖🔧 ai generated

# Native Wi-Fi optimization permission

The app uses Apple's `SMAppService.daemon(plistName:)` API on macOS 13 and later. Registration runs inside the GUI process through the signed Swift permissions bridge. It no longer invokes AppleScript, Python, sudo, or a separate installer. macOS owns approval in Login Items & Extensions and associates the background service with extend.computer. This is the modern approval flow, not an imitation of Duet's older Add Helper dialog.

## User flow

1. Choose Allow beside Wi-Fi optimization in Permissions.
2. The app checks the bundled broker's signing identity and registers its LaunchDaemon.
3. If macOS requires approval, the app opens Login Items settings. The permission row becomes Open settings until approval succeeds.
4. The existing permission refresh checks both service setup state and authenticated broker readiness. Only a working broker shows Allowed; pending approval does not bypass connection gates.
5. If an enabled service is unresponsive, Allow unregisters it and waits for the asynchronous unregister completion before registering again. The UI shows Setting up while this happens and reports failure without exposing shell output.

## Bundle and trust

- `Contents/Library/LaunchDaemons/computer.extend.lowjitter.plist` uses `BundleProgram` to locate `Contents/Library/LaunchServices/ExtendComputerLowJitter`.
- Production service: `computer.extend.lowjitter`; broker: `computer.extend.lowjitter.broker`.
- Development service: `computer.extend.lowjitter.development`; broker: `computer.extend.lowjitter.broker.development`.
- Production requires an Apple-anchored signature, the expected broker identifier, and the application's team identifier. Set `APPLE_SIGNING_IDENTITY` to the production signing identity when building; the native helper build uses it too. Ad-hoc builds cannot register through the GUI.
- `scripts/dev-app.py` rewrites the development daemon label and associated bundle identifier, and signs the broker, bridge and app with the existing stable local development certificate. It uses the same registration API as production.
- The broker and daemon mutually require the broker signing identity. Development pins the certificate; production pins the Apple team plus broker identifier. The daemon additionally restricts clients to the active console user.
- Each profile has a distinct fixed root-owned recovery journal under `/private/var/db`. Lease expiry, disconnect restoration, signature checks, and Wi-Fi-only session gates remain in force.

Install production apps in `/Applications` so the daemon remains available before user login, as Apple recommends. Development uses the existing stable workspace app path. Production distribution still needs the normal Developer ID signing and notarization pipeline; this change does not supply a release certificate or claim notarization.

## Legacy development installation

The older standalone helper and manual installer remain diagnostic tools. They are no longer packaged or used by the desktop app. App bundles never fall back to the old globally installed helper. Existing legacy installations are not silently removed: use the documented legacy uninstall command after ending any old sessions. That command restores journaled AWDL state before deleting its files.

## Verification

Run `scripts/test-wifi-signing.py` after configuring local dev signing. It verifies matching app/helper acceptance and rejects a wrong identifier, wrong signer, and ad-hoc application. Rust tests cover pending approval, setup failure, and connection permission gates; native lifecycle tests cover restoration and externally reactivated AWDL. Live approval remains a macOS user action. A production certificate build must also be tested in the release environment.

References: [Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice), and the installed macOS SDK's `SMAppService.h` (bundle-relative daemon programs, approval state, and asynchronous unregister completion).


Local live check (2026-09-18): the signed development app registered successfully, opened macOS Login Items & Extensions, and appeared as `extend.computer.app` under App Background Activity with its switch off. The app changed Allow to Open settings while approval was pending. No AppleScript authorization dialog appeared. Service activation and a real AWDL session remain pending user approval; no production Developer ID certificate was available for a distribution test.
