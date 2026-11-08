# Native Wi-Fi optimization permission

[← Input control](input-control.md) · [Index](../README.md) · [Desktop development →](../development/desktop.md)

> [!NOTE]
> macOS only. Other platform adapters are **not supported yet**.

The app uses Apple's `SMAppService.daemon(plistName:)` API on macOS 13 and later. Registration runs inside the GUI process through the signed Swift permissions bridge without invoking AppleScript, Python, or sudo. macOS owns approval in Login Items & Extensions and associates the background service with extend.computer. Development also supports an explicit administrator-installed signed broker, described below.

## Input permissions and guidance

The native input helper checks effective input-monitoring and event-posting access. Accessibility can satisfy the effective read check without a separate Input Monitoring grant. The desktop refreshes status on startup, focus, and while setup is open.

The PermissionFlow bridge opens the relevant System Settings page and shows a floating guide containing the outer app bundle for dragging into the permissions list. Its observer closes the guide when access is granted. Native approval remains a user action; guidance does not bypass macOS checks.

## Screen Recording

Extend display and Mirror display capture the screen, which needs Screen Recording. Every permission check also asks the display helper (`status`, `capture=`), so the answer comes from the process that captures, whichever app macOS holds responsible for it. Before an Extend or Mirror from this Mac, the app checks it; when it is missing, the `Allow screen sharing` dialog shows only the Screen Recording row instead of starting a session that would fail, and resumes the Extend or Mirror once access is allowed. Settings › Permissions lists the same row.

`Open settings` runs the display helper's `request-permission`, which calls `CGRequestScreenCaptureAccess` (macOS shows its prompt the first time and adds the app to the list), then opens the Screen Recording list in System Settings. A grant taken back later still ends the session with the helper's own message. The persistent screen-capture entitlement is not applied for, so macOS may ask again periodically on recent versions.

## Session leases

Both endpoints inspect the actual routed interface. Eligible Wi-Fi control sessions require an authenticated optimization lease before native input is ready. Ethernet and loopback relay bridges skip the lease. Missing or failed required leases end control rather than silently accepting unoptimized Wi-Fi.

## Receiving without Wi-Fi optimization

Receiving needs Wi-Fi optimization only for connections that arrive over Wi-Fi. Permission checks also note whether this Mac reaches the network over Ethernet and not Wi-Fi (`wired`): an Ethernet link has a routable IPv4 address and no Wi-Fi link, or link that can't be classified, has one. Then Allow connections works without the broker and the footer no longer reads `USB only`. A Mac that also uses Wi-Fi still needs the broker, as before, since peers may reach it over either link and would find the Wi-Fi one turned down. Each incoming keyboard and mouse session, display session, presence reply and display-control grant also checks the link of its own connection: Wi-Fi, or a link that can't be classified, needs a working broker; Ethernet, other links such as Thunderbolt Bridge or a VPN tunnel, loopback relay bridges and USB cables need only the native permissions. A Mac that joins Wi-Fi while receiving without the broker turns down Wi-Fi connections, and the next permission check pauses receiving with a message that asks for Wi-Fi optimization.

The broker temporarily pauses AWDL, holds a restoration obligation, and renews while its session process remains alive. Disconnect, expiry, and process loss release leases. The prior interface state is restored after the last lease; originally disabled AWDL stays disabled. Externally reactivated AWDL invalidates a lease. AirDrop and some Continuity features can be affected while paused.

The broker API permits only lease and status operations, not arbitrary commands or paths. Its client checks signing identity; the daemon also checks the active console user.

## Wi-Fi setup flow

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
- `scripts/macos/dev-app.py` rewrites the development daemon label and associated bundle identifier, and signs the broker, bridge and app with the existing stable local development certificate. It uses the same registration API as production.
- The broker and daemon mutually require the broker signing identity. Development pins the certificate; production pins the Apple team plus broker identifier. The daemon additionally restricts clients to the active console user.
- Each profile has a distinct fixed root-owned recovery journal under `/private/var/db`. Lease expiry, disconnect restoration, signature checks, and Wi-Fi-only session gates remain in force.

Install production apps in `/Applications` so the daemon remains available before user login, as Apple recommends. Development uses the existing stable workspace app path. Production builds use the [Developer ID signing and notarization pipeline](../operations/releases.md).

## Legacy development installation

The older standalone helper and manual installer remain diagnostic tools. They are no longer packaged or used by the desktop app. App bundles never fall back to the old globally installed helper. Existing legacy installations are not silently removed: use the documented legacy uninstall command after ending any old sessions. That command restores journaled AWDL state before deleting its files.

## Explicit signed development installation

On machines where macOS approves the local certificate but blocks daemon launch
with `Launch Constraint Violation`, `npm --prefix desktop run desktop:wifi:prepare`
prepares a separate administrator installer:

```sh
npm --prefix desktop run desktop:wifi:prepare
sudo /usr/bin/python3 target/development/wifi-admin/install.py
```

Stop input sharing before installation. After rebuilding the broker, uninstall
and prepare/install again. To uninstall:

```sh
sudo /usr/bin/python3 target/development/wifi-admin/install.py uninstall
```

It copies the same signed bundled broker to a root-owned location, verifies its
certificate pin and payload hash, and installs a separate `.development.manual`
daemon label with the existing development Mach service. Production identities
cannot use this path. The GUI accepts only the matching root-owned signed broker
and still probes its authenticated service before reporting Allowed. No legacy
ad-hoc helper, global background-item reset, or system certificate trust change is
involved. The installer refuses to replace a running development service.

## Open at login and notifications

**Open at login** (Settings, off by default) writes a launch agent, `~/Library/LaunchAgents/computer.extend.desktop.plist` (`computer.extend.desktop.development.plist` for development), through `tauri-plugin-autostart`. It starts the app with `--autostart`, which skips opening the window. The launch agent runs the executable directly rather than through Launch Services, so `tauri-plugin-single-instance` makes any second launch, such as this one after macOS reopened the app at login, hand its arguments to the running app and quit. macOS reports the new background item and lists it in Login Items & Extensions under Allow in the Background; turning it off there stops the launch even though the setting still shows it on. The launch agent records the app's path, so moving the app needs the setting turned off and on again.

The notice that another device is controlling this Mac or showing a display on it uses macOS user notifications through `tauri-plugin-notification`. The app asks for no permission first; System Settings › Notifications lists extend.computer after its first notice, and turning them off there silences only the notice: the menu bar icon still shows the session.

## Verification

Run `scripts/macos/test-wifi-signing.py` after configuring local dev signing. It verifies matching app/helper acceptance and rejects a wrong identifier, wrong signer, and ad-hoc application. Rust tests cover pending approval, setup failure, and connection permission gates; native lifecycle tests cover restoration and externally reactivated AWDL. Live approval remains a macOS user action. A production certificate build must also be tested in the release environment.

References: [Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice), and the installed macOS SDK's `SMAppService.h` (bundle-relative daemon programs, approval state, and asynchronous unregister completion).

---

[← Input control](input-control.md) · [Index](../README.md) · [Desktop development →](../development/desktop.md)
