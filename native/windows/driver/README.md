# extend.computer Windows indirect display driver (v0)

A UMDF 2 / IddCx indirect display driver that lets extend.computer create a
virtual monitor on Windows on demand and read its frames. It is the first of the
Windows-extend components; see [the Windows development
guide](../../../docs/development/windows.md) for how it fits in and how to build,
sign, install and test it.

## What it does

- The adapter is a single root-enumerated devnode (`Root\ExtendComputerDisplay`)
  created once by the elevated install step. It starts with **zero monitors**.
- A client in the signed-in user's session opens the device interface
  `{E053DB0E-1431-463B-ABDC-0CB768D6E649}` and sends IOCTLs to create and remove
  monitors (up to four), each with its own preferred mode, fallback modes,
  physical size and EDID identity.
- Each monitor is **owned by the file handle that created it**. Only that
  handle can query or remove it, and when the handle closes (crash, kill or
  normal exit) the driver departs the monitor, so a dead helper never leaves a
  stray monitor behind.
- The swap-chain processor copies every desktop frame into a per-monitor
  keyed-mutex shared D3D11 texture and counts it, so a client reads frames
  without Desktop Duplication (the Duet handoff pattern).

## Derivation and license

`Driver.cpp`, `Driver.h` and `ExtendComputerDisplay.rc` are derived from
Microsoft's IddSampleDriver in the Windows-driver-samples repository
(`video/IndirectDisplay/IddSampleDriver`, commit `f4c1ff8`), which is licensed
under the **Microsoft Public License (MS-PL)**. The full MS-PL text is in
`LICENSE-MS-PL`, and the Microsoft copyright notice is retained at the top of the
derived files, as MS-PL section 3(C)/(D) requires. `Protocol.h` and `Edid.cpp`
are original extend.computer code.

## Control contract (protocol v2)

The single source of truth is `Protocol.h`; the Rust client in
`native/windows/shared/display_driver.rs` mirrors it and asserts every struct
size. Summary:

- Interface class GUID: `{E053DB0E-1431-463B-ABDC-0CB768D6E649}`. Open it with
  `GENERIC_READ|GENERIC_WRITE` and `SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION`.
- IOCTLs are `FILE_DEVICE_UNKNOWN` (0x22), `METHOD_BUFFERED`, functions 0x800+,
  access `FILE_READ_DATA|FILE_WRITE_DATA`. Every request checks its exact input
  length (0 for `GET_INFO`) and minimum output length before touching buffers.

  | IOCTL | Code | In | Out |
  | --- | --- | --- | --- |
  | `IOCTL_EXTEND_GET_INFO` | `0x0022E000` | – (0 bytes) | `EXTEND_INFO` (28) |
  | `IOCTL_EXTEND_ADD_MONITOR` | `0x0022E004` | `EXTEND_ADD_MONITOR_IN` (132) | `EXTEND_ADD_MONITOR_OUT` (24) |
  | `IOCTL_EXTEND_REMOVE_MONITOR` | `0x0022E008` | `EXTEND_REMOVE_MONITOR_IN` (8) | – |
  | `IOCTL_EXTEND_QUERY_MONITOR` | `0x0022E00C` | `EXTEND_QUERY_MONITOR_IN` (8) | `EXTEND_QUERY_MONITOR_OUT` (172) |

- `GET_INFO` returns the protocol version, the package's DriverVer
  (`major<<16|minor`, `build<<16|revision`), the IddCx version and the monitor
  counts.
- `ADD_MONITOR` takes the preferred mode, up to eight fallback modes (each
  320..8192 pixels per side, 1..1000 Hz; any invalid one fails the request), the
  physical size in mm (0 picks ~96 DPI) and a `SerialSeed`. It returns the
  monitor id, the arrival `OsAdapterLuid` and `OsTargetId` (the monitor's target
  in `QueryDisplayConfig`) and the connector index. A seed already used by an
  active monitor fails with `STATUS_OBJECT_NAME_COLLISION`; a fifth monitor with
  `STATUS_INSUFFICIENT_RESOURCES`.
- `QUERY_MONITOR` returns flags (`PRESENT`/`SURFACE`/`FRAME`), the
  `SurfaceGeneration`, the render-adapter LUID, the texture width, height and
  `DXGI_FORMAT`, a 64-bit frame counter and the NUL-terminated texture name for
  `OpenSharedResourceByName`. The render LUID and name are only known once
  Windows assigns a swap chain, so the client polls until `SURFACE`.
- `REMOVE_MONITOR` and `QUERY_MONITOR` from any handle but the creator's fail
  with `STATUS_ACCESS_DENIED`.

The Win32 error each status surfaces as is listed at the top of `Protocol.h`.

## Modes and identity

The driver offers Windows exactly the client's modes: the requested mode is the
preferred one, followed by the fallbacks, from both `ParseMonitorDescription`
and `QueryTargetModes`. The list sits in the monitor's context before
`IddCxMonitorArrival`, because IddCx queries target modes during arrival, so any
valid size activates as requested (tablet sizes such as 1366x1024, 2388x1668
and 2732x2048 included).

Each monitor gets a synthesised 128-byte EDID 1.4 block (`Edid.cpp`) with our
own PNP manufacturer id **XEC** (absent from the registered-PNP-id list, so not
a real vendor), product 0x0001, the client's `SerialSeed` as the serial number,
the requested mode as the preferred detailed timing (with CVT reduced-blanking
timings; modes past a DTD's 4095-pixel or 655 MHz limits are described at a
fraction of their size or refresh), the physical size, the name `extend`, and a
dummy descriptor (no range-limits descriptor: the display is not
continuous-frequency). The monitor's container id is derived from the seed too.

Windows keys its display-configuration database (arrangement, primary, scale)
on the EDID identity, so a client that sends a stable seed per device, such as
a hash of the peer's device id, gets that device's layout back every session,
and the database does not grow with every connection. Send the real physical
size: with the default ~96 DPI size Windows picks 100% scale for ordinary sizes
but treats very large ones (2732x2048, about 35") like a TV and scales them up.

## Frame handoff

Per monitor, on the first frame of a swap chain the processor creates one D3D11
texture on the swap chain's render adapter with
`D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX | SHARED_NTHANDLE` and publishes it under
a name unique to that generation,
`Global\ExtendComputerDisplay-<monitor>-<generation>-<nonce>`. Ownership
protocol (Duet):

- driver: `AcquireSync(0, 0ms)` (or `AcquireSync(1, 0ms)` to replace a frame the
  client has not picked up yet), `CopyResource`, `ReleaseSync(1)`; it never
  waits on the client;
- client: `AcquireSync(1, timeout)`, read (copy out), `ReleaseSync(0)`.

`AcquireSync` returns `S_OK` for a frame, `WAIT_TIMEOUT` (0x102) when none came
and `WAIT_ABANDONED` (0x80) when the texture is dead. The `windows` Rust crate
folds the last two into `Ok`, so the shared client calls the vtable directly.

- **Newest frame kept.** While the client holds the texture, new frames go to
  a driver-private copy; the processor publishes it within 16 ms of the release,
  even if the desktop does not change again.
- **Generations.** A swap-chain reassign (mode or topology change, monitor
  off/on, TDR), a change of surface size or format, or an abandoned keyed mutex
  makes the driver replace the texture: `SurfaceGeneration` goes up and the name
  changes. When it can, the driver takes the old texture's mutex before dropping
  it, so a client still holding the old texture gets `WAIT_ABANDONED`; a client
  that was reading at that moment only sees timeouts from then on. Either way
  the client re-queries, sees the new generation and reopens by name.
- **Abandoned mutex.** If a client's device dies holding key 1 (crash, device
  reset), the driver sees `WAIT_ABANDONED` on its next frame and publishes a new
  generation, including that frame, instead of going silent.
- IddCx presents only when the desktop changes, so an idle monitor times out;
  that is not an error.
- Windows draws nothing on any monitor while its display is powered off or the
  session is locked, so no surface is assigned and no frame arrives. A client
  that extends should hold `ES_DISPLAY_REQUIRED` (`SetThreadExecutionState`)
  for as long as it needs frames.

## Monitor teardown

A monitor departs on `REMOVE`, when its handle closes, or when its handle
closed while its `ADD` was still running. Every departure goes through
`IddCxMonitorDeparture`, which runs IddCx's own unassign path (the driver's
`EvtIddCxMonitorUnassignSwapChain`: the processor stops, it checks for that on
every frame, and deletes its swap chain as it exits, as the IddCx docs and the
sample have it) and then deletes the monitor object, whose cleanup frees the
driver's context. The driver never stops or deletes the swap chain ahead of the
departure: dropping the swap chain of a monitor that is still present makes
Windows assign it a new one at once, and that assignment races the departure.

What makes this safe is ordering. IddCx (`IddCx.dll` 10.0.26100.4202, checked
by disassembly) receives the kernel's messages, among them "here is a new swap
chain" (`IddMonitor::HandleNewSwapChain`, which calls the assign callback), as
IOCTLs on one sequential, non-power-managed queue, the same queue that delivers
the driver's own IOCTLs, and `IddCxMonitorDeparture` calls that same
`HandleNewSwapChain` (with no swap chain) itself. A departure that overlaps a
message being handled for the same monitor corrupts IddCx's monitor state. So:

- `REMOVE`, and an `ADD` whose handle closed while it ran, depart from within
  their IOCTL, on that queue, so nothing else runs meanwhile;
- a closing handle's cleanup does not come through the queue, so the driver
  holds the queue stopped (`WdfIoQueueStopSynchronously` on the queue its
  IOCTLs arrive on, which first waits for the request in progress) while it
  departs that handle's monitors, then starts it again; queued messages for a
  departed monitor are then handled by IddCx as for any departed monitor;
- a departing monitor's connector and serial seed stay taken until its
  departure completes, and if IddCx refuses a departure the slot stays taken
  for good (`GET_INFO` keeps counting it) rather than being reused;
- IddCx calls the assign callback holding its monitor lock, which a swap
  chain's deletion also takes. A processor whose swap chain failed deletes it
  itself (which makes the OS assign a new one), so the assign callback never
  joins a processor that is still running: it retires it and joins it at the
  next unassign or with the monitor. A retired processor only withdraws the
  published surface if no newer generation replaced it.
- the monitor callbacks check for a monitor whose context is gone and then
  touch nothing (assign returns `STATUS_GRAPHICS_INDIRECT_DISPLAY_ABANDON_SWAPCHAIN`,
  the one failure IddCx does not turn into a host bugcheck).

The earlier design, which stopped the processor and deleted the swap chain
before `IddCxMonitorDeparture`, crashed the driver host when a handle closed
100-300 ms after `ADD`, just as Windows assigns the new monitor its first swap
chains: an access violation in `HandleNewSwapChain`, or IddCx's "driver did not
release a frame" watchdog bugcheck. A single watchdog bugcheck leaves the device
at problem 43 (`CM_PROB_FAILED_POST_START`) until it is reinstalled. The
acceptance test's spaced phase reproduces it (it failed at the 15th close on
that build) and passes on this one; see `docs/development/windows.md`.

## Driver host handles

The driver releases everything it holds per monitor and per swap chain (its
threads, events, textures, shared handles and render devices). Handle tracing
(`!htrace`) of the driver host over add-and-remove cycles finds no handle the
driver opened left open, and breakpoints on the swap-chain destructors show
both DXGI's `CDXGIIndirectSwapChain` and IddCx's swap-chain object destroyed
for every swap chain. Windows itself still keeps some handles, on build 26200:
the ones DXGI opens when IddCx opens a swap chain for `IddCxSwapChainSetDevice`
(`dxgi!CDXGIIndirectSwapChain::Open`, `NtGdiDdDDIOpenSwapChain`), the swap-chain
handle the compositor's DXGI puts into the host
(`NtGdiDdDDISetDodIndirectSwapchain`), and, on NVIDIA GPUs, a file handle each
time the user-mode driver loads. Windows assigns each new monitor two swap
chains, so the driver host's handle count grows by about four per monitor added
and removed (20 cycles on the test PC: 4.1 per cycle), the same with the swap
chain deleted by IddCx instead of the driver and with the render device kept
across swap chains. The acceptance test's `cycles` check allows up to eight
per cycle, which still catches a leak of the driver's own (a render device is
about 80 handles, a shared texture about 20).

## Security

**Device interface.** Set in the INF `.hw` section:
`D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)` plus `FILE_DEVICE_SECURE_OPEN`
(`DeviceCharacteristics` 0x100). SYSTEM and local Administrators get full
control; an interactive logon gets `GENERIC_READ|WRITE`, enough for the IOCTLs.
Non-admin service, network and batch logons are denied. That DACL alone lets a
Low-integrity process of the signed-in user in: the device object carries no
mandatory label, and PnP drops one written into the INF's SDDL (the device's
`DEVPKEY_Device_SecuritySDS` reads back with an empty `S:`). So the driver
checks every open itself (`EvtDeviceFileCreate`): it reads the caller's token
at identification level and refuses anything below Medium integrity, and a
handle opened with anonymous QoS (which hides the token), with
`STATUS_ACCESS_DENIED`. Opens from kernel mode pass. Verified on the test PC:
a Low-integrity copy of the acceptance tool, started from the signed-in user's
session, gets `ERROR_ACCESS_DENIED` from `CreateFileW`, where the previous
build let it call `GET_INFO` and add a monitor. **An elevated administrator
over any logon, ssh included, can open the device.**

**Monitors.** Only the creating handle can query or remove a monitor, so no
other process gets a monitor's texture name from the driver. Another interactive user (fast user switching, RDP) can still open the
device, see the active-monitor count and create monitors of their own, up to
the four-monitor limit.

**Textures.** On `ADD_MONITOR` the driver reads the caller's **logon SID**
(`S-1-5-5-X-Y`; its user SID if the token has none) through
`WdfRequestImpersonate` at identification level (INF
`UmdfImpersonationLevel=Identification`) and creates that monitor's textures
with

```
D:P(A;;GA;;;SY)(A;;RC;;;OW)(A;;0x80000001;;;<logon SID>)S:(ML;;NWNR;;;ME)
```

SYSTEM keeps full control; the owner (WUDFHost's LOCAL SERVICE identity) only
gets `READ_CONTROL`, so other LOCAL SERVICE processes cannot rewrite the DACL;
the client's logon session gets exactly `DXGI_SHARED_RESOURCE_READ|WRITE`; and
the Medium label with no-read-up/no-write-up keeps low-integrity processes out.
Verified on the test PC: the creating session opens the texture, while the same
user over ssh (another logon session, elevated) gets `STATUS_ACCESS_DENIED` from
`D3DKMTOpenNtHandleFromName` and a low-integrity process in the same session is
refused; `OpenSharedResourceByName` reports a denial as `0x8876086A`. The
texture name is not a secret; the descriptor is the gate.

Remaining exposure: any process of the client's own logon session at Medium
integrity or above can open its textures (that is the helper's own trust
boundary), and SYSTEM or an administrator who takes ownership can open anything.

## Building, signing and installing

See `docs/development/windows.md` ("Driver development") and
`scripts/windows/driver.ps1`, driven by `scripts/windows/remote.sh`:
`driver-build`, `driver-install`, `driver-accept`, `driver-remove`. Development
uses a self-signed code-signing certificate valid for 14 days whose private key
is deleted as soon as signing ends, successful or not; install trusts only the
exported `.cer`, after checking that no store still holds its private key and
that it really signed the package. Release uses Microsoft attestation signing.
