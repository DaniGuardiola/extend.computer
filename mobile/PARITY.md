# Android parity tracking

The target is all desktop behavior that Android can reasonably support, in both
directions. This table is derived from `desktop/src/bridge.ts`, the desktop
runtime and the current mobile/native implementations. Passing one row does not
establish full parity. No Android OS limitation is accepted here without API and
device evidence.

| Desktop capability | Android state | Remaining verification or work |
| --- | --- | --- |
| Platform-adapted application UI and appearance | React Native, Expo Router, NativeWind, desktop color tokens; persistent appearance | Audit all final flows in tablet/phone layouts and both themes |
| Persistent device identity and trust | Shared Rust identity/trust; Android Keystore wrapping; physical restart/tamper checks passed; verified account membership feeds ephemeral 90-second trust without changing manual pairing; isolated physical account checks preserved identity across a fresh process and removed account peers after server revocation while retaining the manual Mac pair | Active account-session revocation on hardware, increasing-version upgrade and physical backup/restore lifecycle |
| Code pairing | Shared PAKE/Noise client and foreground incoming 2-minute code window; tablet Allow/Deny/Cancel, LAN pinned reconnect and screen-exit cleanup passed; native expiry interrupts idle handshakes; authenticated incoming names survive restart; real macOS UI code pairing and tablet-to-Mac reconnect after restart passed. Outgoing manual UI has a reserved native operation, explicit cancel, screen/background/focus cleanup, bounded socket/expiry handling and a serialized local trust commit. physical Cancel, Back and Home each closed the pending PAKE socket before a deliberately late grant, preserving exact local trust, routes, names, saved peers and availability; a fresh retry paired successfully, and normal UI unpair restored the same baseline | Android-originated desktop UI code pairing; additional expiry and lifecycle cases |
| Nearby discovery and visual pairing | Foreground native NSD discovery and separate easy/manual pages; Android-to-Mac and Mac-to-Android nearby visual pairing passed in the real application UIs on Android 16, with the same eight ordered symbols and numbered labels on both screens, explicit approval on both devices and authenticated paired names; focused verification and peer-confirmation waiting hide discovery controls, checked on the tablet; pairing and Online availability restored afterward | Legacy Android resolution, scoped IPv6 and address repair; physical visual denial, expiry, cancellation and background/retry |
| Local device information and rename | App-specific name editor with explicit Save/Discard and system-name reset; installed version/build and public fingerprint. physical Settings checks passed custom-name Save, persistence across a fresh process, reset remaining unsaved until Save, and restoration of the original system name; fingerprint and existing Mac pair remained visible | Authenticated name propagation to connected peers and account registration after rename, additional screen sizes/themes |
| Paired-device management | List, authenticated connection check and names, pinned nearby-presence route repair for incoming peers, persisted endpoint and confirmed local forget; foreground/focus status refresh with 25-second freshness, distinct Online/Offline/Not accepting connections/Unavailable states and guarded connection actions; saved route host precedes distinct native fallback in bounded pinned status queries. Advanced address editor accepts literal IPv4/IPv6 endpoints with explicit Save/Discard; offline save preserves pairing and name metadata. physical invalid-address rejection, discard, offline save/reopen and authenticated route repair passed. physical same-screen repaired-address refresh, discarded-error cleanup, independent unsaved name draft retention and non-persistence passed with exact original public metadata restored | Scoped IPv6 and account-only address hardware cases; edge/placement, physical remote unpair notification and retry; account peers, other network changes and repeated foreground recovery |
| Availability and receiving controls | Devices Allow connections and Settings Stay available share the user-enabled foreground service with notification Stop action; native reconnect/name listener and desktop-compatible pinned presence; background and app-restart reconnect, repaired receiving port and switch-off cleanup passed on tablet; both real application UIs show the paired peer Online; Mac receiving off changes Android to Not accepting connections and hides connection actions, then receiving on restores Online. physical notification Stop closed control/presence/display listeners and removed the current own NSD registration; both switches stayed off through Home/reopen. Normal re-enable restored listeners, registration, notification and Online status while retaining input preference, USB carrier and exact saved identity/trust/name/route files | Long runs/Doze, permission revocation, active media/input receiving controls, fresh-process and OS-stop recovery |
| Receive extended or mirrored desktop | Shared-engine pinned signaling and encrypted LAN media wired to native AVC/HEVC SurfaceView; 1280×800@30 generated patterns passed 120 actual rendered callbacks per codec; native accessory USB receiver also passed both codecs with 120 rendered frames and verified pattern pixels each; real desktop UI extension also passed at 1920×1200/60-fps requested HEVC mode over native USB, with rendered-frame telemetry and tablet screenshot; normal explicit desktop USB connection and cancellation passed; initial automatic 1920×1200 HEVC mirror produced 4,200 USB callbacks, and fresh normal GUI launch after macOS Screen Recording consent produced 3,840 LAN callbacks with real mirrored pixels inspected. cable-only normal GUI extension passed with tablet Wi-Fi disabled, pinned USB signaling and HEVC 1138×710 actual render callbacks and wallpaper pixels; the physical Mac display remained unmirrored at 1728×1117 | Rotation/background offers, disconnect inventory fix physical recheck and reconnect remain |
| Display configuration and live updates | Native decoder-capability negotiation and initial auto size; pinned live size/frame-rate/bitrate requests hand off decoder before acknowledgement and isolate configuration revisions; physical AVC/HEVC 1280×800 → 960×600 and 8 → 4 Mbps checks passed with 120 rendered frames after each switch. Receiver size/FPS/quality controls negotiate support, retain explicit Apply/Cancel and confirmed output, and require source-helper acknowledgement followed by a matching native rendered frame. passed native USB 1138×710 → 1280×800 and Fit-display restoration, changing FPS and quality in both transitions, with actual source geometry and preserved extended topology; reopened choices, Back and a balanced source tap after reconfiguration passed. Native separate cursor passed physical AVC/HEVC USB shape/hotspot, before-video, paused-video movement and hidden-state pixel checks. API31+ compositor cursor defaults on after exact surface/session/decoded-frame admission, with Canvas fallback and a separate debug override; opt-in tablet checks passed geometry, hotspot/transparency and background cleanup; physically selected the native backend over LAN with debug opt-in and tracing disabled | Receiver requests on other source platforms and transports, live mode/codec changes, rotation-driven auto size, other vendor compositor geometry and profile/lifecycle coverage |
| Send Android display | System MediaProjection consent, foreground capture service, hardware AVC/HEVC Surface encoder and pinned encrypted shared-engine transport; real tablet screen visible in macOS Metal viewer at 1280×800 with 60 fps requested mode and 20 Mbps budget, plus HEVC at 1920×1200 with 30 fps requested mode and 40 Mbps budget; per-device resolution/frame-rate/quality/codec controls, full-screen and selected-app capture, background capture, consent cancellation, abandoned-selector recovery, Android 16 portrait/landscape source resize with both codecs on the same session, live source resolution/frame-rate/quality updates with unsupported hardware rejection preserving capture, and app Stop cleanup verified; native USB tablet-to-Mac capture passed AVC/HEVC at 1280×800/30-fps requested mode, with real macOS Metal renderer, projection cleanup and HEVC viewer inspection. fresh normal Android Mirror and consent flow also rendered HEVC 1280×800 with tablet Wi-Fi disabled, using pinned USB signaling and media; native Metal presented events and own-fixture pixels were inspected. The original notification Stop runner failed its title lookup, while fallback app Stop released projection and restored original sharing settings. A separate fresh USB capture passed the normal notification action: the own-app title and Stop action were present after MIUI group expansion; projection, capture service and notification stopped while availability, USB and sharing preferences remained | Other capture profiles and sustained performance, live codec changes, selected-app visibility/split screen, legacy source resize and incoming rotation, OS Stop and revocation, reconnect; Android display extension feasibility |
| Extend Android with another display | Ordinary-UID probe on the Android 16 tablet created a public, own-content-only virtual display without shell permissions or a projection token; the platform rejected launching a standard exported activity from another UID (`isActivityStartAllowedOnDisplay=false`). Probe display released afterward | General app extension is unavailable through this ordinary app-created display route. Android restricts untrusted display activity embedding; its app-streaming role requires an OEM-preinstalled privileged app. Vendor or privileged integration remains a separate capability, not a shipped feature |
| Bidirectional keyboard/mouse control | Tablet-to-Mac native trackpad/keyboard sender uses pinned encrypted control sessions; physical movement, left/right click, typing, toolbar Tab, held drag and background release passed. Opt-in Android accessibility receiver passed native button click, focused-field typing, scroll, two-second stationary hold, continued drag and held-touch release on disconnect from an encrypted Mac fixture. Opt-in display-scoped native touch gestures, rendered-configuration/grant gating and lifecycle release; pure gesture and native queue checks pass. current build physical native-observer acceptance passed tap, double tap, repeated hold-drag, two-finger right click, scroll and cancel-drag with target/main-display guards. native USB extension at 1138×710 passed six balanced ASCII key pairs, shifted text, Backspace and arrow editing in an actual source text field; turning keyboard off released a held remote key while the Android hold command was still running. Current-density tap, hold-drag, two-finger scroll and cancel-drag passed with native target/main guards. independent control delivered an exact 32-UTF16-unit Unicode commit (combining accents, CJK and ZWJ emoji) into an actual source editor; local composition emitted no remote events, final commit arrived once, finishing composition did not duplicate it, draft-to-finish delivered once, and a retained IME added no source events after keyboard off. native USB display-scoped input passed the same full Unicode, local-composition, commit-once, finish-once and retained-connection absence checks at 1138×710. An encrypted native Mac fixture also delivered exact physical `extend` followed by a 28-UTF16-unit Unicode sample into the tablet editor, with independent native change records and accessibility text | Non-US layout and normal IME-switch/focus recovery; command shortcuts (Android intercepted a Meta+A attempt); remaining current-build gesture variants, background/home drag acceptance (previous topology guard invalid), real desktop sender UI and edge handoff, Android gesture/keyboard coverage and permission lifecycle, other desktop platforms and reconnect |
| Local and Internet routes | Pair/probe uses TCP; pinned LAN video uses the shared encrypted media transport; native accessory bulk media passed on physical tablet without ADB media transport; normal desktop explicit USB discovery/selection and mirror over USB and LAN passed. implements already-paired USB-only pinned presence, control/Unicode and display signaling in both directions, with host checks preserving admission, cancellation and carrier epochs; cable-only extension and reverse mirror physically rendered with tablet Wi-Fi disabled. Private signaling sockets are not saved as LAN routes. includes account Internet presence and bidirectional keyboard/mouse/text relay through pinned Noise sessions, using fresh verified account membership and native device tokens. Isolated real HTTP/WebSocket server integration passed both directions, held-input cancellation and receiving-off gates; Internet-only UI enables control and explains the local-connection requirement for screen sharing | Physical Android/desktop Internet control and route recovery, production account interoperability, physical reverse USB control/Unicode and global sender capture, Intel runtime verification and universal/release USB distribution. Internet display streaming/NAT traversal is not implemented on either desktop or Android |
| Account sign-in and devices | Native browser/PKCE flow with cancellation/expiry; native password/authenticator/recovery-code fallback for API-only servers; Keystore AES-GCM origin-bound no-backup credentials; Android registration/proof, heartbeat/directory refresh, verified ephemeral account peers, removal/logout and scoped session revocation. Actual host HTTP client checks passed state/replay, native MFA, restore, late/unsaved-token cleanup, expiration, removal and bounded HTTP; separate backend HTTP regression with temporary SQLite passed shared identity proof, credential rotation/revocation, wrong-key/replay rejection and cross-account isolation. physical checks against an isolated loopback server passed browser PKCE completion, native identity proof with matching fingerprint, heartbeat/directory updates, fresh-process Keystore restoration, peer-removal cancellation and confirmation, then server revocation and fresh-process signed-out state with manual pairing preserved | Physical password/MFA fallback, self-device removal and stale pending callback; storage/install lifecycle, production server interoperability, account website passkey/security-key coverage and Internet relay |
| Permission management | OS-native per-session capture consent and opt-in input accessibility settings; consent cancellation and capture app Stop verified. Android 16 tablet input service was confirmed bound in the OS and the actual Settings page visibly reported Ready for paired devices, preserving existing accessibility services. fixes invalid neutral teardown dimensions that crashed on service binding; the updated service bound normally with original preferences preserved. Physical recovery without a held contact passed source disconnect, exact same-peer permission denial, original service restoration and fresh native click/Unicode delivery; input/receiving preferences and existing peers were preserved | Notification/network consent, OS Stop and supported Android versions. The strict continued-contact revocation case failed: the disposable target still recorded a held contact after its service was unbound; post-unbind gesture release is not guaranteed through the public API |
| Disconnect/cancellation/reliability | Physical accessory detach/reconnect, original MTP/ADB mode restoration, temporary-pair-only cleanup and receiver controls; foreground availability service survives navigation/background, restores after app reopening, shuts sockets when disabled | Full media/input session state machine, outgoing cancellation, rotation, OS-stop/revocation recovery and long runs |
| Distribution and upgrades | standalone development package contains arm64-v8a and x86_64; actual engine ELF architecture, public JNI export, 16 KiB engine LOAD alignment and exact ABI coverage verified. →20→21 in-place replacements preserved nine measured identity/metadata/preferences files. A tenth target was a directory and does not establish native trust-file byte preservation. Legacy, cloud and device-transfer backup rules allow only appearance preferences; packaged manifest references and compiled rules verified | Production package/signing/versioning and distribution, increasing-version upgrade, clean install and physical backup/restore; x86_64 runtime remains unverified |
| Comparative performance | Earlier USB optical receiver campaign retains its original APK and controlled display geometry; separate three-scene native USB diagnostics used an older build at 1280×800 source geometry, with recorded queue overflows; current cursor/mirror receipts are functional checks only | New optical runs for the current APK and matched geometry, both directions, quality/input/cursor/reliability; preserve invalid evidence as invalid |

Public generic instrumentation and optical tooling live in this repository.
External product adapters, reverse-engineering notes and combined comparison
results remain in their separate private repository.

The UI/pairing/status receipts are retained locally under
`native/android/.local/android-ui-parity-physical-20261010-1`. The backend check is
[`server/tests/android-device-http.rs`](../server/tests/android-device-http.rs).
Separate display-scoped touch receipts are retained under
`native/android/.local/display-input-physical` for current build.
The initial hold-drag capture included extra physical Mac events and is invalid;
the home-drag capture observed native release but failed its topology guard and
is invalid for full acceptance. These receipts do not establish current-build
touch, cold-decoder recovery, current-APK optical performance or production
account server interoperability.

address-editor acceptance is retained under
`native/android/.local/peer-address-polish-physical-20261010-1/acceptance.json`.
An unsaved name draft survived invalid-address rejection, address discard,
offline address save and authenticated route repair. Discard removed the address
error immediately. Check connection refreshed the repaired endpoint on the same
screen; closing/reopening did not persist the unsaved name. Original public
trust, routes, names, paired metadata and receiving preference were restored
exactly. This does not cover physical scoped IPv6 or account-only endpoints.

Account relay source and checks are retained under
`native/android/.local/account-relay-source-20261010-1/source-ready.json`.
The build retained 131 frozen source hashes and verified the standalone bundle,
development package, signature and final stripped JNI stage. Core/native checks
reject a substituted trusted Noise identity before any grant or peer-specific
side effect, isolate per-operation cancellation and registration epochs, and
keep fresh account routing separate from manual trust. The isolated real-server
integration exercises registration/proof, presence, Unicode and control in both
directions through actual HTTP and WebSocket connections; it is not physical
Android-to-desktop Internet acceptance or a WAN performance measurement.
Standard WebPKI roots are used; enterprise/custom CA behavior remains unverified.

default cursor selection is recorded in
`native/android/.local/cursor-latency-20261010/production-cursor.runtime.json`;
it is a functional LAN check with tracing disabled, not a new optical result.
Isolated physical account receipts are retained under
`native/android/.local/android-account-physical-20261010-1`, including
`browser-completion.json`, `keystore-restore.json`,
`ui-peer-removal-cancelled.json`, `status-peer-removed.json` and
`revocation-acceptance.json`. The final revocation check used a fresh app process
and a separate observer's accessibility action to enter Account; it confirms the
signed-out UI and preserved manual pairing. It does not establish hardware
attestation, production account deployment or the unverified account paths above.

Current display keyboard/touch receipts are retained under
`native/android/.local/display-keyboard-physical-20261010-1/acceptance.json`.
They verify installed and Mac hashes, native USB media selection, exact
1138×710 source geometry, delivered AppKit events and source text, and held-key
release before the local Android command completed. Earlier shortcut attempts
with lost source focus and subsequent focus-gated attempts remain invalid;
an immediate release snapshot was superseded by a separate live-command wait.
The final observer records focus changes separately from input, and requires
the active application's exact key window and editor for typing. These checks
do not establish Unicode delivery, Command shortcuts, HOME lifecycle acceptance
or optical performance. Display and observer sessions were stopped afterward.

Local device information and permission receipts are retained under
`native/android/.local/local-device-info-physical-20261010-1`. The name check
verified a fresh app process and restored the original system name through the
normal UI. The installed release APK did not permit `run-as`, so these receipts
do not claim exact private pairing-metadata equality. The existing paired Mac
and unchanged public fingerprint were verified in the application UI instead.

outgoing manual-pairing acceptance is consolidated in
`native/android/.local/manual-cancel-physical-20261010-3/acceptance.json`.
Each successful case used a disposable authenticated loopback peer, verified
the installed APK hash and preserved all existing local pairing metadata.
Failed driver attempts remain failed: the first had a private-code file-writing
error; the second retry completed PAKE but failed its uppercase native-dialog
selector. Its disposable peer was removed through the normal UI before the
fresh passing retry. Cancellation proves local cleanup; a remote fixture may
independently commit its own trust after its late grant.

independent-control Unicode receipts are retained under
`native/android/.local/unicode-physical-20261010-2/control-unicode-acceptance.json`.
The actual editable field and tagged AppKit key events contain the entire
32-unit fixture, including its tail after 20 UTF16 units. Mac fixes
`ManagedCursor` failing to delegate the active helper's text capability; its
helpers remain byte-identical to . The initial missing-grant attempt remains
failed. A subsequent shell IME-switch attempt produced no source events and also
remains failed; recreating the keyboard connection enabled delivery. Its exact
rejection guard was not isolated. The accepted
cases use that fresh connection with the ABC receiver layout. A stale binder
may be rejected by Android before reaching the product, so the absence check
does not independently prove which stale-connection guard rejected it. These
receipts are functional input checks, not optical or performance results.

USB display-scoped Unicode receipts are retained under
`native/android/.local/unicode-display-physical-20261010-1/acceptance.json`.
Actual source-editor focus and target/main-display guards were required. The
32-unit commit and 17-unit composition finish each produced one balanced tagged
key pair in the target editor; draft and duplicate finish produced no new key
pairs. Retained-connection calls after keyboard off produced no new source text
or key events. The original harness incorrectly required Cocoa key-up characters
to match the full commit; that attempt remains failed, with its source snapshots
preserved. Corrected checks require exact editor text, full key-down text and a
balanced key-up. The ABC layout was used. Original keyboard and accessibility
settings were restored, the disposable test IME removed, and the observer and
virtual display stopped. These checks use no optical camera.

Android-receiving Unicode stage receipts are retained under
`native/android/.local/input-receiving-text-physical-20261010-4/unicode-stage-acceptance.json`.
The source observed an authenticated text grant. The focused tablet editor's
native change record and separate accessibility value both contained exactly
`extendCafé é — 中文测试 👩‍💻 🧑🏽‍🚀` (34 UTF16 units), with one additional
change for the committed-text event. The full run remains failed: its later
scroll did not reach the test content while the keyboard was open, and the
editor subsequently changed. Current-build scroll, hold and disconnect are not
accepted by this stage-only receipt. The temporary peer was removed through
normal UI and availability/input preferences remained unchanged.

A Mac GUI-launch session showed both peers Offline while native USB and
incoming control worked. Restarting the identical signed build restored Online;
the original cause remains unproved. If it recurs, retain route diagnostics
before changing launch context. Identity mismatches from trying another peer's
mDNS candidate alone do not establish a failed route to the expected peer.

receiver-settings acceptance is retained under
`native/android/.local/receiver-settings-physical-20261010-2/acceptance.json`.
The tablet changed the source from Fit-display 1138×710 / 60 FPS / 20 Mbps to
1280×800 / 30 FPS / 8 Mbps, then restored the exact starting settings. Native
callbacks after both configuration revisions and CoreGraphics inventories
confirmed actual output. The physical Mac display remained active, unmirrored,
main and 1728×1117 throughout. Mac fixes mode changes restoring a mirror
group: the virtual adapter retains valid placement, clears only mirror links
involving its own virtual display and verifies topology and mode before its
acknowledgement. A tagged tap after reconfiguration reached the actual target
window with one balanced down/up pair and no click in the main guard. The
observer's global move records are not misclassified as main-window clicks.
Reopening retained confirmed choices; Back closed only the panel. Normal Stop
restored the original source inventory; the observer stopped, touch/keyboard
were off, and original Android IME, accessibility and brightness settings were
preserved. USB and receiving remained available. These are functional checks
on Redmi Pad 2 / Android 16, with no camera or performance measurements.

The earlier Mac receipts in
`native/android/.local/receiver-settings-physical` remain failed:
the initial connection overflowed the unchanged frame queue after a pause and
burst; reconnecting the same builds succeeded, but Fit-display restoration
then reproducibly mirrored the physical display. The first failure's upstream
pause location remains unproved and its investigation is deferred. Mac 
addresses the separately reproduced topology defect. A metadata-builder
attempt in the passing directory also retains its incorrect helper-filename
assumption; the final receipt reads the verified package manifest instead.

availability notification receipts are retained under
`native/android/.local/notification-stop-physical-20261010-1/acceptance.json`.
The action was selected from the actual expanded availability card, separately
from the USB notification. Native cleanup, both off switches and a same-process
Home/reopen passed before normal Settings re-enable restored Available and
Online in both applications. Current OS NSD client requests contained no own
presence advertisement while off and exactly one afterward; historical logs and
remote discovery caches were excluded. Other preferences and all five saved
engine files remained byte-identical. Receiving was restored on, the USB carrier
remained connected, and the tablet app stayed in Devices. No active incoming
media/control session was present; this does not accept active-session Stop,
fresh-process recovery, Doze or optical performance.

The standalone `scripts/android/probe_display_extension.py` builds the Android
extension capability probe. Its instrumentation targets its own ordinary UID,
uses `PUBLIC | OWN_CONTENT_ONLY`, and does not adopt shell permissions. The
device observation above matches Android's [virtual-display launch restrictions](https://source.android.com/docs/core/display/multi_display/recommended-practices)
and the [privileged companion app-streaming role](https://source.android.com/docs/core/permissions/app-streaming).

saved-address recovery receipts are retained under
`native/android/.local/peer-address-physical-20261010-1/acceptance.json`.
The tablet rejected a hostname draft without changing stored state, retained the
unsaved indicator while Advanced was collapsed, discarded back to the original
address, and saved/reopened the offline literal hint `192.0.2.123:48177`.
Normal Check connection recovered the existing Mac route after a pinned probe;
the same screen showed Online, and reopening showed the repaired address.
Original canonical decoded paired metadata, byte-identical identity/trust/name/route files, local settings,
labels, keyboard and accessibility choices were retained. Devices ended Online
with receiving enabled. Physical IPv6, account-only routes and independent name
and address drafts remain unverified. Initial sleeping-installer and premature
editor-driver failures remain failed. These are
functional checks without a camera or new performance comparison.

corrects the input service's neutral disabled dimensions to the shared
protocol minimum of 16×16. The standalone JS and stripped JNI library are byte-identical to ; only
`InputReceivingService.java` changed among the 131 frozen build inputs.
`native/android/.local/input-binding/input-binding-ready.json` confirms normal
service binding without an additional permission toggle, preserving existing
accessibility and input-method preferences. The initial binding crash is
retained as failed evidence under
`native/android/.local/input-permission-recovery-physical`.

The strict revocation attempt remains failed under
`native/android/.local/input-permission-recovery-physical`.
An actual native continued contact was held before the exact own service was
removed from the enabled list. The encrypted source disconnected, but the
target still recorded `held`; it did not observe a terminal UP/CANCEL event.
All other services, original accessibility/IME preferences and original public
pairing metadata were restored, and the temporary source peer was removed
through normal UI. The failure is not upgraded to a release pass by cleanup.

The [public continued-stroke API](https://developer.android.com/reference/android/accessibilityservice/GestureDescription.StrokeDescription)
intentionally keeps a continued pointer down after its segment ends. AOSP
[connection reset](https://android.googlesource.com/platform/frameworks/base/+/855d1739177c/services/accessibility/java/com/android/server/accessibility/AbstractAccessibilityServiceConnection.java)
clears the service proxy without an injector-cancel operation, and
[dispatchGesture](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/core/java/android/accessibilityservice/AccessibilityService.java)
returns false when that connection is unavailable. This is consistent with the
observed failure; the exact Xiaomi framework implementation has not been
inspected. Best-effort release while still bound remains necessary, but does not
establish guaranteed cleanup after external permission removal. A prequeued
finite tail also cannot simply renew a live drag: the AOSP
[injector](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/accessibility/java/com/android/server/accessibility/MotionEventInjector.java)
computes its terminal state before delivery. Bounded complete gestures would
change the continuous-drag behavior and are not silently substituted here.

Separate recovery-only acceptance is retained under
`native/android/.local/input-permission-recovery-physical/result.json`.
An actual balanced click was observed before revocation. Removing only the own
service disconnected the granted encrypted source; a fresh request by the same
pinned identity returned the typed `RemoteConsentDenied` result. Restoring the
exact original service list restored Ready for paired devices. The same peer
then delivered a fresh native click and the exact 28-UTF16-unit Unicode fixture,
verified by the target's native change record and separate accessibility value.
Normal UI removal affected only the disposable peer, original peer metadata and
receiving/input preferences were preserved, all five captured accessibility/IME
settings matched, and the transient pairing-code file was deleted. This receipt
explicitly records `held_revocation_accepted=false` and does not supersede the
failed continued-contact case. No camera or performance benchmark was used.

USB-only signaling receipts are retained under
`native/android/.local/usb-only-physical-20261010-1`.
Both full builds, frozen inputs and signatures passed. Shared and adapter host
checks cover pinned presence, control/Unicode and display signaling in both
directions, wrong-identity rejection before grants or peer-specific effects,
receiving-off admission, detach and captured-carrier cancellation. These source
checks do not establish physical input delivery over USB.

With tablet Wi-Fi verified off and USB in `accessory,adb` mode, normal desktop
GUI Extend produced HEVC 1138×710 tablet output. Native rendered counts advanced
from 1 to 120, and inspected tablet pixels showed the actual desktop wallpaper.
The physical Mac display stayed 1728×1117 and unmirrored. Normal UI Stop ended
that display session. A first 20-second logging wait was insufficient for the
static desktop; the retained accepted receipt uses the later 120-frame snapshot
and does not interpret its elapsed time as cadence or latency. Reverse capture
through Android's normal Mirror and consent flow also rendered HEVC 1280×800
with Wi-Fi still disabled. The fresh source reported 119 encoded and sent frames;
the native receiver recorded decoded/presented events and its actual viewer
showed the disposable Android target. Those counts establish functional delivery,
not a frame-rate result. The original full notification Stop runner remains failed
at its title lookup. Its fallback app Stop released projection and restored
original per-peer sharing choices. A separate notification-only acceptance is
recorded below; the patched full runner has not been rerun on hardware.

The limited acceptance is recorded in `acceptance.json`; failed preconditions
and notification evidence remain retained. Wi-Fi was restored to its original
value, Accessibility/IME and awake preferences matched, and both normal display
stops restored the exact original physical Mac topology. Mac trust and receiving
preference matched; the existing tablet LAN port refreshed after the app restart
while its name/edge remained unchanged. No private bridge address was persisted.

Minor concurrent detach/unpair versus authenticated-name metadata updates remain
a separate lifecycle limitation; they are not evidence of an identity or input
admission bypass. No camera was used, no latency or external-product comparison
is accepted by these functional receipts, and streaming tuning remains deferred.


A separate USB capture passed the normal notification Stop action under
`native/android/.local/screen-notification-physical-20261011-1/acceptance.json`.
The capture notification and its PendingIntent existed. MIUI collapsed summary
nodes belong to SystemUI, while expanded notification RemoteViews expose the
posting app's package. The old harness excluded the actual title and action.
The corrected harness searches both packages and scopes its expansion/action
selection to native notification resources; seven host cases passed. The fresh
normal Stop action removed projection, the capture service and notification 3402,
retaining availability/USB notices and the original sharing, Accessibility/IME,
Wi-Fi and awake settings. Four actual peer-name/host and paired/local preference
file hashes matched. Three earlier nonexistent filename measurements hashed
error output; they are explicitly invalid, retained and corrected in the receipt.
They do not establish native trust-file preservation. The original full runner's
failure remains unchanged.

Build and install receipts are retained under
`native/android/.local/standalone-package`.
Both packages contain the same standalone JavaScript and arm64/x86_64 engine
libraries. Exact packaged ABI sets, ELF machines, defined public JNI export and
16 KiB engine LOAD alignment passed. Only arm64 has physical runtime acceptance.
The in-place replacements retained nine valid SHA-only measured files, including the
actual `no_backup/engine/identity.v1`, peer names/hosts, receiving,
paired/local and per-peer sharing preferences, account configuration and
appearance. These same-versionCode-1 replacements do not establish a production
increasing-version update or credential backup/restore.

adds an appearance-only backup allowlist for legacy backup, cloud backup
and device transfer. Identity-bound preferences remain outside that allowlist;
identity/trust and account credentials already use no-backup storage. The actual
packaged manifest points to the compiled XML rules, verified by resource IDs
and decoded packaged XML. The first full release build failed Android lint
because redundant exclusions were outside its sole included appearance path;
the invalid rules and failure log remain retained. Removing those redundant
exclusions preserved the effective allowlist and passed release lint and the
full build. No lint suppression was added. No physical backup/restore was
attempted, and this policy does not remove historical backups or earlier orphan
preferences. USB accessory sessions end during app replacement; a fresh normal
cable connection remains required. Testing restored the original MTP mode before
reconnecting the exact paired tablet through the normal desktop USB chooser.


normal terminal UI acceptance is retained under
`native/android/.local/end-copy-physical-20261011-1/acceptance.json`.
A fresh native USB extension rendered the actual desktop at 1138×710, with
separate cursor enabled and hardware HEVC decoding. Normal desktop Stop now
shows the neutral `Display ended` title with no empty native error. Back returns
to the existing paired Mac marked `Online · USB`. The exact original physical
Mac topology was restored; all nine valid measured own files still matched after fresh
launch, display and cleanup. Settings retained the original public fingerprint,
Stay available and input receiving on, and Ready for paired devices. USB power,
Awake and charging stay-on were verified, with the extended screen timeout
retained. This is a functional check, not an optical/performance result. Failure
details remain visible when native termination supplies an actual reason.


Metadata evidence correction: `no_backup/engine/trust` is a
directory, not the native trust data file. Android's hash command returned the
empty SHA256 for that directory; those raw targets are not trust-byte evidence.
The raw receipts remain retained, with `metadata-target-correction.json`
explaining the invalid target. Nine actual files still match across the recorded
replacements. The actual native file is `no_backup/engine/trust/trust.json`; its
first corrected hash was captured after replacement, so it cannot establish
historical trust-byte preservation. Functional pairing/identity observations
remain separate from byte comparisons.

The package installs the native control keyboard focus-return fix. A blurred editor loses
commit permission immediately. The current connected, attached, focused keyboard
gets one deferred `restartInput` after focus returns; a framework-created editor
cancels that recovery, and old composing drafts are never replayed. Native
compile, focused Kotlin lifecycle/commit checks, TypeScript, lint and the full
frozen-source standalone APK build passed. The first install attempt timed out
while Android reported no focused window; its failure remains retained. A retry
with independent scoped dialog observation installed the exact verified APK.
Nine valid measured files matched before and after replacement. Physical IME
focus-return acceptance passed under `native/android/.local/ime-focus-recovery-physical/acceptance.json`. Notification-shade blur/return retained one authenticated control grant, advanced the IME epoch, delivered the exact 32-UTF16-unit Unicode fixture once, discarded the prior composing draft and delivered no text from retained stale commit/finish calls. The receiver observed encrypted input only and never invoked Mac OS input APIs; ADB reverse carried this isolated fixture, so this is not production USB control or native Mac editor acceptance. The first driver attempt failed due to shell quoting and remains retained. Both attempts restored the original Gboard/Accessibility settings, removed their own reverse and unpaired only their disposable peer. Normal native USB was reconnected afterward; both apps show the original pair online, all nine valid file hashes match, the test IME is removed, and the tablet remains charging and awake at 80%.

A feature-gated desktop `--usb-control-test` diagnostic uses the actual selected
USB adapter, paired identity, pinned Noise handshake and outgoing job cleanup
without global input capture or LAN fallback. Four focused host checks passed,
including encrypted ASCII/Unicode delivery and held-input cancellation through
the shared USB adapter with a synthetic carrier. Source hashes and checks are
retained under
`desktop/.local/usb-control-diagnostic-source/source-ready.json`.

Signed Mac physical USB control acceptance passed under
`native/android/.local/usb-control-physical-20261011-1/result.json`. With tablet
Wi-Fi disabled, the installed desktop diagnostic delivered a native button
click, physical `extend` key pairs and the exact 28-UTF16-unit Unicode fixture
into the disposable tablet editor. Native text-change records and an independent
focused-editor observer agreed. The normal Mac **Stop sharing** action cancelled
the authenticated job and released a held native touch while preserving the
editor text. **Disconnect USB** was disabled during the active job; this receipt
therefore proves normal session cancellation, not unplug or carrier-disconnect
release. All ten measured regular files, including the actual native
`no_backup/engine/trust/trust.json`, matched before and after this check. The
existing tablet process, original pairing, names, routes, availability, input,
Gboard and Accessibility preferences were preserved; Wi-Fi was restored on.
Normal Mac USB was then reconnected through the GUI, which showed the tablet
Online, USB connected and receiving enabled. The tablet remains awake and USB
powered. This covers Mac-to-Android native input through the production USB
session adapter; global desktop input capture, edge handoff, reverse USB native
delivery and permission-revocation behavior remain separate acceptance gaps.

native password sign-in acceptance is retained under
`native/android/.local/account-held-revocation-physical-20261011-1/native-signin.json`.
The actual secure native dialog signed in to a disposable local account; the
tablet UI and server directory agreed on both verified public identities. The
password was not saved in Google Password Manager. The account-only Mac input
attempt failed at the initial presence TCP connection, before any input grant or
delivery; active held-input revocation remains unverified. The later backend
refresh error coincided with the fixture's ADB reverse already being absent; it
does not establish a product account-refresh defect. Normal UI sign-out and
server restoration removed the fixture credential. `cleanup.json` verifies all
ten regular file hashes, original public pairing state, input/awake preferences,
Wi-Fi and the unchanged tablet process. The owned local server was stopped.

The next native control focus fix is frozen under
`native/android/.local/control-touch-focus-20261011-1/source-ready.json`.
It cancels a pending long press on blur, releases a held contact once and rejects
stale motion until a fresh focused touch begins. Native compilation and focused
gesture/IME checks passed; this source is not yet in an installed build and needs
physical drawer/drag acceptance. A separate Mac Wi-Fi helper diagnostic change
is frozen under
`native/android/.local/wifi-readiness-diagnostic-20261011-1/source-ready.json`.
It records bounded, allowlisted XPC failure categories without changing the
readiness gate; Swift and focused Rust checks passed. It is not yet in Mac .
