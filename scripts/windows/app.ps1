# Runs the debug desktop app from the build directory until it exits, logging its output to app.log beside
# this script. It shares this hidden console, so the debug build opens no console window.
param([Parameter(Mandatory)][string]$Bin)
$exe = Join-Path $Bin extend-computer-desktop.exe
$log = Join-Path $PSScriptRoot app.log
Start-Process cmd -ArgumentList "/c `"`"$exe`" > `"$log`" 2>&1`"" -WorkingDirectory $Bin -NoNewWindow -Wait
