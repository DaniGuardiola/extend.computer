# Injects a fixed sequence in the desktop session as if from the user: the pointer out across the left edge,
# "a" and "A", back across, then Ctrl+Alt+Esc. -Mode capture runs capture-control-left for a 1728x1117 peer
# against it and prints what the helper reports; -Mode drive only injects, to exercise the running app.
param([string]$Bin, [ValidateSet('capture', 'drive')][string]$Mode = 'capture')
Add-Type -Namespace T -Name I -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)] public struct MI { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
[StructLayout(LayoutKind.Sequential)] public struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; public uint pad1, pad2; }
[StructLayout(LayoutKind.Sequential)] public struct INPUTM { public uint type; public MI mi; }
[StructLayout(LayoutKind.Sequential)] public struct INPUTK { public uint type; public KI ki; }
[DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUTM[] i, int size);
[DllImport("user32.dll", EntryPoint="SendInput")] public static extern uint SendKey(uint n, INPUTK[] i, int size);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
public static void Move(int dx, int dy) { var i = new INPUTM[1]; i[0].type = 0; i[0].mi.dx = dx; i[0].mi.dy = dy; i[0].mi.flags = 1; SendInput(1, i, Marshal.SizeOf(typeof(INPUTM))); }
public static void Key(ushort scan, bool down) { var i = new INPUTK[1]; i[0].type = 1; i[0].ki.scan = scan; i[0].ki.flags = 8u | (down ? 0u : 2u); SendKey(1, i, Marshal.SizeOf(typeof(INPUTK))); }
'@
[T.I]::SetProcessDPIAware() | Out-Null

$helper = $null
if ($Mode -eq 'capture') {
  $exe = Join-Path $Bin ExtendComputerCursor.exe
  if (-not (Test-Path $exe)) { throw "missing $exe; build native/windows first" }
  $info = New-Object Diagnostics.ProcessStartInfo $exe, 'capture-control-left 1728 1117 0'
  $info.UseShellExecute = $false; $info.RedirectStandardInput = $true; $info.RedirectStandardOutput = $true; $info.CreateNoWindow = $true
  $helper = [Diagnostics.Process]::Start($info)
  $lines = New-Object Collections.Concurrent.ConcurrentQueue[string]
  Register-ObjectEvent $helper OutputDataReceived -MessageData $lines -Action { if ($EventArgs.Data) { $Event.MessageData.Enqueue($EventArgs.Data) } } | Out-Null
  $helper.BeginOutputReadLine()
}
# The helper stops after 2 s without a heartbeat.
function Alive { if ($helper) { $helper.StandardInput.WriteLine('ALIVE'); $helper.StandardInput.Flush() } }
function Keys([int[]]$down) {
  foreach ($scan in $down) { [T.I]::Key($scan, $true) }
  [array]::Reverse($down); foreach ($scan in $down) { [T.I]::Key($scan, $false) }
}

Start-Sleep -Milliseconds 400; Alive
[T.I]::SetCursorPos(3, 500) | Out-Null; Start-Sleep -Milliseconds 150
[T.I]::Move(-20, 0); Start-Sleep -Milliseconds 150; Alive
for ($i = 0; $i -lt 10; $i++) { [T.I]::Move(-30, 5); Start-Sleep -Milliseconds 30 }
Alive
Keys 0x1E; Keys 0x2A, 0x1E
Start-Sleep -Milliseconds 300; Alive
for ($i = 0; $i -lt 14; $i++) { [T.I]::Move(60, 0); Start-Sleep -Milliseconds 30 }
Start-Sleep -Milliseconds 300; Alive
Keys 0x1D, 0x38, 0x01

if ($helper) {
  if (-not $helper.WaitForExit(3000)) { $helper.Kill() }
  Start-Sleep -Milliseconds 300
  $lines.ToArray()
  "exit=$($helper.ExitCode)"
}
