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

---

[← Desktop development](desktop.md) · [Index](../README.md)
