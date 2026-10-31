# Runs the display helper in mirror mode for ~5 s while sweeping the pointer, writing its stream to display.bin
# beside this script, and prints its log. -Encoder software selects the software H.264 encoder.
param([Parameter(Mandatory)][string]$Bin, [string]$Encoder = '')
Add-Type -Namespace W -Name M -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
'@
[W.M]::SetProcessDPIAware() | Out-Null
$exe = Join-Path $Bin ExtendComputerDisplay.exe
if (-not (Test-Path $exe)) { throw "missing $exe; build native/windows first" }
$stream = Join-Path $PSScriptRoot display.bin
$log = Join-Path $PSScriptRoot display.err
$env:EXTEND_DISPLAY_ENCODER = $Encoder
# ping holds the helper's stdin open for ~6 s; the helper stops when it closes.
$helper = Start-Process cmd -NoNewWindow -PassThru `
  -ArgumentList "/c ping -n 7 127.0.0.1 | `"$exe`" mirror 1920 1080 60 12000000 standard 0 h264 embedded > `"$stream`" 2> `"$log`""
Start-Sleep -Milliseconds 800
$clock = [Diagnostics.Stopwatch]::StartNew()
while ($clock.ElapsedMilliseconds -lt 4000) {
  $t = $clock.ElapsedMilliseconds / 300
  [W.M]::SetCursorPos(400 + [int](600 * [Math]::Sin($t)), 500 + [int](250 * [Math]::Cos($t))) | Out-Null
  Start-Sleep -Milliseconds 8
}
$helper.WaitForExit(10000) | Out-Null
Get-Content $log
