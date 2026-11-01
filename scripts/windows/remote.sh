#!/bin/bash
# Drives a Windows test PC over SSH from this checkout; see docs/development/windows.md.
#
#   sync                     mirror HEAD and uncommitted changes to the PC's checkout
#   run [seconds]            sync, build the debug app and helpers, relaunch the app, print its log
#   shot [file.png]          screenshot the PC's desktop (default /tmp/extend-dev-shot.png)
#   ui Action=... [Key=...]  UI Automation in the desktop session, elevated; see ui.ps1
#   display-test [extend] [software] [WxH [WxH]] [retina] [replace] [kill] [idle] [serial=N]
#                            run the display helper for ~5 s while sweeping the pointer, summarize its stream;
#                            extend adds the virtual monitor at the first size, switches to the second halfway
#                            (a mode it offers, or a new monitor with replace) and checks the cleanup; kill
#                            kills the helper instead of stopping it; idle parks the pointer for 10 s before the
#                            stop; serial=N sets EXTEND_DISPLAY_SERIAL. Exits 1 if any check failed
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
    local mode=mirror encoder= size= change= retina= replace= kill= idle= serial= status=0 arg
    for arg; do
        case $arg in
            extend | mirror) mode=$arg ;;
            software) encoder=software ;;
            retina) retina=1 ;;
            replace) replace=1 ;;
            kill) kill=1 ;;
            idle) idle=10 ;;
            serial=[0-9]*) serial=${arg#serial=} ;;
            [0-9]*x[0-9]*) if [ -z "$size" ]; then size=$arg; else change=$arg; fi ;;
            *) echo "display-test: unknown argument $arg" >&2; return 2 ;;
        esac
    done
    size=${size:-1920x1080}
    # The stream's sizes in pixels, as the helper encodes them: twice the points with Retina, even.
    local scale=1 sizes= wanted
    [ -z "$retina" ] || scale=2
    for wanted in $size $change; do
        sizes+="${sizes:+,}$(( ${wanted%x*} * scale & ~1 ))x$(( ${wanted#*x} * scale & ~1 ))"
    done
    prepare
    # A run that fails before it starts must not leave the last run's stream to summarize.
    remote "Remove-Item $(quote "$work\\display.bin") -ErrorAction SilentlyContinue"
    rm -f /tmp/extend-dev-display.bin
    # A failed check still shows the stream's summary; the exit status reports both.
    session display-test '-Timeout 90' "Bin=$target\\debug" "Encoder=$encoder" "Mode=$mode" \
        "Width=${size%x*}" "Height=${size#*x}" "Retina=$retina" "Change=$change" "Replace=$replace" "Kill=$kill" \
        "Idle=$idle" "Serial=$serial" || status=$?
    scp -q "$host:$(slashes "$work")/display.bin" /tmp/extend-dev-display.bin || status=1
    python3 - /tmp/extend-dev-display.bin /tmp/extend-dev-display.h264 "$sizes" <<'PY' || status=1
import struct, sys

def nals(data):
    """NAL units of an Annex B stream, without their start codes."""
    starts, i = [], 0
    while i + 3 <= len(data):
        if data[i:i + 3] == b'\0\0\1':
            starts.append(i + 3)
            i += 3
        else:
            i += 1
    return [data[a:(starts[n + 1] - 3 if n + 1 < len(starts) else len(data))].rstrip(b'\0') or b'\0'
            for n, a in enumerate(starts)]

class Bits:
    def __init__(self, nal):
        self.data, self.pos, zeros = bytearray(), 0, 0
        for b in nal:  # drop emulation prevention bytes
            if zeros >= 2 and b == 3:
                zeros = 0
                continue
            self.data.append(b)
            zeros = zeros + 1 if b == 0 else 0
    def u(self, n):
        v = 0
        for _ in range(n):
            v = v << 1 | (self.data[self.pos >> 3] >> (7 - (self.pos & 7)) & 1)
            self.pos += 1
        return v
    def ue(self):
        z = 0
        while self.u(1) == 0:
            z += 1
        return (1 << z) - 1 + self.u(z)
    def se(self):
        v = self.ue()
        return (v + 1) // 2 if v & 1 else -(v // 2)

def sps_size(nal):
    """The picture size an H.264 sequence parameter set describes."""
    b = Bits(nal[1:])
    profile = b.u(8)
    b.u(16)
    b.ue()
    chroma = 1
    if profile in (100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135):
        chroma = b.ue()
        if chroma == 3:
            b.u(1)
        b.ue(); b.ue(); b.u(1)
        if b.u(1):
            for i in range(12 if chroma == 3 else 8):
                if b.u(1):
                    last = scale = 8
                    for _ in range(16 if i < 6 else 64):
                        if scale:
                            scale = (last + b.se() + 256) % 256
                        last = scale or last
    b.ue()
    order = b.ue()
    if order == 0:
        b.ue()
    elif order == 1:
        b.u(1); b.se(); b.se()
        for _ in range(b.ue()):
            b.se()
    b.ue(); b.u(1)
    width, height = (b.ue() + 1) * 16, b.ue() + 1
    frames_only = b.u(1)
    if not frames_only:
        b.u(1)
    b.u(1)
    height *= 16 * (2 - frames_only)
    if b.u(1):
        left, right, top, bottom = b.ue(), b.ue(), b.ue(), b.ue()
        cx = 2 if chroma in (1, 2) else 1
        cy = (2 if chroma == 1 else 1) * (2 - frames_only)
        width -= cx * (left + right)
        height -= cy * (top + bottom)
    return width, height

data = open(sys.argv[1], 'rb').read()
frames, sizes, failures, i = [], [], [], 0
with open(sys.argv[2], 'wb') as video:
    while i + 5 <= len(data):
        kind, size = data[i], struct.unpack('>I', data[i + 1:i + 5])[0]
        payload = data[i + 5:i + 5 + size]
        i += 5 + size
        if kind == 1:
            frames.append((struct.unpack('>Q', payload[:8])[0], payload[8], len(payload) - 9))
            video.write(payload[9:])
            for nal in nals(payload[9:]) if payload[8] else []:
                if nal[0] & 31 == 7:
                    picture = sps_size(nal)
                    if not sizes or sizes[-1][0] != picture:
                        sizes.append((picture, len(frames) - 1))
        else:
            print('kind', kind, payload[:160])
            if kind == 2 and b'"ok":false' in payload.replace(b' ', b''):
                failures.append('a configuration failed')
if len(frames) > 1:
    span = (frames[-1][0] - frames[0][0]) / 1e9
    gap = max(b[0] - a[0] for a, b in zip(frames, frames[1:])) / 1e6
    print(f'{len(frames)} frames in {span:.2f}s = {len(frames) / span:.1f} fps; '
          f'keyframes {sum(f[1] for f in frames)}; avg {sum(f[2] for f in frames) // len(frames)} B; '
          f'longest gap {gap:.0f} ms')
if not frames:
    failures.append('no frames')
for n, (picture, first) in enumerate(sizes):
    last = sizes[n + 1][1] - 1 if n + 1 < len(sizes) else len(frames) - 1
    rate = ''
    if last > first:
        rate = f', {(last - first) / ((frames[last][0] - frames[first][0]) / 1e9):.1f} fps'
    print(f'frame size {picture[0]}x{picture[1]}: frames {first}-{last}{rate}')
expected = [tuple(map(int, wanted.split('x'))) for wanted in sys.argv[3].split(',') if wanted]
if frames and [picture for picture, _ in sizes] != expected:
    failures.append('frame sizes ' + ', '.join(f'{w}x{h}' for (w, h), _ in sizes) + ' instead of '
                    + ', '.join(f'{w}x{h}' for w, h in expected))
if sizes:
    # The pointer sweeps over the source after the last change, so Windows draws at about 40 fps; a stream that
    # repeats one picture has only the 500 ms idle refresh.
    times = [f[0] for f in frames[sizes[-1][1]:]]
    busiest, start = 0, 0
    for end, time in enumerate(times):
        while time - times[start] >= 1e9:
            start += 1
        busiest = max(busiest, end - start + 1)
    print(f'busiest second at {sizes[-1][0][0]}x{sizes[-1][0][1]}: {busiest} frames')
    if busiest < 10:
        failures.append(f'at most {busiest} frames in any second at the last size: the picture looks frozen')
print('stream:', sys.argv[2])
if failures:
    print('FAIL stream:', '; '.join(failures))
    sys.exit(1)
PY
    if [ "$status" -ne 0 ]; then echo "display-test: FAIL" >&2; fi
    return "$status"
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
