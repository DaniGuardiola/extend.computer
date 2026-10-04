> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

> 🤖🔧 ai generated

# first prototype — verification

Implemented in this checkout; a separate checkout was used on the second Mac. Git initialized locally, no commit or remote created.

## Implemented

- Rust library and CLI with native macOS Keychain identity storage.
- Local mDNS discovery with unverified endpoint labels; explicit IPv4 LAN listener opt-in.
- Expiring, single-attempt pairing codes using SPAKE2 and Noise encrypted sessions.
- Explicit approval on both sides; separate session-only, remembered, and automatic diagnostic-probe grants.
- Pinned device identities for reconnect; live and future-session revocation checks.
- Bounded frames, absolute read deadlines, fail-closed trust storage and authenticated transport.
- Repeatable two-Mac smoke-test driver; no input, screen, clipboard, shell or file-transfer capabilities in the agent.

## Verified

- `cargo test --locked`: 15 tests passed on each Mac.
- `cargo clippy --all-targets --locked -- -D warnings`: passed locally.
- Successful real Wi-Fi discovery, pairing, 50 encrypted probes, remembered reconnect, active revocation and rejected reconnect after revocation.
- Latest 50-probe run: median 6.644 ms, p95 9.595 ms, maximum 10.595 ms. Debug build, diagnostic request/response, includes local processing and trust-store checks. Not input-to-photon latency or video latency; not a controlled comparison with Duet.
- Test cases cover wrong/expired/consumed codes, either side denying consent, unknown clients, changed peer identity, remembered-but-not-automatic grants, one-off expiry at disconnect, active revocation, invalid/truncated frames, corrupt trust state, encrypted replay/tampering, and slow-drip deadlines.
- Persistent Keychain identity survived separate launches locally. Remote interactive initialization succeeded; a temporary launch job in the logged-in desktop session loaded the exact user-confirmed identity. Plain SSH Keychain access remains rejected by macOS. Desktop-session job was removed afterward.
- Remote smoke-test listener stopped after each run. No always-on extend.computer service installed.

## Development access

SSH key-only authentication is working with the user-verified host key pinned in a dedicated known-hosts file. Test account is the user's existing account; access is for authorized extend.computer build/test work. SSH orchestrates tests; application probes travel directly over LAN.

Local executable build target is x86_64 running under Rosetta; remote build is native arm64. Both ran the same test suite. The local Rust toolchain is workspace-contained and did not modify shell startup files.

## Limits / next step

This is a diagnostic pairing/session prototype, not a security-audited remote-control application. No input sharing, display streaming, Tauri UI, lock-state lifecycle, same-session resume, remote relay or other-platform identity adapters yet. The PAKE/Noise composition needs independent review before enabling powerful capabilities. No claim of zero security flaws.

Next: native mac input adapter plus a desktop-session agent lifecycle, keeping capture/injection behind separately approved capabilities. Keep an early virtual-display/hardware-media feasibility experiment on the plan.
