# UI Automation in the desktop session:
#   -Action list [-Window <title part>]                top-level windows, or the named elements of matching ones
#   -Action invoke -Name <element> [-Window <title part>]
#   -Action click -X <x> -Y <y>                        physical pixels
#   -Action type -Text <SendKeys text>
param(
  [Parameter(Mandatory)][ValidateSet('list', 'invoke', 'click', 'type')][string]$Action,
  [string]$Name,
  [string]$Window,
  [int]$X,
  [int]$Y,
  [string]$Text
)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
'@
[W.U]::SetProcessDPIAware() | Out-Null
$A = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.TreeScope]
$any = [System.Windows.Automation.Condition]::TrueCondition

function Roots { $A::RootElement.FindAll($T::Children, $any) | ? { $_.Current.Name -like "*$Window*" } }
# Approaches the point so the target sees the pointer arrive, then clicks.
function Click([int]$x, [int]$y) {
  [W.U]::SetCursorPos($x - 3, $y) | Out-Null; Start-Sleep -Milliseconds 80
  [W.U]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 80
  [W.U]::mouse_event(2, 0, 0, 0, 0); Start-Sleep -Milliseconds 60; [W.U]::mouse_event(4, 0, 0, 0, 0)
}

switch ($Action) {
  'list' {
    foreach ($w in Roots) {
      "== [$($w.Current.ControlType.ProgrammaticName)] $($w.Current.Name)"
      if ($Window) {
        foreach ($e in $w.FindAll($T::Descendants, $any)) {
          if ($e.Current.Name) {
            $b = $e.Current.BoundingRectangle
            $at = if ($b.IsEmpty -or [double]::IsInfinity($b.X)) { 'offscreen' } else { '{0},{1} {2}x{3}' -f [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height }
            "  [$($e.Current.ControlType.ProgrammaticName)] '$($e.Current.Name)' @ $at"
          }
        }
      }
    }
  }
  'invoke' {
    $c = New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Name)
    $e = $null
    foreach ($w in Roots) { $e = $w.FindFirst($T::Descendants, $c); if ($e) { break } }
    if (-not $e) { throw "not found: $Name" }
    $p = $null
    if ($e.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke() }
    else { $b = $e.Current.BoundingRectangle; Click ($b.X + $b.Width / 2) ($b.Y + $b.Height / 2) }
    "ok invoke $Name"
  }
  'click' { Click $X $Y; "ok click $X,$Y" }
  'type' { [System.Windows.Forms.SendKeys]::SendWait($Text); 'ok type' }
}
