> 🤖🔧 ai generated

# Full input prototype

For sessions beyond 30 seconds and trusted reconnect, see [persistent control](persistent-control.md). The short test mode described below remains available.

Mac-to-Mac edge control now forwards left/right/middle clicks, double-clicks, pointer dragging, two-axis scrolling, ordinary keyboard input, repeats, and Shift/Control/Option/Command shortcuts. The user confirmed clicking, typing, shortcuts, text selection, scrolling, and returning locally worked in the first live test (1,145 ordered input events).

## Run

Add `--input` to the previous `scripts/two_mac_cursor.py` command to run the explicitly authorized two-Mac development test. This harness starts both endpoints using the configured SSH key, grants one-session full input consent, and stops after approximately 30 seconds. Omit `--input` to retain cursor-only behavior.

For manual pairing:

```sh
extend-computer cursor RECEIVER:48177 --input --edge left --offset-y 0 \
  --helper "$PWD/target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor"
```

The receiver must be running `extend-computer serve` with its native helper. It asks for `allow-input`, covering mouse and keyboard together. Cursor-only consent (`allow-cursor`) and remembered diagnostic permissions never authorize keys or clicks. Persistent full-control grants are not implemented yet.

Control-Option-Escape stops capture. Both sides retain their 30-second limit. Release held keys and buttons before entering the remote display. A remote drag stays on that display until its button is released; file dragging between computers is not implemented. Returning locally releases the receiver’s held input state.

## Ordering and recovery

Adjacent movement samples can coalesce; button, key, modifier, and scroll transitions retain order. A bounded queue aborts the session on overflow instead of dropping transitions. The encrypted protocol validates input fields, checks revocation before every event, and limits in-flight events to four before an acknowledgment barrier. Protocol version is now `EXTEND03`; rebuild both Rust endpoints together.

The native receiver tracks only inputs this session pressed. Return, normal disconnect, explicit release, SIGTERM, the session deadline, and a two-second heartbeat timeout release that state. The sender also restores its cursor on normal stop or heartbeat loss. Killing the receiver helper with SIGKILL cannot execute its cleanup; full recovery under every OS/process failure has not been established.

A native dry-run lifecycle test verifies state release on EOF, SIGTERM, explicit release, and missing heartbeat, without injecting input into applications. The live test validates ordinary interaction, not every crash case.

## Validation

- 37 Rust tests passed on each Mac, including separate input consent, revocation, ordered transitions, bounded windows, queue overflow, and prior pairing/cursor tests.
- Native input-state and layout tests passed on both Macs; local Clippy passed with warnings denied.
- Universal native builds succeeded on both Macs.
- Dry-run receiver cleanup completed in approximately 4–8 ms for EOF/stop/release and approximately 2.1 seconds for heartbeat loss.
- Live test: 1,145 events, user confirmed all requested interactions. ACK median 6.727 ms, p95 10.15 ms, maximum 97.481 ms; six waits exceeded 40 ms. These are protocol timings, not display latency; occasional network outliers remain.
- Both AWDL interfaces were up afterward, with zero leases and no pending restoration; no receiver process remained.

## Limits

Physical keys currently use explicitly tagged Mac virtual key codes. Cross-platform keyboard adapters remain future work; the receiver’s keyboard layout determines characters. The modifier mask represents Shift, Control, Alt/Option, and Meta/Command. Caps Lock, Fn/media keys, extra mouse buttons, gesture phases, clipboard sharing, file transfer, and virtual display output are not implemented. Scrolling forwards pixel deltas, without full trackpad gesture/momentum metadata.

Ordinary input contents are not logged. Optional `EXTEND_COMPUTER_EDGE_DIAGNOSTICS=1` can record bounded pointer positions; it is off by default. `EXTEND_COMPUTER_INPUT_DRY_RUN=1` disables native event posting for lifecycle tests and must not be set during normal control.

The prototype retains the previously documented macOS background-cursor private API dependency and deprecated warp-suppression compatibility boundary. Development app rebuilds can require a refreshed Accessibility grant because ad-hoc signatures pin an executable hash.
