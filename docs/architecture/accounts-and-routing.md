# Account connections

[← Pairing](pairing.md) · [Contents](../README.md) · [Input control →](input-control.md)

Signing in registers the computer's existing X25519 identity. A fresh, one-use server challenge verifies its private key without uploading the key. Other verified devices in the account appear automatically, without exchanging pairing codes.

Account membership authorizes a pinned encrypted handshake, not input control. The receiver still needs Allow connections enabled, OS permissions, and local approval of the first account control request. That approval lasts while account membership remains fresh; it is not saved as a manual pairing. Signing out, removing a device, changing MFA settings, or losing membership ends account access. Manual pairings remain independent.

Discovery uses IPv4 sockets to match the IPv4 listeners. Nearby connections are preferred. If a local endpoint cannot be reached, the app opens an authenticated WebSocket tunnel through the configured account server. Presence and control use the same existing Noise protocol and pin the peer fingerprint; relay routing never substitutes for identity verification.

The hosted service uses an account-specific SQLite Durable Object. Self-hosting uses the equivalent Axum WebSocket routes. Both reject unverified device sessions, isolate accounts, bound simultaneous tunnels and message size, expire abandoned offers, and recheck session revocation. No private keys or decrypted input reach the account server. Direct internet NAT traversal is future work; internet traffic currently uses the relay.

## Verification

Run the ordinary Rust suites. For real transport checks, start a disposable local Worker or standalone server and run:

```sh
EXTEND_RELAY_TEST_URL=http://localhost:3010 \
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  encrypted_presence_cross_account_isolation_and_revocation -- --ignored
```

The test creates disposable accounts. Set `EXTEND_RELAY_TEST_CLEANUP` to a temporary file to record their exact IDs for cleanup. It checks wrong-key proof rejection, one-use proofs, pinned encrypted presence and actual desktop control using fake input helpers, wrong peer identity, cross-account rejection, and logout revocation. The desktop account-control regression separately verifies denial, explicit local consent, and active-session termination on sign-out.

---

[← Pairing](pairing.md) · [Contents](../README.md) · [Input control →](input-control.md)
