# Runs the display helper as a source for ~5 s while sweeping the pointer, writing its stream to display.bin
# beside this script, and prints its log. -Encoder software selects the software H.264 encoder.
# -Mode extend adds the virtual monitor (Width x Height, Retina=1 doubles it) and sweeps the pointer over it;
# Change=WxH sends a configuration halfway, offered from the start as EXTEND_DISPLAY_PRESETS does in the app
# unless Replace=1, and Kill=1 kills the helper instead of stopping it. Idle=N parks the pointer for N s before
# the stop, and Serial=N sets EXTEND_DISPLAY_SERIAL. Afterwards it checks that the monitor is gone and the
# display layout is as before, and throws (so the run fails) unless every check passed, including that the helper
# never warned that Windows scales the frames or refused the scale. remote.sh checks the stream itself.
param(
  [Parameter(Mandatory)][string]$Bin,
  [string]$Encoder = '',
  [string]$Mode = 'mirror',
  [int]$Width = 1920,
  [int]$Height = 1080,
  [string]$Retina = '',
  [string]$Change = '',
  [string]$Replace = '',
  [string]$Kill = '',
  [string]$Idle = '',
  [string]$Serial = ''
)
Add-Type -Namespace W -Name M -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
[StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
[StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public MOUSEINPUT mi; }
[DllImport("user32.dll")] public static extern uint SendInput(uint count, INPUT[] inputs, int size);
// A relative mouse move, real input to Windows: it wakes a display that went to sleep. SetCursorPos doesn't.
public static uint Nudge(int dx) {
  var input = new INPUT[1]; input[0].mi.dx = dx; input[0].mi.flags = 1;
  return SendInput(1, input, Marshal.SizeOf(typeof(INPUT)));
}
[StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
[StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int Size; public RECT Monitor, Work; public uint Flags; }
public delegate bool MonitorProc(IntPtr monitor, IntPtr dc, ref RECT rect, IntPtr data);
[DllImport("user32.dll")] public static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, MonitorProc proc, IntPtr data);
[DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr monitor, ref MONITORINFO info);
// Every monitor's desktop rectangle, primary marked, sorted.
public static string Layout() {
  var list = new System.Collections.Generic.List<string>();
  EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (IntPtr m, IntPtr dc, ref RECT r, IntPtr d) => {
    var info = new MONITORINFO(); info.Size = Marshal.SizeOf(info); GetMonitorInfo(m, ref info);
    var b = info.Monitor;
    list.Add(string.Format("{0}x{1}@{2},{3}{4}", b.Right - b.Left, b.Bottom - b.Top, b.Left, b.Top, (info.Flags & 1) != 0 ? " primary" : ""));
    return true;
  }, IntPtr.Zero);
  list.Sort();
  return string.Join("; ", list);
}
'@
# Physical pixels on every monitor, like the helper.
if (-not [W.M]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))) { [W.M]::SetProcessDPIAware() | Out-Null }
# Windows draws nothing while the display sleeps, so neither source would see a frame.
[W.M]::Nudge(1) + [W.M]::Nudge(-1) | Out-Null
Start-Sleep -Milliseconds 500
$exe = Join-Path $Bin ExtendComputerDisplay.exe
if (-not (Test-Path $exe)) { throw "missing $exe; build native/windows first" }
$stream = Join-Path $PSScriptRoot display.bin
$log = Join-Path $PSScriptRoot display.err
Remove-Item $stream, $log -ErrorAction SilentlyContinue
function Ours { @(Get-PnpDevice -PresentOnly -Class Monitor -ErrorAction SilentlyContinue | ? InstanceId -like 'DISPLAY\XEC*').Count }
# The newest "Extended display: WxH at X,Y" rectangle in the helper's log.
function Placed {
  $line = Get-Content $log -ErrorAction SilentlyContinue | ? { $_ -match '^Extended display: (\d+)x(\d+) at (-?\d+),(-?\d+)' } | Select -Last 1
  if ($line -and $line -match '^Extended display: (\d+)x(\d+) at (-?\d+),(-?\d+)') {
    return @{ W = [int]$matches[1]; H = [int]$matches[2]; X = [int]$matches[3]; Y = [int]$matches[4]; Line = $line }
  }
}
$layout = [W.M]::Layout()
$before = Ours
$cursor = New-Object W.M+POINT
[W.M]::GetCursorPos([ref]$cursor) | Out-Null
"layout before: $layout; our monitors: $before"

$env:EXTEND_DISPLAY_ENCODER = $Encoder
$env:EXTEND_DISPLAY_SERIAL = $Serial
$env:EXTEND_DISPLAY_PRESETS = if ($Change -and $Replace -ne '1') { $Change } else { '' }
$quality = if ($Retina -eq '1') { 'retina' } else { 'standard' }
$launch = "$Mode $Width $Height 60 12000000 $quality 0 h264 embedded"
# cmd writes the helper's stdout and stderr straight to files; its stdin is this script's pipe.
$start = New-Object Diagnostics.ProcessStartInfo cmd.exe, "/c `"`"$exe`" $launch > `"$stream`" 2> `"$log`"`""
$start.UseShellExecute = $false
$start.RedirectStandardInput = $true
$start.CreateNoWindow = $true
$helper = [Diagnostics.Process]::Start($start)
$clock = [Diagnostics.Stopwatch]::StartNew()

# The sweep's centre and reach: the extended display's middle, or as before on the primary.
function Sweep($rect) { @{ CX = $rect.X + $rect.W / 2; CY = $rect.Y + $rect.H / 2; AX = $rect.W * 0.4; AY = $rect.H * 0.4; Line = $rect.Line } }
$area = @{ CX = 400; CY = 500; AX = 600; AY = 250 }
if ($Mode -eq 'extend') {
  while (-not ($placed = Placed) -and $clock.ElapsedMilliseconds -lt 20000 -and -not $helper.HasExited) { Start-Sleep -Milliseconds 100 }
  if (-not $placed) { "no extended display was placed" }
  else { "placed after $($clock.ElapsedMilliseconds) ms: $($placed.Line); layout: $([W.M]::Layout())"; $area = Sweep $placed }
} else {
  Start-Sleep -Milliseconds 800
}
$sweep = [Diagnostics.Stopwatch]::StartNew()
$changed = $false
$killed = $false
while ($sweep.ElapsedMilliseconds -lt 4000 -and -not $helper.HasExited) {
  if ($Change -and -not $changed -and $sweep.ElapsedMilliseconds -ge 1500) {
    $w, $h = $Change -split 'x'
    # PowerShell names ignore case, so this must not be called $retina.
    $doubled = if ($Retina -eq '1') { 'true' } else { 'false' }
    $helper.StandardInput.WriteLine("{`"bitrate`":12000000,`"fps`":60,`"height`":$h,`"id`":7,`"retina`":$doubled,`"type`":`"configuration`",`"width`":$w}")
    $helper.StandardInput.Flush()
    $changed = $true
    $from = $area.Line
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ($wait.ElapsedMilliseconds -lt 10000 -and ($placed = Placed) -and $placed.Line -eq $from) { Start-Sleep -Milliseconds 50 }
    if ($placed.Line -ne $from) { "configured after $($wait.ElapsedMilliseconds) ms: $($placed.Line); layout: $([W.M]::Layout())"; $area = Sweep $placed }
  }
  if ($Kill -eq '1' -and -not $killed -and $sweep.ElapsedMilliseconds -ge 2500) {
    # Only this test's helper, the child of its cmd.
    Get-CimInstance Win32_Process -Filter "ParentProcessId = $($helper.Id) AND Name = 'ExtendComputerDisplay.exe'" |
      % { Stop-Process -Id $_.ProcessId -Force }
    $killed = $true
    "killed the helper"
    break
  }
  $t = $sweep.ElapsedMilliseconds / 300
  $x = [int]($area.CX + $area.AX * [Math]::Sin($t))
  $y = [int]($area.CY + $area.AY * [Math]::Cos($t))
  [W.M]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 8
}
[W.M]::SetCursorPos($cursor.X, $cursor.Y) | Out-Null
if ($Idle -and -not $killed) {
  # The pointer rests off the extended display: only the idle refresh and keyframe requests encode.
  $rest = [Diagnostics.Stopwatch]::StartNew()
  while ($rest.ElapsedMilliseconds -lt [int]$Idle * 1000 -and -not $helper.HasExited) { Start-Sleep -Milliseconds 100 }
  "idle for $($rest.ElapsedMilliseconds) ms"
}
$early = $helper.HasExited -and -not $killed
$stopping = [Diagnostics.Stopwatch]::StartNew()
# A killed helper's cmd may already be gone.
try { $helper.StandardInput.WriteLine('{"type":"stop"}'); $helper.StandardInput.Close() } catch {}
$helper.WaitForExit(15000) | Out-Null
"helper exited after $($stopping.ElapsedMilliseconds) ms with code $($helper.ExitCode)"
Get-Content $log
$failures = @()
if ($early) { $failures += 'the helper exited before it was stopped' }
elseif (-not $killed -and $helper.ExitCode) { $failures += "the helper exited with code $($helper.ExitCode)" }
if (-not (Test-Path $stream) -or (Get-Item $stream).Length -eq 0) { $failures += 'no stream' }
if ($Mode -eq 'extend') {
  $placements = @(Get-Content $log -ErrorAction SilentlyContinue | ? { $_ -match '^Extended display: \d+x\d+ at ' })
  if (-not $placements) { $failures += 'no extended display was placed' }
  # 100%, or 200% with Retina, whatever the mode.
  $scale = if ($Retina -eq '1') { 200 } else { 100 }
  foreach ($line in $placements) { if ($line -notmatch ", $scale% scale$") { $failures += "not at $scale% scale: $line" } }
  # Frames Windows scales onto another signal or texture size, or a scale it won't allow, would still stream.
  $warnings = @(Get-Content $log -ErrorAction SilentlyContinue | ? {
    $_ -match '^Extended display: Windows scales it to |^Extended display texture: \d+x\d+ for a |^Extended display scale: '
  })
  foreach ($line in $warnings) { $failures += "the helper warned: $line" }
  if (Get-Content $log -ErrorAction SilentlyContinue | ? { $_ -match '^Display settings:' }) { $failures += 'a configuration failed' }
  elseif ($Change -and $changed -and $placements.Count -lt 2 -and -not $killed) { $failures += "the configuration to $Change didn't show" }
  $wait = [Diagnostics.Stopwatch]::StartNew()
  while ($wait.ElapsedMilliseconds -lt 10000 -and ((Ours) -ne $before -or [W.M]::Layout() -ne $layout)) { Start-Sleep -Milliseconds 250 }
  $after = Ours
  $now = [W.M]::Layout()
  "layout after: $now; our monitors: $after"
  if ($after -eq $before -and $now -eq $layout) { "PASS cleanup: monitor gone and layout restored within $($wait.ElapsedMilliseconds) ms" }
  else { "FAIL cleanup: monitors $before -> $after, layout '$layout' -> '$now'"; $failures += 'cleanup' }
}
if ($failures) { throw "FAIL: $($failures -join '; ')" }
"PASS"
