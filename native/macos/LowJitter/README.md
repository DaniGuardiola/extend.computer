> 🤖🔧 ai generated

# extend.computer low-jitter development helper

This development helper temporarily pauses AWDL under short renewable leases. The intended product setting is enabled by default for eligible Mac Wi-Fi sessions, with an opt-out. Automatic cursor-session integration is implemented and verified. The development build also exposes an explicit 30-second diagnostic. See [current session results](../../../docs/session-low-jitter.md).

## Installation scope

The installer creates a root LaunchDaemon, a root-owned executable and configuration under `/Library/PrivilegedHelperTools/computer.extend.lowjitter-development`, and a private recovery directory under `/var/db/computer.extend.lowjitter-development`. Launchd keeps the helper running so it can recover an interrupted lease after a crash. Installing the service does not itself request a lease or disable AWDL.

Connections must come from the installing user and satisfy the exact installed executable's code-directory hash. Both client and helper check the signature requirement. The development executable uses ad-hoc signing; the local authentication smoke test accepted its matching hash and rejected a wrong one. This does not require a paid Apple signing identity. Different builds require reinstalling; it is not yet a production distribution/update system.

The XPC API contains only begin, renew, end, and status. Callers cannot supply shell commands, executable paths, interface names, or journal paths. A lease expires after ten seconds without renewal; a one-second timer processes expiry. Connection loss releases that connection's leases. Other concurrent leases keep the pause active. AWDL originally down is left down.

The helper records a restoration obligation before attempting to lower AWDL. A failed restore retains that obligation and retries; startup restores an interrupted prior lease before serving requests. Corrupt or unsafe journal files prevent startup. Interface changes use fixed `/sbin/ifconfig awdl0 up/down` arguments with a bounded wait.

## Verification and limits

Fourteen lifecycle/journal scenarios passed locally on Apple Silicon: overlapping sessions, original-down state, renewal and expiry, wrong owner, disconnect, restart, failed restore, failed journal writes/clear, partial interface-change failure, clock/capacity checks, long sleep, real file round trips, and symlink/corruption rejection. The installed root adapter has been exercised on both Macs. The signed native arm64 helper builds on both Macs. All fourteen tests pass on both. Installation, authorized-client operation, unauthorized-client rejection, and real lease cleanup checks passed on both Macs.

The journal tests do not simulate sudden machine power loss. Installed-service checks accepted authorized clients and rejected separate executables on both Macs. Pipe-bound session integration and client-crash cleanup passed with real AWDL. The recovery/uninstall path was exercised during both upgrades. Root-daemon forced-crash and power-loss recovery still need end-to-end testing. No claim of a completed security audit.

While paused, AWDL-dependent features such as AirDrop and some Continuity features can be affected. extend.computer cannot infer which other app changed an interface flag; it does not repeatedly force AWDL down. If the helper cannot restart or restore because of an OS/filesystem failure, automatic recovery cannot be guaranteed.

Apple documents [code-signing requirements](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html) and [XPC listener signature requirements](https://developer.apple.com/documentation/foundation/nsxpclistener/setconnectioncodesigningrequirement(_:)).

## Development commands

- `sh test.sh` runs standalone tests with Command Line Tools.
- `sh build-development.sh` builds the native executable, ad-hoc signs it with hardened runtime, and generates an installer pinned to its SHA-256.
- `sudo /usr/bin/python3 .build/development/install.py` installs the prepared build.
- `/Library/PrivilegedHelperTools/computer.extend.lowjitter-development/ExtendComputerLowJitter status` queries the service without changing AWDL.
- The same executable with `test-30s` temporarily acquires a lease; run as the installing user, not root.
- `sudo /usr/bin/python3 .build/development/install.py uninstall` stops the helper, runs recovery, then removes known installed files. Recovery failures preserve the journal.

The native system adapter holds an exclusive lock so startup/manual recovery cannot concurrently mutate the journal. Use one serialized executor for each controller. Standalone tests avoid the local Swift Package Manager manifest-link failure and the receiver's missing XCTest framework.

## Installed local verification

The local installed service accepted its authorized client and rejected a separate executable. With real AWDL, killing the client restored AWDL in about 0.04 seconds; suspending client renewal restored it after 10.72 seconds. Both ended with zero leases, no pending journal obligation, and AWDL up. Receiver installation and two-Mac helper-controlled measurements passed. The root daemon itself has not been force-crashed. Recovery/uninstall completed as part of both helper upgrades.

## Pipe-bound session upgrade

The new `lease` mode emits `READY` after acquisition and renews while stdin stays open, for at most 30 seconds. EOF, unexpected input, pipe errors, process death, or expiry release the lease. extend.computer keeps that pipe open only for its authorized cursor session. This mode replaces use of a separately timed diagnostic process for application integration.

Use the prepared `sudo /usr/bin/python3 .build/development/install.py upgrade` command to restore/stop the old helper and install the new pinned build. No per-session sudo prompt is needed afterward. `--awdl` is a global extend.computer CLI opt-out. Low-jitter settings remain local to each Mac; each endpoint decides whether its own routed interface is Wi-Fi.


## GUI permissions integration (2026-09-18)

The GUI now requires this helper for eligible Wi-Fi keyboard/mouse sessions and exposes installation/repair as Wi-Fi optimization in Permissions. GUI fallback to ordinary Wi-Fi is no longer allowed. `status` includes `protocol=2`; `session` holds a pipe-bound lease until EOF while renewing off the input thread. The older `lease` and `test-30s` diagnostic commands retain their 30-second bound. Renewal rejects an externally reactivated AWDL interface, causing required sessions to end. No periodic broker replacement is needed. The GUI now bundles an app-owned LaunchDaemon and uses native SMAppService registration. Development and production have separate service identities; the manual installer above is only for legacy standalone diagnostics. See [native permission setup](../../../docs/wifi-permissions.md).
