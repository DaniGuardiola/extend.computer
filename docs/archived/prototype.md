> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

> 🤖🔧 ai generated

# pairing prototype

This first executable milestone provides diagnostic probes only. It does not capture input, inject events, read clipboard contents, capture screens, create displays, expose a shell, or grant those capabilities. Pairing/session integration is experimental and has not had an independent security audit.

## Build and test

```sh
cargo build --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

On the receiving Mac:

```sh
./target/debug/extend-computer serve --bind 0.0.0.0:48177 --pair --advertise
```

The explicit bind exposes the diagnostic listener on all IPv4 interfaces. Default without `--bind` is loopback. No router mappings are opened. This is not a firewall-enforced LAN-only boundary; only use the explicit bind on the intended test network. Authentication applies regardless of where packets originate.

On the connecting Mac:

```sh
./target/debug/extend-computer discover
./target/debug/extend-computer pair other-mac.local:48177
```

Enter the displayed code in the hidden terminal prompt. Both terminals must approve probes. Choices are `once`, `remember`, `automatic-probe`, or `deny` (default). `remember` pins identity but still requires approval next session. `automatic-probe` grants future diagnostic probes only. Discovery labels and addresses remain unverified until pairing authenticates the key.

Remembered-device reconnection:

```sh
./target/debug/extend-computer peers
./target/debug/extend-computer connect other-mac.local:48177 --peer FULL_FINGERPRINT
./target/debug/extend-computer revoke FULL_FINGERPRINT
```

Use the same `--state` directory and `--identity` account across invocations. State defaults to `.extend-computer-state` relative to current directory. Device keys use macOS Keychain service `computer.extend.prototype.identity.v1`; default identity account is `default`. Keychain failure is fatal; no file-secret fallback. `--ephemeral` creates an in-memory test identity lost on exit. Never expect an ephemeral key to survive process restart.

Revocation is a persistent deny entry. It blocks the next message in an active session and all future reconnections. The connection itself is dropped on that message or its idle timeout (10 seconds). This prototype has no automatic un-revoke command; use a fresh isolated test state when repeating tests. Revocation cannot be undone by possession of an old pairing code.

## Wire/session design

- Fixed versioned header distinguishes pair and reconnect. No downgrade fallback.
- Pair code is 64 random bits displayed as four hex groups; one attempt per 120-second window, including failures. The UI can later replace hex with a human-friendly encoding. An unauthenticated peer can consume this window (bounded denial of service); local user must reopen it.
- Pairing uses `spake2` 0.4 with distinct connector/listener role identities. HKDF-SHA256 derives a Noise PSK from the PAKE output. Length-delimited PAKE transcript and protocol context enter the Noise prologue.
- Pair transport uses Snow's `Noise_XXpsk0_25519_ChaChaPoly_SHA256`; reconnect uses `Noise_XX_25519_ChaChaPoly_SHA256` with mandatory pinned peer checks on both ends before authorization. Ephemeral handshake keys are fresh per connection.
- Device identity is the SHA-256 fingerprint of the Noise static public key, not hostname/IP. Neither side persists trust until its approval and the relevant authenticated authorization exchange.
- Encrypted server readiness provides key confirmation before the client approval prompt. Server approval follows an authenticated diagnostic request.
- Frame size is capped at 4096 bytes before allocation. Handshake reads have a shared absolute 10-second deadline. Active message reads also have absolute deadlines. Noise transport supplies authenticated encryption and implicit sequence nonces; replay/tampering tests verify rejection.
- Permission is fixed to diagnostics in this wire version. Unknown message variants fail. Input/video will require a new explicit capability model and tests, never reuse an automatic-probe grant.
- Trust writes use a file lock and atomic replacement. Every probe checks current revocation state. State corruption fails closed.
- A single listener handles one session at a time; diagnostic sessions last at most five minutes. Application consent prompts are terminal-based; waiting on local stdin is not a production UI lifecycle implementation.

No claim that library selection alone makes this composition audited. Before enabling input/video, review the PAKE-to-Noise binding, key confirmation, state transitions, identity lifecycle and permission model. Before internet exposure, add connection-level abuse controls, bounded concurrent scheduling and independently reviewed remote rendezvous/relay design.

## Two-Mac automation

`examples/peer_smoke.rs` exercises pairing, encrypted probes, reconnection, active revocation and rejection after revocation. It uses temporary state and an in-memory identity. `scripts/two_mac_smoke.py` starts the normal listener over authenticated SSH, drives explicit diagnostic approval through stdin, discovers it, runs the example over ordinary LAN TCP, revokes via SSH, and terminates the remote listener. SSH is orchestration only, never the probe transport.

```sh
cargo build --locked --bins --examples
python3 scripts/two_mac_smoke.py \
  --ssh-host USER@REMOTE.local --peer-host REMOTE.local \
  --key /path/to/ssh-key --known-hosts /path/to/verified-known-hosts \
  --remote-dir /path/to/second-mac-checkout \
  --local-bin ./target/debug/extend-computer \
  --driver ./target/debug/examples/peer_smoke
```

Build the same source on the remote Mac first. The harness uses port 48177 and refuses to weaken SSH host checking. It retains only nonsecret test trust/revocation records in `.test-state-*`; pairing codes are not printed or saved by the harness. The interactive listener displays its code intentionally, so don't redirect its output into persistent logs.

## Remaining work

No input/video adapter, Tauri UI, lock-state lifecycle, same-session resume, internet traversal/relay, or cross-platform identity store yet. New persistent identities may need initialization in a normal interactive Mac session: macOS can reject Keychain operations from SSH. A temporary job launched through `launchctl bootstrap gui/UID` successfully accessed the remote identity after interactive initialization, while direct SSH still could not. Use the logged-in desktop session for keychain-backed agent tests. Code signing/notarization and stable Keychain access across upgrades remain packaging work.

Discovery currently advertises IPv4 because the prototype's listener is IPv4-only. Address fallback handles hostnames resolving to IPv6 first. A short browse can miss initial advertisements; default browse is eight seconds. Native Bonjour and Rust mDNS must be compared further before shipping.

References: [SPAKE2 API](https://docs.rs/spake2/0.4.0/spake2/), [Snow API](https://docs.rs/snow/0.10.0/snow/), [Noise specification](https://noiseprotocol.org/noise.html), [macOS Keychain wrapper](https://docs.rs/security-framework/latest/security_framework/passwords/index.html).
