#!/bin/bash
# Drives a Windows test PC over SSH from this checkout; see docs/development/windows.md.
#
#   sync                     mirror HEAD and uncommitted changes to the PC's checkout
#   run [seconds]            sync, build the debug app and helpers, relaunch the app, print its log
#   shot [file.png]          screenshot the PC's desktop (default /tmp/extend-dev-shot.png)
#   ui Action=... [Key=...]  UI Automation in the desktop session, elevated; see ui.ps1
#   display-test [software]  run the display helper for ~5 s while sweeping the pointer, summarize its stream
#   input-test [drive]       run capture-control-left against injected input; drive only injects it
#   viewer-test [W H]        sync, build the display helper, run its viewer full screen on a synthetic stream
#   driver-build             sync, then compile+sign the display driver and build the acceptance tool
#   driver-install           trust the dev cert, install or update the driver, prune old packages and certs (elevated)
#   driver-remove            remove the devnode, delete the packages and the dev certs (elevated)
#   driver-accept [args]     run the acceptance tool in the signed-in session (non-elevated), with its arguments
#                            (none: the full test; churn|spaced [N [MIN MAX]], cycles [N], sustain [S], state); prints
#                            the driver host's handle count before and after; exits 1 on FAIL or if the driver host
#                            restarted meanwhile
#   driver-ssh [args]        the same over plain SSH: session 0, elevated, a network logon
#
# EXTEND_WINDOWS_HOST (default windows-pc) names the SSH host, whose default shell must be Windows
# PowerShell, and EXTEND_WINDOWS_REPO (default C:\src\extend.computer) the checkout there.
set -euo pipefail
cd "$(dirname "$0")/../.."
host=${EXTEND_WINDOWS_HOST:-windows-pc}
repo=${EXTEND_WINDOWS_REPO:-'C:\src\extend.computer'}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/extend-dev.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
ok='if ($LASTEXITCODE) { exit 1 }'

# A PowerShell string literal.
quote() { local s=${1//\'/\'\'}; printf "'%s'" "$s"; }
# A Windows path as scp expects it.
slashes() { printf '%s' "${1//\\//}"; }
remote() { ssh "$host" "$1" | LC_ALL=C tr -d '\r'; }

# Creates the PC's work directory, %TEMP%\extend-dev, and finds Cargo's target directory there.
prepare() {
    local paths
    paths=$(remote "(New-Item -ItemType Directory -Force (Join-Path \$env:TEMP extend-dev)).FullName; if (\$env:CARGO_TARGET_DIR) { \$env:CARGO_TARGET_DIR } else { Join-Path $(quote "$repo") target }")
    { read -r work; read -r target; } <<<"$paths"
}

# Uploads the PowerShell helpers and runs $1.ps1 in the desktop session with in-session.ps1 flags $2;
# the remaining key=value arguments become the script's parameters.
session() {
    local script=$1 flags=$2
    shift 2
    printf '%s\n' "$@" >"$tmp/$script.args"
    scp -q scripts/windows/*.ps1 "$tmp/$script.args" "$host:$(slashes "$work")/"
    remote "powershell -NoProfile -ExecutionPolicy Bypass -File $(quote "$work\\in-session.ps1") $script $flags"
}

# Resets the PC's checkout to HEAD and applies the working tree's changes, including untracked files.
sync() {
    local head git_pc changed
    head=$(git rev-parse HEAD)
    git_pc="git -C $(quote "$repo") -c core.autocrlf=false"
    if ! ssh "$host" "if (-not (Test-Path $(quote "$repo\\.git"))) { git init -q $(quote "$repo") }; $git_pc cat-file -e '$head^{commit}'" 2>/dev/null; then
        git bundle create -q "$tmp/sync.bundle" HEAD
        scp -q "$tmp/sync.bundle" "$host:$(slashes "$repo")/.git/extend-dev-sync.bundle"
        remote "$git_pc fetch -q .git/extend-dev-sync.bundle HEAD; Remove-Item $(quote "$repo\\.git\\extend-dev-sync.bundle")" >/dev/null
    fi
    # A temporary index leaves the real one untouched.
    (
        export GIT_INDEX_FILE="$tmp/index"
        git read-tree HEAD
        git add -A -- . ':(exclude,glob)**/target/**' ':(exclude,glob)**/node_modules/**'
        git diff-index --cached --binary -p HEAD
    ) >"$tmp/sync.patch"
    scp -q "$tmp/sync.patch" "$host:$(slashes "$repo")/.git/extend-dev-sync.patch"
    changed=$(remote "$git_pc reset -q --hard $head; $ok; $git_pc clean -qfd -e target -e node_modules; \
\$patch = $(quote "$repo\\.git\\extend-dev-sync.patch"); \
if ((Get-Item \$patch).Length) { $git_pc apply --whitespace=nowarn .git/extend-dev-sync.patch; $ok }; \
Remove-Item \$patch; @($git_pc status --short).Count")
    echo "synced $head + $changed changed"
}

# Builds the app and helpers into one directory, since the app looks for its helpers beside itself.
run() {
    sync
    prepare
    # Cargo's progress on stderr marks a pipeline failed in Windows PowerShell, which would make
    # powershell exit 1 after a good build; the exit code checks decide and the command exits 0.
    remote "Stop-Process -Name extend-computer-desktop -Force -ErrorAction SilentlyContinue; Start-Sleep 1; \
\$env:CARGO_TARGET_DIR = $(quote "$target"); Set-Location $(quote "$repo\\desktop"); \
npx.cmd tauri build --debug --no-bundle 2>&1 | Select-String '^error|Finished|Built application' -Context 0,8 | Out-String -Width 300; \
if (\$LASTEXITCODE) { \"app build failed (\$LASTEXITCODE)\"; exit 1 }; \
Set-Location ..\\native\\windows; cargo build 2>&1 | Select-String '^error|Finished' -Context 0,8 | Out-String -Width 300; \
if (\$LASTEXITCODE) { \"helper build failed (\$LASTEXITCODE)\"; exit 1 }; exit 0"
    session app -NoWait "Bin=$target\\debug"
    remote "Start-Sleep ${1:-10}; \
Get-Process extend-computer-desktop -ErrorAction SilentlyContinue | Select-Object Id, SessionId, MainWindowTitle | Format-Table -AutoSize | Out-String; \
Get-Content $(quote "$work\\app.log") -Tail 40 -ErrorAction SilentlyContinue; exit 0"
}

shot() {
    local out=${1:-/tmp/extend-dev-shot.png}
    prepare
    session shot ''
    scp -q "$host:$(slashes "$work")/shot.png" "$out"
    echo "$out"
}

ui() {
    prepare
    session ui '-Elevated -Timeout 30' "$@"
}

display_test() {
    prepare
    session display-test '-Timeout 30' "Bin=$target\\debug" "Encoder=${1:-}"
    scp -q "$host:$(slashes "$work")/display.bin" /tmp/extend-dev-display.bin
    python3 - /tmp/extend-dev-display.bin /tmp/extend-dev-display.h264 <<'PY'
import struct, sys
data = open(sys.argv[1], 'rb').read()
frames, i = [], 0
with open(sys.argv[2], 'wb') as video:
    while i + 5 <= len(data):
        kind, size = data[i], struct.unpack('>I', data[i + 1:i + 5])[0]
        payload = data[i + 5:i + 5 + size]
        i += 5 + size
        if kind == 1:
            frames.append((struct.unpack('>Q', payload[:8])[0], payload[8], len(payload) - 9))
            video.write(payload[9:])
        else:
            print('kind', kind, payload[:120])
if len(frames) > 1:
    span = (frames[-1][0] - frames[0][0]) / 1e9
    print(f'{len(frames)} frames in {span:.2f}s = {len(frames) / span:.1f} fps; '
          f'keyframes {sum(f[1] for f in frames)}; avg {sum(f[2] for f in frames) // len(frames)} B')
print('stream:', sys.argv[2])
PY
}

input_test() {
    prepare
    session input-test '-Timeout 30' "Bin=$target\\debug" "Mode=${1:-capture}"
}

# The viewer opens full screen and the test clicks and types into it, so the PC's desktop is busy meanwhile.
viewer_test() {
    sync
    prepare
    remote "\$env:CARGO_TARGET_DIR = $(quote "$target"); Set-Location $(quote "$repo\\native\\windows"); \
cargo build -p extend-computer-display 2>&1 | Select-String '^error|Finished' -Context 0,8 | Out-String -Width 300; \
if (\$LASTEXITCODE) { \"helper build failed (\$LASTEXITCODE)\"; exit 1 }; exit 0"
    scp -q scripts/windows/viewer-test.py "$host:$(slashes "$work")/"
    session viewer-test '-Timeout 120' "Bin=$target\\debug" "Width=${1:-1920}" "Height=${2:-1080}"
}

# Runs scripts/windows/driver.ps1 from the PC's checkout over plain SSH (an admin
# ssh session is already elevated), for the build/sign/install/remove actions.
driver_ps1() {
    remote "powershell -NoProfile -ExecutionPolicy Bypass -File $(quote "$repo\\scripts\\windows\\driver.ps1") -Action $1; exit \$LASTEXITCODE"
}

driver_build() {
    sync
    prepare
    driver_ps1 build
    driver_ps1 sign
    # Build the Rust acceptance tool beside the helpers.
    remote "\$env:CARGO_TARGET_DIR = $(quote "$target"); Set-Location $(quote "$repo\\native\\windows"); \
cargo build -p extend-computer-driver-test 2>&1 | Select-String '^error|Finished' -Context 0,8 | Out-String -Width 300; \
if (\$LASTEXITCODE) { \"acceptance-tool build failed (\$LASTEXITCODE)\"; exit 1 }; exit 0"
}

driver_install() { driver_ps1 install; }
driver_remove() { driver_ps1 remove; }

# "<pid> <handle count>" of the WUDFHost that hosts the driver (its command line names the INF's device
# group), readable over the admin SSH session.
driver_host() {
    remote "@(Get-CimInstance Win32_Process -Filter \"Name='WUDFHost.exe'\" | \
Where-Object CommandLine -match 'ExtendComputerDisplayGroup' | ForEach-Object { \"\$(\$_.ProcessId) \$(\$_.HandleCount)\" }) -join ','"
}

# Runs the given command (an acceptance-tool run), then reports whether the driver host ran throughout
# and how its handle count moved.
with_host_check() {
    local before after status=0
    before=$(driver_host)
    "$@" || status=$?
    # A host crash ends that process; UMDF restarts the device in a new one.
    after=$(driver_host)
    echo "driver host WUDFHost (pid handles) before: ${before:-none}, after: ${after:-none}"
    if [ -z "$before" ] || [ "${before%% *}" != "${after%% *}" ]; then
        echo "FAIL: the driver host did not run throughout" >&2
        status=1
    fi
    return "$status"
}

driver_accept() {
    prepare
    # Non-elevated interactive session, like the display helper: the token
    # carries INTERACTIVE for the device SDDL, and its logon SID gets the textures.
    with_host_check session driver '-Timeout 900' "Action=accept" \
        "Bin=$target\\debug\\extend-computer-driver-test.exe" "Arguments=$*"
}

driver_ssh() {
    prepare
    with_host_check remote "& $(quote "$target\\debug\\extend-computer-driver-test.exe") $*; exit \$LASTEXITCODE"
}

command=${1:-}
case $command in
    sync | run | shot | ui | display-test | input-test | viewer-test | driver-build | driver-install | driver-remove | driver-accept | driver-ssh)
        shift
        "${command//-/_}" "$@"
        ;;
    *)
        echo "usage: $0 <command>" >&2
        sed -n 's/^#   /  /p' scripts/windows/remote.sh >&2
        exit 2
        ;;
esac
