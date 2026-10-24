# Android input receiving: physical keys and committed text

This fixture drives the development app through its current pairing UI, then sends authenticated input to a disposable native editor. It preserves every baseline peer, public name/route record, saved metadata and availability/input preference. It never edits trust files, clears app data, imports a desktop identity or reads account credentials/device keys.

## Prepared source

- `native/android/engine/examples/tablet_input_source.rs` creates a fresh identity, pairs through the tablet's one-time code and reconnects using the tablet's pinned fingerprint. Its `text_supported` receipt comes from the actual authenticated control grant. Every command receipt retains that grant and includes an ordered `command_sequence`; a previous same-named command cannot satisfy a newer command's wait.
- The `unicode` command sends only the fixed public fixture `Café é — 中文测试 👩‍💻 🧑🏽‍🚀`: 23 Unicode scalars, 28 UTF-16 code units and 54 UTF-8 bytes. It covers precomposed/decomposed accents, CJK, emoji, ZWJ and a skin-tone modifier. It does not accept arbitrary user text. The existing physical-key `type` command still emits key down/up pairs for `extend`.
- `native/android/tests/ReceiverTargetActivity.java` records actual changes in its own disposable editor, including the exact resulting text, UTF-16 length and change count. It never observes another application's editor. This test target is absent from the shipped app.
- `scripts/android/test_input_receiving_ui.py` uses the separate observer with accessibility services preserved. It follows **Pair device → Pair manually → Show my code**, checks the incoming dialog's exact source fingerprint, and approves only that source. Cleanup uses the temporary peer's unique name and the scoped native Android positive buttons, including uppercase button labels.
- `pairing-selection.json` records the chosen address, every displayed address and the observed expiry label without the code. `source.json` retains the chosen numeric address, a capped stage history with process-local monotonic times and separate TCP-connected, authenticated-ready and remote-grant observations. A terminal failure records the last stage and typed I/O/policy category, without arbitrary error text.

Host checks and fixture build:

```sh
RUSTC="$(rustup which rustc)" CARGO_TARGET_DIR=native/android/.local/manual-fixture-target "$(rustup which cargo)" test --locked -p extend-android-engine --example tablet_input_source -- --test-threads=1
RUSTC="$(rustup which rustc)" CARGO_TARGET_DIR=native/android/.local/manual-fixture-target "$(rustup which cargo)" build --locked -p extend-android-engine --example tablet_input_source
python3 -m unittest discover -s scripts/android -p test_input_receiving_driver.py
python3 -m unittest discover -s scripts/android -p test_manual_pairing_driver.py
```

The Rust tests use real authenticated loopback sessions. They require committed-text delivery only when the receiver actually grants it, reject the unavailable-adapter case without sending text, and retain physical key down/up coverage in both cases. They also distinguish refused TCP from a TCP listener that closes before authenticated readiness, and record readiness before a delayed remote grant or denial. Receipt tests cover private atomic files, retained support and stale command IDs. Host driver tests require exact Unicode/UTF-16 editor evidence, preserve XML types, reject unobserved code/address choices, exclude stale same-stage acknowledgments and report a failed source's sanitized stage even after process exit. These checks do not establish Android editor delivery.

## Physical preparation owned by the device operator

1. Keep all existing tablet/Mac pairings intact and stop active display/control sessions normally. Leave Devices visible.
2. Use a debuggable development app with the negotiated text implementation. The driver requires `run-as` for the allowlisted public metadata files and fails if it cannot inspect them.
3. Install the separate observer and refresh the app-signed `computer.extend.android.enginetest` APK with the updated `ReceiverTargetActivity`. The target must produce `text`, `text-utf16` and `text-changes` in its own `shared_prefs/input-target.xml`. An older test target deliberately fails this delivery gate. The device operator can use `scripts/android/test_engine_device.py --build-only` to rebuild the test package, then install only that test APK. Do not rerun its unrelated identity/tamper instrumentation as part of this acceptance slice.
4. Turn on **Stay available** and **Receive keyboard & mouse** normally; enable the input accessibility service. The driver requires **Ready for paired devices** before pairing and never changes these settings. Keep tablet orientation fixed and unlocked.
5. Supply the physically verified native tablet fingerprint as `tablet_peer`. The optional `--pair-address` must match an exact address shown on the current code screen. The fixture uses a normal authenticated network connection; no ADB transport tunnel is created.

Run once with a new private receipt directory:

```sh
python3 scripts/android/test_input_receiving_ui.py --serial "$tablet_serial" --tablet-peer "$tablet_peer" --peer-executable native/android/.local/manual-fixture-target/debug/examples/tablet_input_source --output native/android/.local/input-receiving-text-physical
```

Optional `--preserve-name 'Existing Mac'` requires that peer to be visible both before and after the run. Exact preservation of all public persistent state is required regardless of this additional visible-name check.

## Required delivery gates

- The authenticated control grant reports `text_supported=true` and the expected physical display dimensions.
- Remote click selects the test editor. The separate observer must report that exact editor's actual input focus before any physical key is sent; `editor-focused-before-type.json` saves that observation. A click acknowledgment alone cannot satisfy this gate. Physical key events produce exactly `extend` in both its actual editor and its independent change record.
- One committed-text event appends the full 28-code-unit Unicode sample. `unicode-editor.json` must contain exactly `extend` plus that sample, the correct UTF-16 length and a new change count. `unicode-editor-ui.json` must independently expose the same exact editor value. A protocol pong or queued-event receipt alone is insufficient.
- Release leaves the control session usable. Native click, scroll, continued drag and a two-second stationary hold work. Closing the source releases the held contact and preserves the editor's full text.
- The normal UI unpairs only the newly generated source. Original public trust, revocations, routes, names, saved metadata and both availability/input preferences remain unchanged.

## Cleanup and evidence limits

All receipt directories are `0700`; transient code and receipts are `0600`. No code screenshots or unredacted pairing-code UI dumps are saved. The transient code is deleted; the source's own temporary trust directory is removed on process exit. Cleanup closes stdin and terminates only the child process created by this driver if necessary. It does not stop the application or its receiving/accessibility services.

Failed gates retain sanitized receipts and remain failures. If ordinary test-peer cleanup is unavailable, `result.json` records `own_peer_cleanup=needs-root-review`; remove only that receipt's temporary fingerprint/name through device settings. No success is inferred from source protocol acknowledgment when the actual editor evidence is missing. This single Android/version/editor acceptance does not prove every third-party editor, IME, device vendor or older Android input adapter.

Before failure cleanup changes the source or foreground UI, the driver attempts to save the disposable editor's own native preferences and package-filtered focus nodes, the latest source receipt and all allowlisted public state. `failure-evidence.json` records unavailable observations and the exact availability/input preference delta. It never changes a disabled input preference back automatically or upgrades a failed gate to a pass.

For connection failures, `pairing-tcp-connect` means that the TCP connection has not been confirmed; `pairing-handshake-ready` covers PAKE, Noise, the authenticated compatibility exchange and remote Ready as one boundary. `pairing-request-grant` means that authenticated Ready was received and the local consent callback ran; the fixture has not yet confirmed remote Granted, and it cannot distinguish RequestProbe send from the subsequent grant wait. `pairing_remote_grant_observed=true` is set only when `Client::connect` returns successfully. Later probe, close and device-name exchange stages remain separate. An older receipt that only says `waiting` cannot identify the failed call; preserve that failed run and use a fresh directory for a diagnostic retry.
