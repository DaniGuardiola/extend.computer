# Replays a synthetic H.264 stream into the display helper's viewer, which opens full screen on the primary
# monitor, and checks its replies and input; viewer-test.py beside this script does the work. It clicks and
# types into the viewer window.
param([Parameter(Mandatory)][string]$Bin, [int]$Width = 1920, [int]$Height = 1080)
$exe = Join-Path $Bin ExtendComputerDisplay.exe
if (-not (Test-Path $exe)) { throw "missing $exe; build native/windows first" }
$script = Join-Path $PSScriptRoot viewer-test.py
# cmd merges Python's stderr, which Windows PowerShell would turn into an error record.
cmd /c "python `"$script`" `"$exe`" $Width $Height 2>&1"
if ($LASTEXITCODE) { throw "viewer test failed" }
