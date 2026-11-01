# Builds, signs, installs, removes and acceptance-tests the extend.computer
# indirect display driver on the Windows test PC. Driven by remote.sh subcommands
# driver-build / driver-install / driver-remove / driver-accept; see
# docs/development/windows.md. Build and sign need no elevation; install and
# remove need an elevated (admin) shell, which a plain ssh session already is.
#
#   -Action build    compile the driver (cl/link) into $env:TEMP\extend-driver\pkg
#   -Action sign     fresh short-lived self-signed cert, sign DLL + catalog, delete the private key
#   -Action install  trust the cert, stage the package, create or update the devnode, prune old ones
#   -Action remove   remove the devnode, delete the packages, remove the dev certs
#   -Action accept   run the acceptance tool at -Bin with -Arguments (none: the full test); exits non-zero on FAIL
param(
  [Parameter(Mandatory)][ValidateSet('build','sign','install','remove','accept')][string]$Action,
  [string]$Bin = '',
  [string]$Arguments = ''
)
$ErrorActionPreference = 'Stop'
$out = Join-Path $env:TEMP 'extend-driver'
$pkg = Join-Path $out 'pkg'
$cer = Join-Path $out 'dev.cer'
$hwid = 'Root\ExtendComputerDisplay'
$certSubject = 'CN=extend.computer Driver Dev'
# Framework versions the driver targets; the INF stamp and the headers must agree.
$umdf = '2.25'; $iddcx = '1.6'
# Signing timestamps through this server, so it needs network access. Without a
# timestamp the signatures would stop validating when the dev certificate expires.
$timestampUrl = 'http://timestamp.digicert.com'

# Runs a native tool with its stderr merged into its output, as text. Under
# $ErrorActionPreference = 'Stop', Windows PowerShell turns the first stderr
# line that 2>&1 merges into a terminating NativeCommandError, which would skip
# the exit-code checks and their messages; here $LASTEXITCODE decides.
function Invoke-Native([scriptblock]$Command) {
  $ErrorActionPreference = 'Continue'
  & $Command 2>&1 | ForEach-Object { "$_" }
}

# The newest installed Windows Kit with the IddCx headers, the UMDF headers and
# the driver tools (WDK); see "Test PC setup" in docs/development/windows.md.
function Find-Kit {
  $root = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue).KitsRoot10
  if (-not $root) { $root = "${env:ProgramFiles(x86)}\Windows Kits\10\" }
  $root = $root.TrimEnd('\')
  $version = Get-ChildItem (Join-Path $root 'Include') -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^10\.' -and (Test-Path "$($_.FullName)\um\iddcx\$iddcx\IddCx.h") -and
      (Test-Path "$root\bin\$($_.Name)\x86\stampinf.exe") -and (Test-Path "$root\bin\$($_.Name)\x86\Inf2Cat.exe") } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1 -ExpandProperty Name
  if (-not $version) { throw "no Windows Kit under $root has um\iddcx\$iddcx headers and stampinf/Inf2Cat; install the WDK" }
  if (-not (Test-Path "$root\Include\wdf\umdf\$umdf\wdf.h")) { throw "the WDK lacks the UMDF $umdf headers ($root\Include\wdf\umdf\$umdf)" }
  @{ Root = $root; Version = $version; Bin = "$root\bin\$version" }
}

# vcvars64.bat of the newest Visual Studio (Build Tools) with the x64 C++ tools.
function Find-Vcvars {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) { throw "vswhere.exe not found; install Visual Studio Build Tools with the C++ workload" }
  $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  $bat = if ($vs) { Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat' }
  if (-not $bat -or -not (Test-Path $bat)) { throw "no Visual Studio installation with the x64 C++ tools" }
  $bat
}

function Build {
  $kit = Find-Kit
  $vcvars = Find-Vcvars
  $k = $kit.Root; $v = $kit.Version
  "kit $v at $k; vcvars $vcvars"
  $src = Resolve-Path (Join-Path $PSScriptRoot '..\..\native\windows\driver')
  if (Test-Path $out) { Remove-Item $out -Recurse -Force }
  New-Item -ItemType Directory $pkg | Out-Null
  Copy-Item (Join-Path $src '*') $out
  Set-Location $out

  # DriverVer is unique and increasing for every build, so pnputil /add-driver
  # /install replaces the old package on update: 1.0.MMDD.<seconds since
  # midnight / 2>, each part <= 65535 (a build takes longer than 2 s). The same
  # version goes into the DLL (EXTEND_INFO.DriverVersion*, file version)
  # through ExtendVersion.h.
  $now = Get-Date
  $parts = 1, 0, [int]$now.ToString('MMdd'), [int][Math]::Floor($now.TimeOfDay.TotalSeconds / 2)
  $ver = $parts -join '.'
  $stamp = Invoke-Native { & "$($kit.Bin)\x86\stampinf.exe" -f ExtendComputerDisplay.inf -d '*' -v $ver -u "$umdf.0" }
  if ($LASTEXITCODE) { $stamp; throw "stampinf failed ($LASTEXITCODE)" }
  Set-Content ExtendVersion.h -Encoding ASCII -Value @(
    "#define EXTEND_VER_MAJOR $($parts[0])", "#define EXTEND_VER_MINOR $($parts[1])",
    "#define EXTEND_VER_BUILD $($parts[2])", "#define EXTEND_VER_REVISION $($parts[3])",
    "#define EXTEND_VER_STR `"$ver`"")

  $umdfMinor = $umdf.Split('.')[1]; $iddcxMinor = $iddcx.Split('.')[1]
  $inc = "/I. /I`"$k\Include\wdf\umdf\$umdf`" /I`"$k\Include\$v\um\iddcx\$iddcx`" /I`"$k\Include\$v\um`" /I`"$k\Include\$v\shared`""
  $defs = "/DUMDF_DRIVER /DUMDF_VERSION_MAJOR=2 /DUMDF_VERSION_MINOR=$umdfMinor " +
          "/DIDDCX_VERSION_MAJOR=1 /DIDDCX_VERSION_MINOR=$iddcxMinor /DIDDCX_MINIMUM_VERSION_REQUIRED=4 " +
          '/D_ATL_NO_WIN_SUPPORT /DUNICODE /D_UNICODE /D_WIN32_WINNT=0x0A00 /DWINVER=0x0A00 /DEXTEND_HAVE_VERSION_H'
  $libdirs = "/LIBPATH:`"$k\Lib\wdf\umdf\x64\$umdf`" /LIBPATH:`"$k\Lib\$v\um\x64\iddcx\$iddcx`""
  $libs = 'WdfDriverStubUm.lib IddCxStub.lib OneCoreUAP.lib ntdll.lib avrt.lib d3d11.lib dxgi.lib ole32.lib advapi32.lib'
  $cmd = @"
@echo off
call "$vcvars" >nul || exit /b 1
cl /nologo /c /O2 /MT /EHsc /std:c++17 /W3 /Zi $defs $inc Driver.cpp Edid.cpp || exit /b 2
rc /nologo /DEXTEND_HAVE_VERSION_H /I. /I"$k\Include\$v\um" /I"$k\Include\$v\shared" ExtendComputerDisplay.rc || exit /b 3
link /nologo /DLL /DEBUG /INCREMENTAL:NO /OUT:pkg\ExtendComputerDisplay.dll Driver.obj Edid.obj ExtendComputerDisplay.res $libdirs $libs || exit /b 4
"@
  Set-Content build.cmd $cmd -Encoding ASCII
  cmd /c build.cmd
  if ($LASTEXITCODE) { throw "driver build failed at step $LASTEXITCODE" }
  Copy-Item ExtendComputerDisplay.inf pkg\
  Get-ChildItem pkg | Select-Object Name, Length | Format-Table -AutoSize | Out-String
  Select-String -Path pkg\ExtendComputerDisplay.inf -Pattern 'DriverVer|UmdfLibraryVersion' | ForEach-Object { $_.Line.Trim() }
}

# Deletes every extend.computer dev certificate (and its key) from the user's
# personal store, a leftover from an interrupted signing run, and the public
# copies New-SelfSignedCertificate also leaves in the user's CA store.
function Remove-SigningCerts {
  Get-ChildItem Cert:\CurrentUser\My | Where-Object Subject -eq $certSubject | ForEach-Object {
    Remove-Item $_.PSPath -DeleteKey
    "removed stale signing cert $($_.Thumbprint) and its private key"
  }
  Get-ChildItem Cert:\CurrentUser\CA | Where-Object Subject -eq $certSubject | Remove-Item
}

function Sign {
  $kit = Find-Kit
  Set-Location $out
  Get-ChildItem pkg -Exclude ExtendComputerDisplay.dll, ExtendComputerDisplay.inf | Remove-Item
  Remove-Item $cer -ErrorAction SilentlyContinue
  Remove-SigningCerts
  $signtool = "$($kit.Bin)\x64\signtool.exe"
  $c = New-SelfSignedCertificate -Type CodeSigningCert -Subject $certSubject -KeyUsage DigitalSignature `
    -KeyExportPolicy NonExportable -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddDays(14)
  "cert $($c.Thumbprint) until $($c.NotAfter)"
  try {
    # The DLL first, so WUDFHost sees an Authenticode signature, then the
    # catalog over the signed files.
    Invoke-Native { & $signtool sign /fd sha256 /sha1 $c.Thumbprint /tr $timestampUrl /td sha256 pkg\ExtendComputerDisplay.dll } | Select-Object -Last 2
    if ($LASTEXITCODE) { throw "signtool (dll) failed ($LASTEXITCODE); timestamping needs network access to $timestampUrl" }
    Invoke-Native { & "$($kit.Bin)\x86\Inf2Cat.exe" /driver:pkg /os:10_X64,10_GE_X64 /uselocaltime } | Select-Object -Last 3
    if ($LASTEXITCODE) { throw "Inf2Cat failed ($LASTEXITCODE)" }
    Invoke-Native { & $signtool sign /fd sha256 /sha1 $c.Thumbprint /tr $timestampUrl /td sha256 pkg\ExtendComputerDisplay.cat } | Select-Object -Last 2
    if ($LASTEXITCODE) { throw "signtool (catalog) failed ($LASTEXITCODE); timestamping needs network access to $timestampUrl" }
    # Only a fully signed package gets the .cer that install trusts.
    Export-Certificate -Cert $c -FilePath $cer | Out-Null
  } finally {
    # The private key never outlives signing, whether or not signing succeeded.
    Remove-Item "Cert:\CurrentUser\My\$($c.Thumbprint)" -DeleteKey
    Remove-Item "Cert:\CurrentUser\CA\$($c.Thumbprint)" -ErrorAction SilentlyContinue
  }
  "private key removed: " + (-not (Test-Path "Cert:\CurrentUser\My\$($c.Thumbprint)"))
  foreach ($f in 'ExtendComputerDisplay.dll', 'ExtendComputerDisplay.cat') {
    $s = Get-AuthenticodeSignature "pkg\$f"
    "$f signer $($s.SignerCertificate.Thumbprint) timestamped $([bool]$s.TimeStamperCertificate)"
  }
}

Add-Type -Namespace Extend -Name Devcon -UsingNamespace System.Text -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)]
public struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public int DevInst; public IntPtr Reserved; }
[DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)]
public static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid ClassGuid, IntPtr hwndParent);
[DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)]
public static extern bool SetupDiCreateDeviceInfoW(IntPtr set, string name, ref Guid cls, string desc, IntPtr hwnd, int flags, ref SP_DEVINFO_DATA d);
[DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)]
public static extern bool SetupDiSetDeviceRegistryPropertyW(IntPtr set, ref SP_DEVINFO_DATA d, int prop, byte[] buf, int size);
[DllImport("setupapi.dll", SetLastError=true)]
public static extern bool SetupDiCallClassInstaller(int func, IntPtr set, ref SP_DEVINFO_DATA d);
[DllImport("setupapi.dll", SetLastError=true)]
public static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
[DllImport("newdev.dll", SetLastError=true, CharSet=CharSet.Unicode)]
public static extern bool UpdateDriverForPlugAndPlayDevicesW(IntPtr hwnd, string hwid, string inf, int flags, out bool reboot);
[DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disposition, uint flags, IntPtr template);
[DllImport("kernel32.dll", SetLastError=true)]
static extern bool DeviceIoControl(IntPtr h, uint code, IntPtr input, uint inputLength, uint[] output, uint outputLength, out uint returned, IntPtr overlapped);
[DllImport("kernel32.dll")]
static extern bool CloseHandle(IntPtr h);
// The DriverVer of the code the device actually runs, from IOCTL_EXTEND_GET_INFO
// (EXTEND_INFO, protocol in native/windows/driver/Protocol.h), or null when the
// interface does not open or answer.
public static string RunningVersion(string path) {
  // GENERIC_READ|GENERIC_WRITE, share read/write, OPEN_EXISTING,
  // SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION.
  IntPtr h = CreateFileW(path, 0xC0000000, 3, IntPtr.Zero, 3, 0x00110000, IntPtr.Zero);
  if (h == new IntPtr(-1)) return null;
  try {
    uint[] info = new uint[7];
    uint returned;
    if (!DeviceIoControl(h, 0x0022E000, IntPtr.Zero, 0, info, 28, out returned, IntPtr.Zero) || returned != 28) return null;
    return string.Format("{0}.{1}.{2}.{3}", info[2] >> 16, info[2] & 0xFFFF, info[3] >> 16, info[3] & 0xFFFF);
  } finally { CloseHandle(h); }
}
'@

function Create-Devnode($infPath) {
  # SetupDiCreateDeviceInfo + SPDRP_HARDWAREID + DIF_REGISTERDEVICE +
  # UpdateDriverForPlugAndPlayDevices: the devcon-install path without the WDK.
  $cls = [Guid]'4d36e968-e325-11ce-bfc1-08002be10318' # Display class
  $set = [Extend.Devcon]::SetupDiCreateDeviceInfoList([ref]$cls, [IntPtr]::Zero)
  if ($set -eq [IntPtr]::Zero -or $set -eq -1) { throw "SetupDiCreateDeviceInfoList failed" }
  try {
    $d = New-Object Extend.Devcon+SP_DEVINFO_DATA
    $d.cbSize = [Runtime.InteropServices.Marshal]::SizeOf([type][Extend.Devcon+SP_DEVINFO_DATA])
    if (-not [Extend.Devcon]::SetupDiCreateDeviceInfoW($set, 'Display', [ref]$cls, $null, [IntPtr]::Zero, 1, [ref]$d)) {
      throw "SetupDiCreateDeviceInfoW failed ($([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
    }
    $bytes = [Text.Encoding]::Unicode.GetBytes("$hwid`0`0")
    if (-not [Extend.Devcon]::SetupDiSetDeviceRegistryPropertyW($set, [ref]$d, 1, $bytes, $bytes.Length)) {
      throw "set SPDRP_HARDWAREID failed ($([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
    }
    if (-not [Extend.Devcon]::SetupDiCallClassInstaller(0x19, $set, [ref]$d)) {
      throw "DIF_REGISTERDEVICE failed ($([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
    }
    $reboot = $false
    if (-not [Extend.Devcon]::UpdateDriverForPlugAndPlayDevicesW([IntPtr]::Zero, $hwid, $infPath, 1, [ref]$reboot)) {
      throw "UpdateDriverForPlugAndPlayDevices failed ($([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
    }
    "devnode created; setup's reboot flag: $reboot"
  } finally {
    [void][Extend.Devcon]::SetupDiDestroyDeviceInfoList($set)
  }
}

function Get-OurDevice { Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains $hwid } }

# Version is the installed driver node's DriverVer; Running is the DriverVer of
# the code the device runs (GET_INFO), which differs while an update waits for a
# restart that has not happened.
function Get-DeviceState($device) {
  $p = Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName DEVPKEY_Device_ProblemCode, DEVPKEY_Device_DriverInfPath, DEVPKEY_Device_DriverVersion
  $hostProcess = Get-CimInstance Win32_Process -Filter "Name='WUDFHost.exe'" | Where-Object { $_.CommandLine -match 'ExtendComputerDisplayGroup' } | Select-Object -First 1
  $path = '\\?\' + ($device.InstanceId -replace '\\', '#') + '#{e053db0e-1431-463b-abdc-0cb768d6e649}'
  [pscustomobject]@{ Problem = $p[0].Data; Inf = $p[1].Data; Version = $p[2].Data; HostPid = $hostProcess.ProcessId
    Running = [Extend.Devcon]::RunningVersion($path) }
}

# Polls until the device runs $version (a starting device takes a moment).
function Wait-Running($version) {
  $deadline = (Get-Date).AddSeconds(15)
  do {
    $device = Get-OurDevice
    $state = if ($device) { Get-DeviceState $device }
    if ($state.Running -eq $version) { break }
    Start-Sleep 1
  } while ((Get-Date) -lt $deadline)
  $state
}

# Whether the DLL Windows loads for the device is this package's. DriverVer is
# unique per build, so this only catches an install that kept another build.
function Test-SameDll {
  $installed = 'C:\Windows\System32\drivers\UMDF\ExtendComputerDisplay.dll'
  (Test-Path $installed) -and (Get-FileHash $installed).Hash -eq (Get-FileHash (Join-Path $pkg 'ExtendComputerDisplay.dll')).Hash
}

# The PnP vetoes logged since $offset in setupapi.dev.log, and the Kernel-PnP
# 225 events naming the process that held a device open, split into those
# about our device and those about others.
function Get-Vetoes($offset, $start, $instanceId) {
  $log = 'C:\Windows\INF\setupapi.dev.log'
  $stream = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite')
  try {
    [void]$stream.Seek($offset, 'Begin')
    $text = (New-Object IO.StreamReader $stream).ReadToEnd()
  } finally { $stream.Dispose() }
  $lines = @($text -split "`r?`n" | Where-Object { $_ -match 'vetoed by' } | ForEach-Object { $_.Trim() } | Select-Object -Unique)
  $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-PnP'; Id = 225; StartTime = $start } -ErrorAction SilentlyContinue |
    ForEach-Object { "Kernel-PnP 225: $($_.Message -replace '\s+', ' ')" })
  $all = @($lines + $events | Where-Object { $_ })
  $id = [regex]::Escape($instanceId)
  [pscustomobject]@{ Ours = @($all | Where-Object { $_ -match $id }); Others = @($all | Where-Object { $_ -notmatch $id }) }
}

function Report-Vetoes($vetoes) {
  if (-not $vetoes.Ours -and -not $vetoes.Others) { "no PnP veto: nothing asks for a reboot"; return }
  if ($vetoes.Others) {
    "Windows set its reboot-pending flag because another device refused to restart (harmless to ours):"
    $vetoes.Others | ForEach-Object { "  $_" }
  }
  if ($vetoes.Ours) {
    "our device refused to restart, because a client holds it open:"
    $vetoes.Ours | ForEach-Object { "  $_" }
  }
}

# Deletes staged ExtendComputerDisplay packages other than the devnode's, then
# the dev certs that no longer sign any remaining package.
function Remove-Stale($currentInf) {
  $blocks = (pnputil /enum-drivers | Out-String) -split "`r?`n`r?`n" | Where-Object { $_ -match 'Original Name:\s+extendcomputerdisplay\.inf' }
  foreach ($b in $blocks) {
    if ($b -match 'Published Name:\s+(oem\d+\.inf)' -and $Matches[1] -ne $currentInf) {
      $old = $Matches[1]
      pnputil /delete-driver $old | Out-Null
      if ($LASTEXITCODE) { "could not delete stale package $old ($LASTEXITCODE)" } else { "deleted stale package $old" }
    }
  }
  $signers = @(Get-ChildItem C:\Windows\System32\DriverStore\FileRepository -Directory -Filter 'extendcomputerdisplay.inf_*' |
    ForEach-Object { (Get-AuthenticodeSignature (Join-Path $_.FullName 'extendcomputerdisplay.cat')).SignerCertificate.Thumbprint } |
    Where-Object { $_ })
  if (-not $signers) { "no staged package signer found; keeping the dev certs"; return }
  foreach ($store in 'Root', 'TrustedPublisher') {
    Get-ChildItem "Cert:\LocalMachine\$store" | Where-Object { $_.Subject -eq $certSubject -and $signers -notcontains $_.Thumbprint } | ForEach-Object {
      Remove-Item $_.PSPath; "removed stale dev cert $($_.Thumbprint) from $store"
    }
  }
}

function Install {
  $cat = Join-Path $pkg 'ExtendComputerDisplay.cat'
  $inf = Join-Path $pkg 'ExtendComputerDisplay.inf'
  if (-not (Test-Path $cer) -or -not (Test-Path $cat)) { throw "no signed package in $pkg; run build+sign first" }

  # Trust the dev cert only if its private key is gone everywhere and it really
  # signed this package.
  $cert = New-Object Security.Cryptography.X509Certificates.X509Certificate2 $cer
  $thumb = $cert.Thumbprint
  $keyed = @(Get-ChildItem Cert:\ -Recurse -ErrorAction SilentlyContinue | Where-Object { $_.Thumbprint -eq $thumb -and $_.HasPrivateKey })
  if ($keyed) { throw "refusing to trust ${thumb}: its private key still exists ($($keyed[0].PSParentPath)); re-run sign" }
  foreach ($f in $cat, (Join-Path $pkg 'ExtendComputerDisplay.dll')) {
    $s = Get-AuthenticodeSignature $f
    if ($s.Status -eq 'NotSigned' -or $s.SignerCertificate.Thumbprint -ne $thumb) {
      throw "refusing to install: $(Split-Path -Leaf $f) is $($s.Status), signer $($s.SignerCertificate.Thumbprint), expected $thumb"
    }
  }
  $newVer = ((Select-String -Path $inf -Pattern '^DriverVer\s*=\s*[^,]+,\s*([\d.]+)').Matches[0].Groups[1].Value)

  $start = Get-Date
  $logOffset = (Get-Item C:\Windows\INF\setupapi.dev.log).Length
  Invoke-Native { wevtutil sl Microsoft-Windows-DriverFrameworks-UserMode/Operational /e:true } | Out-Null
  Import-Certificate $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
  Import-Certificate $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
  $status = (Get-AuthenticodeSignature $cat).Status
  "catalog: $status (DriverVer $newVer)"
  if ($status -ne 'Valid') { throw "catalog does not validate after trusting $thumb ($status)" }

  $existing = Get-OurDevice
  if ($existing) {
    $before = Get-DeviceState $existing
    "== update $($existing.InstanceId): installed $($before.Version), running $($before.Running) (host pid $($before.HostPid))"
    if ($before.Running -eq $newVer) {
      if (-not (Test-SameDll)) { throw "the device runs DriverVer $newVer from another build; rebuild so the version differs" }
      "already running $newVer; nothing to update"
    } else {
      # pnputil /install restarts the device on the newer DriverVer itself
      # when no client holds it open.
      pnputil /add-driver $inf /install | Select-String 'Published|Driver package|Failed|reboot' | ForEach-Object { $_.Line.Trim() }
      $after = Wait-Running $newVer
      if ($after.Running -ne $newVer) {
        $vetoes = Get-Vetoes $logOffset $start $existing.InstanceId
        if ($vetoes.Ours) {
          Report-Vetoes $vetoes
          throw "the device still runs $($after.Running): a client holds it open (above). Close it, then run driver-install again; nothing was pruned"
        }
        # Nothing holds the device, but it did not restart: an earlier update
        # was vetoed, or the device failed. Once another device's veto has set
        # Windows' reboot-pending flag, pnputil /restart-device refuses to
        # restart ours, so re-create the devnode on the staged package.
        "the device runs $($after.Running) (problem $($after.Problem)) instead of $newVer; re-creating the devnode"
        Invoke-Native { pnputil /remove-device $existing.InstanceId } | Select-String 'removed|Failed|reboot' | ForEach-Object { $_.Line.Trim() }
        Start-Sleep 2
        if (Get-OurDevice) { throw "could not remove $($existing.InstanceId) to re-create it; close every client and run driver-install again" }
        Create-Devnode $inf
      }
    }
  } else {
    # First install: stage the package, then create the devnode.
    "== stage package"
    pnputil /add-driver $inf | Select-String 'Published|Driver package|Failed' | ForEach-Object { $_.Line.Trim() }
    "== create devnode"
    Create-Devnode $inf
  }

  "== device"
  $state = Wait-Running $newVer
  $device = Get-OurDevice
  if (-not $device) { throw "no $hwid devnode after install" }
  "$($device.InstanceId) status=$($device.Status) problem=$($state.Problem) inf=$($state.Inf) installed=$($state.Version) running=$($state.Running) host pid=$($state.HostPid)"
  $dll = Get-Item C:\Windows\System32\drivers\UMDF\ExtendComputerDisplay.dll -ErrorAction SilentlyContinue
  "installed DLL file version $($dll.VersionInfo.FileVersion)"
  if ($state.HostPid) {
    (Get-Process -Id $state.HostPid).Modules | Where-Object { $_.ModuleName -match 'ExtendComputerDisplay|iddcx' } | ForEach-Object { "  module $($_.ModuleName)" }
  }
  "== code integrity since start"
  Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-CodeIntegrity/Operational'; StartTime=$start} -ErrorAction SilentlyContinue |
    Where-Object { $_.Message -match 'ExtendComputerDisplay|WUDFHost' } | Select-Object -First 5 |
    ForEach-Object { "$($_.Id): $($_.Message.Substring(0,[Math]::Min(160,$_.Message.Length)))" }
  "== reboot flag"
  Report-Vetoes (Get-Vetoes $logOffset $start $device.InstanceId)

  if ($state.Problem -ne 0 -or $state.Running -ne $newVer) {
    throw "the device runs $($state.Running) (problem $($state.Problem)), not DriverVer $newVer; nothing was pruned"
  }
  if (-not (Test-SameDll)) { throw "the installed DLL is not this build's although DriverVer matches; rebuild and install again" }
  "our device runs DriverVer $newVer (GET_INFO); no reboot is needed to use it"
  "== prune"
  Remove-Stale $state.Inf
}

function Remove {
  "== remove devnode"
  Get-OurDevice | ForEach-Object {
    Invoke-Native { pnputil /remove-device "$($_.InstanceId)" } | Out-String
  }
  "== delete driver packages"
  $pub = (pnputil /enum-drivers | Out-String) -split "`r?`n`r?`n" | Where-Object { $_ -match 'Original Name:\s+extendcomputerdisplay\.inf' } | ForEach-Object { if ($_ -match 'Published Name:\s+(oem\d+\.inf)') { $Matches[1] } }
  foreach ($p in $pub) { Invoke-Native { pnputil /delete-driver $p /uninstall /force } | Out-String }
  "== remove dev certificates"
  foreach ($store in 'Root', 'TrustedPublisher') {
    Get-ChildItem "Cert:\LocalMachine\$store" | Where-Object Subject -eq $certSubject | ForEach-Object { Remove-Item $_.PSPath; "removed $($_.Thumbprint) from $store" }
  }
  Remove-SigningCerts
  "== state"
  "devnodes: " + @(Get-OurDevice).Count
}

function Accept {
  if (-not $Bin) { throw "accept needs -Bin <path to extend-computer-driver-test.exe>" }
  $exe = if ($Bin -like '*.exe') { $Bin } else { Join-Path $Bin 'extend-computer-driver-test.exe' }
  if (-not (Test-Path $exe)) { throw "missing $exe; build native/windows first" }
  $argv = @($Arguments -split '\s+' | Where-Object { $_ })
  & $exe @argv
  if ($LASTEXITCODE) { throw "acceptance FAIL (exit code $LASTEXITCODE)" }
}

switch ($Action) {
  'build'   { Build }
  'sign'    { Sign }
  'install' { Install }
  'remove'  { Remove }
  'accept'  { Accept }
}
