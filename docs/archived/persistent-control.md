> Archived milestone report. See [current input control](../architecture/input-control.md) and [CLI operation](../operations/cli.md).

> 🤖🔧 ai generated

# Terminal sessions and trusted reconnect

`extend-computer control` provides full mouse and keyboard control until stopped or disconnected. The existing `extend-computer cursor --input` remains a short 30-second development mode. Both interfaces use the same session, permission, and native input engine.

On Macs with several local screens, handoff uses the screen containing the cursor,
not just the main display. Crossing an exposed left or right edge transfers
control; seams between local screens keep normal local movement. Return and
emergency cancellation restore the cursor to the screen that initiated handoff.
Screen arrangement changes end capture safely. Persistent desktop sessions report
unexpected capture shutdowns, including lost heartbeats, instead of treating them
as a normal user stop.

The desktop sender retries transport failures after an established connection up
to three times, waiting 1, 2, and 4 seconds. It shows Connecting while recovering,
pins the same peer identity, and requests only already remembered control on
retries. Each capture starts locally with fresh input state. Disconnect cancels
backoff; permission, identity, helper, and protocol failures are terminal. An
unrecovered outgoing failure appears as an error instead of silently returning
to idle. This recovery does not diagnose or eliminate the underlying network
stall.

## Existing two-Mac launcher

Replace `--input` with `--session` in the previous `scripts/two_mac_cursor.py` command. It starts both sides over the existing SSH setup, uses ephemeral test identities, and runs until Control-Option-Escape or Ctrl-C. Add `--seconds 75` for an optional automatic stop. This development launcher does not provide identity continuity or automatic reconnect across process restarts.

## Normal CLI workflow

Use a persistent Keychain identity and the same explicit state directory for every command. On each Mac, for example:

```sh
extend-computer --state "$HOME/Library/Application Support/extend.computer" serve \
  --bind 0.0.0.0:48177 --pair \
  --cursor-helper "$PWD/target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor"
```

Run the receiver's command in its normal desktop Terminal so macOS can attribute its native helper permissions correctly. For the SSH development setup, the existing desktop helper wrapper provides that GUI-session context.

On the controlling Mac:

```sh
extend-computer --state "$HOME/Library/Application Support/extend.computer" control RECEIVER:48177 \
  --helper "$PWD/target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor" \
  --edge left --offset-y 0
```

Enter the receiver's pairing code and choose `remember` when approving its authenticated identity locally. On the receiver, choose `always-control` to remember this controlling device's full-control permission, or `allow-control` for this connection only. The listener supports one active connection at a time.

Find fingerprints with `extend-computer --state PATH peers`. Then reconnect without another pairing code:

```sh
extend-computer --state PATH control RECEIVER:48177 --peer RECEIVER_FINGERPRINT \
  --helper HELPER_PATH --edge left --reconnect
```

`--reconnect` requires a persistent identity, a pinned receiver fingerprint, and already remembered full-control permission on the receiver. It retries transport failures with 1/2/4-second backoff. Identity mismatch, permission denial/removal, protocol errors, and normal user stop are terminal. Every new capture starts locally; no stale keys, button state, or remote cursor ownership are replayed.

`--seconds N` optionally limits each connection. Without it, control continues until stopped/disconnected. Fixed short-test deadlines remain unchanged in the existing diagnostic modes. Protocol is now `EXTEND04`; update both Rust endpoints together.

## Permissions and stop controls

On the receiver:

```sh
extend-computer --state PATH permissions CONTROLLER_FINGERPRINT --input ask
extend-computer --state PATH permissions CONTROLLER_FINGERPRINT --input allow
extend-computer --state PATH revoke CONTROLLER_FINGERPRINT
```

`ask` removes saved full-control permission and terminates an active session using that permission on its next message/heartbeat. `allow` requires an already known peer. `revoke` blocks the device entirely. Changing saved permissions does not silently upgrade old cursor-only consent. Old trust files default to no automatic full-control permission.

Control-Option-Escape stops the controlling session. Ctrl-C stops the foreground CLI. Both native endpoints retain two-second missing-heartbeat recovery, and the receiver releases owned input state on graceful disconnect or stop. Native hard-SIGKILL input-state recovery remains unproven.

The existing AWDL broker stays bounded to 30 seconds. The engine obtains a replacement lease before releasing the old one every 20 seconds, so the privileged helper always has an accountable live owner. The last lease's release restores the original interface state; no privileged helper upgrade was needed. `--awdl` still opts out of temporary pauses.

## Validation status

Protocol tests cover lifetime beyond 30 seconds, remembered pinned reconnect, permission removal, old-state migration, and a real forced socket drop followed by a new encrypted handshake. A native dry-run receiver stayed alive beyond 30 seconds and released held state on SIGTERM. Bounded 45-second lease-rotation checks passed on both Macs. Live 75-second verification delivered 3,307 events; the user confirmed control worked throughout, beyond the old cutoff. Both helpers had active leases during the run; afterward both reported zero leases and no pending restoration, both AWDL interfaces were up, and no receiver process remained. ACK median was 6.592 ms and p95 11.296 ms, with 21 waits over 40 ms; these are protocol timings, not visual latency. Emergency escape was also tested against a simultaneous connection failure and did not reconnect.

The future GUI will use these same operations and state. Menu-bar UI, shared config loading/default paths, account-like recovery, and cloud backup providers are not implemented in this milestone. See [product direction](product-direction.md).

The automated suite now contains 44 tests, including four persistent-session integration tests. Local Clippy passes with warnings denied. The last sender-only stop marker change does not require replacing the receiving Mac’s already approved native executable.
