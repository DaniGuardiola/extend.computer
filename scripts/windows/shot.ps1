# Saves the whole desktop, in physical pixels, as shot.png beside this script.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace W -Name D -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[W.D]::SetProcessDPIAware() | Out-Null
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bitmap = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
[System.Drawing.Graphics]::FromImage($bitmap).CopyFromScreen($screen.Left, $screen.Top, 0, 0, $bitmap.Size)
$bitmap.Save((Join-Path $PSScriptRoot shot.png), [System.Drawing.Imaging.ImageFormat]::Png)
