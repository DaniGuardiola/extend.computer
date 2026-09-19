> 🤖🔧 ai generated

# extend.computer cursor-session low-jitter integration

Implemented and tested on both Macs. After cursor authorization, each endpoint checks the actual route to its authenticated peer. If that route uses a recognized Wi-Fi hardware interface and the helper is available, extend.computer acquires a temporary AWDL lease. Ethernet, VPN, loopback, and unknown routes skip it. `--awdl` opts out on that Mac.

Missing or incompatible helpers produce a warning and preserve ordinary networking. A live fallback test with the older installed helper completed 1,535 encrypted updates without changing AWDL.

The upgraded native broker emits readiness only after lease acquisition. extend.computer holds its stdin pipe open for the cursor session; normal completion or parent death closes the pipe and releases the lease. Receiver resources also release on session errors and revocation. The helper retains a ten-second renewal deadline and the prototype broker's thirty-second maximum.

## Measured two-Mac results

Independent fifteen-second trials used the real encrypted cursor protocol with generated positions and a dummy receiver. These measure acknowledgment waits, not visible cursor latency.

| Mode | Cursor updates | Median ACK wait | p95 | Worst | Waits above 40 ms |
|---|---:|---:|---:|---:|---:|
| Automatic session leases | 2,165 | 6.58 ms | 9.64 ms | 15.23 ms | 0 |
| Opt-out | 1,381 | 9.73 ms | 94.98 ms | 101.70 ms | 57 |

Mid-session checks confirmed one lease on each Mac and both AWDL interfaces down only in automatic mode. Opt-out kept zero leases and both interfaces up. Both trials ended with zero leases, no pending restoration, and AWDL up on both Macs.

## Cleanup and validation

- Twenty-six Rust tests passed on both Macs; local Clippy passed with warnings denied.
- Cursor tests now verify cleanup before the caller drops its sink, including consent denial, revocation, invalid protocol use, and normal close.
- Direct broker pipe-close and parent-SIGKILL checks restored AWDL on both Macs in approximately 0.27–0.29 seconds, including verification overhead.
- Killing the actual synthetic Rust sender mid-session released both endpoint leases and restored both interfaces.
- An unauthorized executable remained rejected after the local helper upgrade.
- Both helper upgrades completed the existing recovery/uninstall path before reinstalling the new exact-hash-pinned executable.

A diagnostic setup failure was reproduced and fixed: the nonblocking listener's accepted socket inherited nonblocking behavior on macOS. The synthetic receiver now restores blocking mode before entering the framed protocol. The corrected two-Mac test passed.

## Remaining limits

This is still a thirty-second, main-display cursor-only prototype. Real visual smoothness needs a human check; clicks, scrolling, keyboard, screen sharing, extended desktop, and the settings UI are not implemented. Route eligibility is checked at session start; it is not continuously re-evaluated during this short prototype session. AirDrop/Continuity may be affected while AWDL is paused.

Root-daemon forced-crash and power-loss recovery have not been tested end to end. The helper remains a development installation with an explicit diagnostic CLI and per-build hash pins, not a production distribution/update system or completed security audit.
