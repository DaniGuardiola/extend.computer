# Outgoing manual pairing cancellation

This fixture verifies the installed mobile app through its normal **Pair device → Pair manually** UI. It does not require an empty paired-device list, stop the app, change availability, clear app data or import a desktop identity. The fixture is loopback-only and generates a separate disposable identity and trust store for each case. These are functional pairing checks, not transport or latency benchmarks.

The device owner must run the physical commands. Source preparation and host checks do not establish physical UI acceptance.

## Prerequisites

- Install the development APK containing the scoped manual-pairing lifecycle. Leave its Devices screen in the foreground, with active control/display sessions stopped normally.
- Keep existing tablet/Mac pairings intact. The driver snapshots only public trust records, saved peer metadata, authenticated name/route hints and the availability preference. It never reads a device seed, Keystore record, account token or account credential file.
- The separate `computer.extend.android.observer` test APK must already be installed. The driver uses its `ACTION_CLICK` and `ACTION_SET_TEXT` actions with accessibility services preserved. It does not install instrumentation or rebuild an app.
- The development package must allow `run-as` for those exact public metadata paths. No production-package workaround is attempted.

Build and check the isolated host fixture from the repository root:

```sh
RUSTC="$(rustup which rustc)" CARGO_TARGET_DIR=native/android/.local/manual-fixture-target "$(rustup which cargo)" test --locked -p extend-android-engine --example tablet_manual_pairing_peer -- --test-threads=1
RUSTC="$(rustup which rustc)" CARGO_TARGET_DIR=native/android/.local/manual-fixture-target "$(rustup which cargo)" build --locked -p extend-android-engine --example tablet_manual_pairing_peer
python3 -m unittest discover -s scripts/android -p test_manual_pairing_driver.py
```

The host fixture tests cover a real PAKE handshake, explicit pending approval, local cancellation/socket EOF before a deliberately late grant, the independent remote trust commit, successful approval with a clean protocol Close, fresh identity per directory, and stop-before-connect. Driver tests cover baseline preservation, the bounded disposable-peer exception during successful pairing, availability preservation and code redaction.

## Physical commands

Set `tablet_serial` to the already authorized tablet's ADB serial. Use new output directories; existing receipts are never overwritten. Run cancellation first, then retry without restarting the application, so retry exercises replacement of the canceled native reservation. Run Back and Home as separate cases. Each command leaves the Devices screen ready for the next command on success.

```sh
python3 scripts/android/test_manual_pairing_ui.py --serial "$tablet_serial" --case cancel --peer-executable native/android/.local/manual-fixture-target/debug/examples/tablet_manual_pairing_peer --output native/android/.local/manual-cancel-physical/cancel
python3 scripts/android/test_manual_pairing_ui.py --serial "$tablet_serial" --case retry --peer-executable native/android/.local/manual-fixture-target/debug/examples/tablet_manual_pairing_peer --output native/android/.local/manual-cancel-physical/retry
python3 scripts/android/test_manual_pairing_ui.py --serial "$tablet_serial" --case back --peer-executable native/android/.local/manual-fixture-target/debug/examples/tablet_manual_pairing_peer --output native/android/.local/manual-cancel-physical-20261010-3/back
python3 scripts/android/test_manual_pairing_ui.py --serial "$tablet_serial" --case home --peer-executable native/android/.local/manual-fixture-target/debug/examples/tablet_manual_pairing_peer --output native/android/.local/manual-cancel-physical-20261010-3/home
```

Each cancellation case requires all of these gates:

1. The peer reaches authenticated pending approval while local public trust/routes/names and saved peer metadata still equal the baseline.
2. Cancel, Back or Home causes EOF on that actual session before the fixture receives its explicit grant marker.
3. The fixture grants afterward and records its own independent remote trust commit. The tablet still retains exactly its original public state.
4. Returning to the manual screen shows no active attempt or consumed code. A late grant causes no stale navigation.
5. Existing public state and the availability preference remain unchanged when back on Devices.

The retry case uses a fresh identity and a fresh code, requires a clean Grant/Close and the expected local trust plus saved route, then removes **only that new peer** through **Settings for [test name] → Unpair device → Unpair**. It finally requires exact original public state. No trust-file edits are used for setup or cleanup.

## Receipts and cleanup

- `peer/private.json` temporarily contains the code and is created as `0600` under a `0700` directory. Codes are never printed or included in saved UI nodes or errors. No screenshots of the code are taken.
- `peer/status.json`, `pending.json`, `eof-before-late-grant.json`, sanitized UI nodes and the before/after public-state snapshots contain only public metadata and bounded test outcomes. Status updates use same-directory atomic replacement with mode `0600`.
- Status times use the fixture process's monotonic clock only. They are event-order evidence, not cross-device latency.
- The driver creates an exact, ephemeral `adb reverse --no-rebind tcp:PORT tcp:PORT` mapping. It removes only the mapping it successfully created.
- The driver requests stop and, if necessary, terminates only its own child process. The child owns its temporary trust directory. The private code file is deleted after the case.
- A failed gate produces a failed receipt. Review `result.json` before continuing. Failure cleanup may leave the UI briefly awaiting the fixture EOF or native expiry; reopen the normal UI before retrying. If `own_peer_cleanup` needs review, remove only the receipt's disposable fingerprint/name through its ordinary device settings.

Local cancellation cannot undo another device's explicit approval. The late-grant receipt deliberately proves that remote trust may be committed while tablet trust remains unchanged. Do not describe these outcomes as atomic rollback on both devices.

For manual fixture control, start the executable with a new private directory. Create an empty `grant`, `deny` or `stop` file there using a private umask. The first connection owns the one-time code. The fixture waits at the approval boundary and retains that boundary after EOF specifically to support the late-grant case. It ends after that single session or its bounded lifetime; it does not advertise or open a LAN listener.
