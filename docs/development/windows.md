# Windows development

[← Desktop development](desktop.md) · [Index](../README.md)

> [!NOTE]
> Windows support is in development. These tools drive a Windows 11 test PC over SSH from a Mac checkout.

`scripts/windows/remote.sh` mirrors the local checkout to the PC, builds and relaunches the debug app there, and inspects its desktop. It uploads the PowerShell helpers beside it on each use, so the PC needs only SSH and the toolchain; `sync` creates the checkout.

## Test PC setup

Use an administrator account that stays signed in to the desktop. In an elevated PowerShell, enable the OpenSSH server with Windows PowerShell as its shell:

```powershell
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Set-Service sshd -StartupType Automatic; Start-Service sshd
New-ItemProperty HKLM:\SOFTWARE\OpenSSH -Name DefaultShell -Value C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe -PropertyType String -Force
```

The server reads administrators' keys from a shared file, not `~\.ssh\authorized_keys`. Add the Mac's public key and restrict the file:

```powershell
Add-Content C:\ProgramData\ssh\administrators_authorized_keys '<public key>'
icacls C:\ProgramData\ssh\administrators_authorized_keys /inheritance:r /grant '*S-1-5-32-544:F' /grant '*S-1-5-18:F'
```

On the Mac, name the PC `windows-pc` in `~/.ssh/config`, or set `EXTEND_WINDOWS_HOST`.

Install the toolchain, then open a new shell. Rustup installs the compiler pinned in `rust-toolchain.toml` on first use.

```powershell
winget install -e --id Git.Git
winget install -e --id Rustlang.Rustup
winget install -e --id OpenJS.NodeJS.LTS
winget install -e --id Python.Python.3.13
winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

For the display driver, also install the Windows SDK and the matching Windows Driver Kit. `driver.ps1` uses the newest kit with the IddCx 1.6 headers and finds Visual Studio with `vswhere`. Secure Boot can stay on: a self-signed UMDF driver loads once `driver-install` trusts its certificate.

```powershell
winget install -e --id Microsoft.WindowsSDK.10.0.26100
winget install -e --id Microsoft.WindowsWDK.10.0.26100
```

Share one Cargo output directory and keep debug builds small. The app looks for its helpers beside its executable, so `remote.sh run` builds both into this directory, or into `target` in the checkout when it is unset:

```powershell
[Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', 'C:\src\target', 'User')
[Environment]::SetEnvironmentVariable('CARGO_INCREMENTAL', '0', 'User')
[Environment]::SetEnvironmentVariable('CARGO_PROFILE_DEV_DEBUG', 'line-tables-only', 'User')
```

Keep the PC awake on AC power, and keep it signed in and unlocked:

```powershell
powercfg /change standby-timeout-ac 0
powercfg /change monitor-timeout-ac 0
```

Windows blocks PowerShell scripts by default, including npm's `npm.ps1` and `npx.ps1` shims. Call `npm.cmd` and `npx.cmd` instead of changing the execution policy; the tools run their own scripts with `-ExecutionPolicy Bypass`. After the first `remote.sh sync`, and whenever `package-lock.json` changes, run `npm.cmd ci` in the PC's checkout; `run` does not install packages.

## Desktop session

SSH commands run in session 0, which has no desktop: windows opened there never appear, and screenshots, UI Automation and input capture or injection fail. `remote.sh` runs that work as a scheduled task with an interactive principal, in the signed-in user's session. `ui` uses the highest run level so it can reach elevated windows.

The app's first launch, and the first launch from a new build directory, makes Windows ask whether to allow it through the firewall. UI Automation cannot see that prompt, so approve it by position: take a screenshot, find the Allow button, and click it. `ui` clicks from an elevated task, which the prompt requires.

```sh
bash scripts/windows/remote.sh shot /tmp/windows.png
bash scripts/windows/remote.sh ui Action=click X=830 Y=718
```

Window titles and button names follow the PC's display language, so name-based `ui` actions need the localized text.

## Usage

```sh
bash scripts/windows/remote.sh sync
bash scripts/windows/remote.sh run [seconds]
bash scripts/windows/remote.sh shot [file.png]
bash scripts/windows/remote.sh ui Action=list|invoke|click|type [Window=<title>] [Name=<element>] [X=<x> Y=<y>] [Text=<keys>]
bash scripts/windows/remote.sh display-test [software]
bash scripts/windows/remote.sh input-test [drive]
bash scripts/windows/remote.sh viewer-test [width height]
```

`EXTEND_WINDOWS_HOST` selects the SSH host (default `windows-pc`) and `EXTEND_WINDOWS_REPO` the PC's checkout (default `C:\src\extend.computer`).

`sync` resets the PC's checkout to the local `HEAD`, sending a Git bundle when the PC lacks that commit, removes untracked files except `target` and `node_modules`, and applies the local staged, unstaged and untracked changes. Ignored files are not copied. Do not edit the PC's checkout directly.

`run` syncs, stops the app, builds the debug app (`npx.cmd tauri build --debug --no-bundle`) and the `native/windows` helpers, relaunches the app in the desktop session, and prints the end of its log after 10 seconds or the given delay.

`shot` saves a screenshot of the whole desktop, by default to `/tmp/extend-dev-shot.png`. `ui` lists windows, or the named elements of windows whose title contains `Window`; invokes the element called `Name`; clicks at `X`,`Y` in physical pixels; or types `Text` in SendKeys syntax.

`display-test` runs the display helper in mirror mode for about 5 seconds while moving the pointer, then reports its frame rate, keyframes and frame size and saves the video to `/tmp/extend-dev-display.h264`; `software` selects the software encoder. `input-test` runs `capture-control-left` against injected pointer and key input and prints what the helper reports; `drive` only injects the input, to exercise the running app.

`viewer-test` syncs, builds the display helper and runs its viewer as the app would, on a synthetic stream from `ExtendComputerDisplay.exe self-test stream`, by default 1920x1080. The viewer covers the primary monitor for about 15 seconds while the test clicks, types and closes it, so leave the PC's desktop alone meanwhile. It checks readiness, the full-screen viewport, settings, render readiness and presented counts at 60 fps, a keyframe request after an unreadable frame, input packets (a printable key as text only, Ctrl+C as Command+C, Enter as a physical key), closing, and exit on `stop`, on the end of stdin and when its parent exits. `ExtendComputerDisplay.exe self-test` needs no desktop: it round-trips synthetic frames through the encoder and the viewer's hardware and software decoders.

On the PC, work files live in `%TEMP%\extend-dev` and scheduled tasks are named `extend-dev-*`.

## Driver development

`native/windows/driver` is the extend.computer indirect display driver (UMDF 2 / IddCx); its [README](../../native/windows/driver/README.md) covers the control contract, the frame handoff, monitor teardown, the driver host's handles, the security model and the MS-PL derivation. `native/windows/shared/display_driver.rs` is the Rust client for it. `scripts/windows/driver.ps1`, driven by `remote.sh`, builds, signs, installs, removes and acceptance-tests it on the PC, which needs the WDK (see [Test PC setup](#test-pc-setup)).

```sh
bash scripts/windows/remote.sh driver-build    # sync, compile and sign, build the acceptance tool
bash scripts/windows/remote.sh driver-install  # trust the dev cert, install or update in place, prune old packages and certs
bash scripts/windows/remote.sh driver-accept   # acceptance test in the desktop session; exits 1 on FAIL
bash scripts/windows/remote.sh driver-ssh cycles 20   # a mode of the acceptance tool over plain (elevated) SSH
bash scripts/windows/remote.sh driver-remove   # remove the devnode, the packages and the dev certs
```

- **Build** runs over plain SSH and needs no desktop. Without a WDK MSBuild toolset it compiles with `cl`, `link` and `rc`, stamps the INF with `stampinf` and catalogs it with `Inf2Cat`, into `%TEMP%\extend-driver`. It uses the newest Windows Kit that has the IddCx 1.6 headers and the Visual Studio that `vswhere` finds. `DriverVer` is `1.0.MMDD.<seconds-since-midnight / 2>`, unique and increasing for every build (even two in the same minute), so an update always replaces the installed package; the DLL's file version and `GET_INFO` report the same version.
- **Signing** makes a fresh self-signed code-signing certificate valid for 14 days, signs the DLL and the catalog with a timestamp, and deletes the private key in a `finally` block whether signing worked or not. Timestamping needs network access to `timestamp.digicert.com`; if the server is unreachable, signing fails with a clear message and leaves no key behind. Only a fully signed package gets the `dev.cer` that install trusts.
- **Install** runs in an elevated shell; a plain admin SSH session already is one. It refuses a certificate whose private key still exists in any store, or a DLL or catalog that certificate did not sign, then imports `dev.cer` into LocalMachine Root and TrustedPublisher. The first install stages the package and creates the root devnode with SetupAPI (`SetupDiCreateDeviceInfo`, `SPDRP_HARDWAREID`, `DIF_REGISTERDEVICE` and `UpdateDriverForPlugAndPlayDevices`, through PowerShell P/Invoke). An update runs `pnputil /add-driver /install`, which restarts the device on the new version by itself. Install confirms success from the code the device actually runs, not the installed package node: it opens the device and reads `GET_INFO`'s `DriverVer` (and checks the loaded DLL's hash), so it never claims success while the old driver is still loaded. If a client holds the device open, PnP vetoes the restart and the old driver keeps running; install reports this, names the holder, prunes nothing, and fails — close every client and run it again. If nothing holds the device but it still did not restart (Windows' reboot-pending flag, below, blocks `pnputil /restart-device`), install removes and re-creates the devnode on the staged package instead, which brings it onto the new driver with no reboot. It then deletes older ExtendComputerDisplay packages and removes dev certificates that no longer sign an installed package.
- **Reboot flag.** Setup re-registers the shared `WUDFRd` service and tries to restart the other UMDF display adapters. On the test PC, `duet.exe` holds Duet Display's adapter open and vetoes that restart (`CR_REMOVE_VETOED`), so Windows marks a reboot as pending after the first install and after every update. Install tells that veto (another device, harmless to ours) apart from a veto of our own device (a client holding it open), and prints each. Once the flag is set, `pnputil /restart-device` can no longer restart our devnode until a reboot, so install recovers a vetoed update by re-creating the devnode rather than restarting it. Our device starts on the new driver regardless, and nothing needs the reboot.
- **Acceptance** (`driver-accept`) runs `native/windows/driver-test` in the signed-in, non-elevated session, through the interactive scheduled task the other tests use, the way the display helper runs; it takes about two minutes. It checks `GET_INFO`, that malformed requests are refused, and that the device refuses an anonymous-QoS handle and a Low-integrity process of the same user at open (it starts a copy of itself with a Low-integrity token). It creates four monitors at sizes outside any common list (1366x1024, 1500x1000, 2388x1668, 2732x2048) and checks that each activates at exactly that size, that a duplicate seed and a fifth monitor are refused, and that another handle can neither query nor remove them. It paints colours in a window on a monitor and reads them back through the keyed mutex, counting frames only on `S_OK`, and checks that the newest frame survives the client holding the texture. An abandoned keyed mutex (a child process dies holding a frame) and a swap-chain reassign must each bump the surface generation, after which the client reopens and reads again. It removes one monitor, closes the handle with three left, and checks that the driver and the OS device list (`DISPLAY\XEC*`) are empty. It adds and departs six more monitors one after another, each with a swap chain and a frame, by `REMOVE` and by handle close in turn. Then it closes many handles right after `ADD`, across the window where the monitor arrives and Windows assigns its swap chains: back to back (synchronous ADDs, and overlapped ones closed before they complete), then spaced out, 20 closes 100-300 ms after `ADD`, alternating synchronous and overlapped, each followed by a 3 s pause and a check of the driver host (a long-lived handle's `GET_INFO`, the WUDFHost process and the crash signals below), since a host that dies in IddCx's watchdog does so seconds after the close; and it checks that nothing is left behind. The spaced phase is the one that catches the teardown race described in the driver README: on the build before the fix it failed at the 15th close with IddCx's watchdog bugcheck, which left the device at problem 43. It saves the display configuration first (`QDC_ALL_PATHS`), extends only if Windows does not activate the monitors by itself, and restores it at the end. It keeps the display awake for the run (`ES_DISPLAY_REQUIRED` and a mouse nudge), because Windows draws nothing on any monitor while its display is powered off or the session is locked, so no surface or frames would appear; the helper should likewise hold `ES_DISPLAY_REQUIRED` while it extends. The run fails on any sign of a driver-host crash since it started: a UMDF host-problem event (`Microsoft-Windows-DriverFrameworks-UserMode/Operational` 1009 or 4000; install turns that log on), a critical UMDF event in the System log (device offline, host terminated), a WER report for a WUDF host or a new `WudfHost_*.mdmp` in `C:\ProgramData\Microsoft\WDF`. `driver-accept` also reads the driver host's pid and handle count over the admin SSH session before and after, prints both, and fails if the pid changed.
- **Driver host handles.** The signed-in user cannot read the driver host's handle count, so the acceptance run only reports it through `driver-accept`; `driver-ssh cycles 20` checks it per cycle. It grows by about four handles per monitor added and removed, kept by Windows' own DXGI code for each swap chain, not by the driver (see "Driver host handles" in the driver README); the check allows eight per cycle.
- **Other modes.** Arguments after `driver-accept` (or `driver-ssh`, which runs the tool over plain SSH: session 0, elevated, a network logon) select another mode of the tool (`extend-computer-driver-test`): `monitor <width> <height> <seconds>` keeps one monitor alive for manual checks; `churn [<count> [<min-ms> <max-ms>]]` runs the handle-close checks alone, with `<count>` spaced closes; `spaced [<count> [<min-ms> <max-ms>]]` only the spaced phase, for a soak (say `spaced 40 50 500`); `cycles [<count>]` the add-and-depart cycles (default 20) with the handle count; `sustain [<seconds>]` uses one monitor normally (`ADD`, read frames for 30 s while painting, `REMOVE`; then `ADD` and close the handle after 10 s) and checks that the host ran throughout and nothing was left; `security` the open checks alone; and `state` prints `GET_INFO`, the `DISPLAY\XEC*` monitors, the active display paths and the driver host, changing nothing.
- **Remove** removes the devnode (`pnputil /remove-device`), deletes every package (`pnputil /delete-driver … /uninstall`) and removes the dev certificates.

The self-signed install is for development only; release uses Microsoft attestation signing. Never leave a machine-trusted certificate with its private key on the PC.

---

[← Desktop development](desktop.md) · [Index](../README.md)
