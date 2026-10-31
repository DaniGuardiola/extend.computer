# Runs <Script>.ps1 from this directory in the signed-in user's desktop session, with the key=value lines of
# <Script>.args as its parameters, and prints its output. SSH sessions run in session 0, which cannot see or
# drive the desktop, so the script runs as a scheduled task with an interactive principal. -Elevated uses the
# highest run level, which UI Automation needs to reach elevated windows such as the firewall prompt.
param(
  [Parameter(Mandatory)][ValidatePattern('^[\w-]+$')][string]$Script,
  [switch]$Elevated,
  [switch]$NoWait,
  [int]$Timeout = 60,
  [switch]$Inside
)
$ErrorActionPreference = 'Stop'
$file, $parameters, $out, $done = 'ps1', 'args', 'out', 'done' | % { Join-Path $PSScriptRoot "$Script.$_" }

if ($Inside) {
  $code = 0
  try {
    $values = @{}
    foreach ($line in Get-Content -Encoding UTF8 $parameters) {
      if ($line) { $key, $value = $line -split '=', 2; $values[$key] = $value }
    }
    & $file @values *> $out
  } catch { $_ | Out-File -Append $out; $code = 1 }
  # Publish the exit code atomically; the waiting side reads it once the file exists.
  Set-Content "$done.tmp" $code
  Move-Item -Force "$done.tmp" $done
  exit
}

$task = "extend-dev-$Script"
# A leftover instance would make Task Scheduler ignore this start.
Stop-ScheduledTask $task -ErrorAction SilentlyContinue
Remove-Item $out, $done, "$done.tmp" -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute powershell.exe -WorkingDirectory $PSScriptRoot `
  -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Script $Script -Inside"
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel $(if ($Elevated) { 'Highest' } else { 'Limited' })
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit 0 -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask $task -Action $action -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask $task
if ($NoWait) { exit }

$deadline = (Get-Date).AddSeconds($Timeout)
while (-not (Test-Path $done) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
# The SSH shell passes these bytes on unchanged, so names with accents arrive intact.
[Console]::OutputEncoding = [Text.Encoding]::UTF8
if (Test-Path $out) { Get-Content $out }
if (-not (Test-Path $done)) {
  Stop-ScheduledTask $task
  throw "$task did not finish within $Timeout s. Is a user signed in to the desktop?"
}
Unregister-ScheduledTask $task -Confirm:$false
exit [int](Get-Content $done)
