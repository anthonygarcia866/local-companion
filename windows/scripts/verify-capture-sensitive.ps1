<#
  Live check that the capture layer never reads or passes on payment and ID
  data, on the real app, with no mouse or keyboard input.

    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-capture-sensitive.ps1 [-Exe <glim.exe>] [-Out <dir>]

  How it works:
  - Starts the built Glim with GLIM_DEV=1 (the capture thread and its debug
    panel run only then), and stops it at the end and starts it normally.
  - Serves scripts/fixtures/capture-sensitive on 127.0.0.1 (Python's
    http.server) and opens it in a throwaway Edge profile, off-screen, with
    background networking off. Fixture data is fake: a published test card,
    a never-issued SSN, the standard example IBAN.
  - A tiny WinForms app compiled on the spot as KeePass.exe shows an editable
    text box, for the paused-app check.
  - Focus moves with UI Automation SetFocus only. After each move it reads
    the debug panel's window title (metadata only) and every piece of text
    in the panel (through UI Automation, it is Glim's own window).
  - Then it searches the panel's text, the results log lines and glim.log
    lines written during the run for every fake number, in any formatting.
#>
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\glim.exe"),
  [string]$Out = (Join-Path $PSScriptRoot "..\..\docs\verification\capture-sensitive")
)
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$log = Join-Path $Out "verify-log.txt"
Set-Content -Path $log -Value "" -Encoding utf8
function Note([string]$s) { $line = "{0:HH:mm:ss.fff} {1}" -f (Get-Date), $s; Write-Host $line; Add-Content -Path $log -Value $line -Encoding utf8 }
$failures = New-Object System.Collections.Generic.List[string]
function Fail([string]$s) { $failures.Add($s); Note "FAIL: $s" }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class CW {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  public static IntPtr Find(uint pid, string prefix) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid) { var sb = new System.Text.StringBuilder(512); GetWindowText(h, sb, 512); if (sb.ToString().StartsWith(prefix)) { found = h; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
  [StructLayout(LayoutKind.Sequential)] struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
  [DllImport("user32")] static extern bool GetLastInputInfo(ref LASTINPUTINFO i);
  public static uint IdleMs() { var i = new LASTINPUTINFO(); i.cbSize = 8; GetLastInputInfo(ref i); return (uint)Environment.TickCount - i.dwTime; }
  public static string Title(IntPtr h) { var sb = new System.Text.StringBuilder(512); GetWindowText(h, sb, 512); return sb.ToString(); }
}
'@

# Every fake number, and the digits alone, which must never come out.
$fakes = @("4111 1111 1111 1111", "4111 1111 1111", "411111111111", "4111111111111111", "123-45-6789", "123456789", "DE89 3704 0044 0532 0130 00", "DE89370400440532013000", "Glim must not read this")
$local = Join-Path $env:LOCALAPPDATA "Glim"
$resultsPath = Join-Path $local "capture-results.jsonl"
$glimLog = Join-Path $local "glim.log"
$resultsStart = if (Test-Path $resultsPath) { (Get-Content $resultsPath -Encoding utf8).Count } else { 0 }
$logStart = if (Test-Path $glimLog) { (Get-Content $glimLog -Encoding utf8).Count } else { 0 }

$work = Join-Path $env:TEMP "glim-verify-capture"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
$port = 8765
$fixtures = (Resolve-Path (Join-Path $PSScriptRoot "fixtures\capture-sensitive")).Path
$edgeExe = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
$server = $null; $keepass = $null

function StopGlim { Get-Process glim -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800 }
function StartGlim([bool]$dev) {
  $env:GLIM_DEV = if ($dev) { "1" } else { "" }
  Start-Process -FilePath $Exe | Out-Null
  $env:GLIM_DEV = ""
}
function StopEdge { Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object { $_.CommandLine -like "*$work*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue } }
function OpenEdge([string]$path) {
  Start-Process $edgeExe -ArgumentList "--user-data-dir=$work\edge", "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-sync", "--disable-component-update", "--force-renderer-accessibility",
    "--disable-features=CalculateNativeWinOcclusion", "--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding",
    "--window-position=-2400,-2400", "--window-size=900,700", "--new-window", "http://127.0.0.1:$port/$path"
}
$root = [System.Windows.Automation.AutomationElement]::RootElement
function WindowNamed([string]$part) {
  for ($i = 0; $i -lt 40; $i++) {
    foreach ($w in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
      if ($w.Current.Name -like "*$part*") { return $w }
    }
    Start-Sleep -Milliseconds 250
  }
  throw "window '$part' not found"
}
function ById($win, [string]$id) {
  $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
  for ($i = 0; $i -lt 40; $i++) {
    $e = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
    if ($e) { return $e }
    Start-Sleep -Milliseconds 250
  }
  throw "element '$id' not found"
}
function PanelText {
  $h = [CW]::Find($script:glimPid, "Capture debug")
  if ($h -eq [IntPtr]::Zero) { return "" }
  $p = [System.Windows.Automation.AutomationElement]::FromHandle($h)
  $names = foreach ($e in $p.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) { $e.Current.Name }
  return ([CW]::Title($h) + "`n" + ($names -join "`n"))
}
# Focus one field and check what the panel shows for it.
# Windows won't hand focus to a background script while someone is using the
# computer: then the panel shows another app, and the check is reported as
# inconclusive (never as a pass) and the run fails.
function Check([string]$label, $element, [string]$expectPattern, [bool]$expectText, [string]$app = "msedge.exe") {
  # Only while nobody is using the computer: wait for 4 s without input.
  $waited = 0
  while ([CW]::IdleMs() -lt 4000 -and $waited -lt 120) { Start-Sleep -Seconds 1; $waited++ }
  if ([CW]::IdleMs() -lt 4000) { Fail "$label`: INCONCLUSIVE, the computer was in use; focus not moved"; return }
  $h = [CW]::Find($script:glimPid, "Capture debug")
  $title = ""
  for ($try = 0; $try -lt 3 -and $title -notlike "* $app | Edit |*"; $try++) {
    try { $element.SetFocus() } catch { }
    for ($i = 0; $i -lt 10; $i++) { Start-Sleep -Milliseconds 300; $title = [CW]::Title($h); if ($title -like "* $app | Edit |*") { break } }
  }
  Start-Sleep -Milliseconds 700
  $title = [CW]::Title($h)
  if ($title -notlike "* $app | Edit |*") { Fail "$label`: INCONCLUSIVE, focus did not reach the field in $app (panel: $(($title -split '\|')[0..1] -join '|'))"; return }
  $text = PanelText
  $script:panelSeen += "`n" + $text
  Note ("{0}: {1}" -f $label, $title)
  if ($title -notlike "*| $expectPattern |*") { Fail "$label`: expected pattern '$expectPattern'" }
  $chars = if ($title -match "chars=(\d+)") { [int]$Matches[1] } else { -1 }
  if ($expectText -and $chars -le 0) { Fail "$label`: expected the text to be read" }
  if (-not $expectText -and ($chars -ne 0 -or $title -notlike "*readable=false*" -or $title -notlike "*showing=none*")) { Fail "$label`: expected nothing read" }
}

Note "exe: $Exe ($((Get-Item $Exe).Length) bytes, built $((Get-Item $Exe).LastWriteTime))"
$script:panelSeen = ""
try {
  $server = Start-Process -PassThru -WindowStyle Hidden python -ArgumentList "-I", "-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", "`"$fixtures`""
  StopGlim
  StartGlim $true
  Start-Sleep -Seconds 4
  $script:glimPid = (Get-Process glim | Sort-Object StartTime | Select-Object -Last 1).Id
  if ([CW]::Find($glimPid, "Capture debug") -eq [IntPtr]::Zero) { throw "no capture debug panel: is GLIM_DEV=1 reaching Glim?" }
  Note "dev Glim pid $glimPid, capture panel up"

  OpenEdge "notes.html"
  $w = WindowNamed "GLIM-VERIFY notes"
  $s = "skipped (sensitive)"; $p = "skipped (paused app or site)"
  Check "ordinary notes (phone, address, invoice)" (ById $w "t-ok") "TextPattern" $true
  Check "textarea holding a test card" (ById $w "t-card") $s $false
  Check "textarea holding a card being typed (12 digits)" (ById $w "t-partial") $s $false
  Check "textarea holding an SSN" (ById $w "t-ssn") $s $false
  Check "textarea holding an IBAN" (ById $w "t-iban") $s $false
  Check "field labelled 'Card number'" (ById $w "t-label") $s $false
  Check "field with id cc-exp" (ById $w "cc-exp") $s $false
  Check "notes in a /checkout/ frame" (ById $w "t-frame") $p $false
  Check "ordinary notes again (back to readable)" (ById $w "t-ok") "TextPattern" $true

  OpenEdge "checkout/index.html"
  $w = WindowNamed "GLIM-VERIFY checkout"
  Check "notes on a /checkout/ page" (ById $w "t-checkout") $p $false

  # A tiny GUI app compiled here and named like a password manager (no
  # console window), with one editable text box.
  Add-Type -OutputType WindowsApplication -OutputAssembly "$work\KeePass.exe" -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @'
using System; using System.Windows.Forms; using System.Drawing;
public static class P { [STAThread] public static void Main() {
  Application.EnableVisualStyles();
  var f = new Form { Text = "GLIM-VERIFY keepass", StartPosition = FormStartPosition.Manual, Location = new Point(-2400, -2400) };
  var t = new TextBox { Width = 300, Text = "Meeting notes for Friday" };
  f.Controls.Add(t); Application.Run(f); } }
'@
  $keepass = Start-Process -PassThru "$work\KeePass.exe"
  $w = WindowNamed "GLIM-VERIFY keepass"
  # The managed UIA client calls a WinForms box a Pane; find it by class.
  $box = $null
  for ($i = 0; $i -lt 40 -and -not $box; $i++) {
    $box = $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.ClassName -like "WindowsForms10.EDIT*" } | Select-Object -First 1
    if (-not $box) { Start-Sleep -Milliseconds 250 }
  }
  if (-not $box) { throw "KeePass.exe text box not found" }
  Check "text box in a paused app (KeePass.exe)" $box $p $false "KeePass.exe"
} finally {
  if ($keepass) { Stop-Process -Id $keepass.Id -Force -ErrorAction SilentlyContinue }
  StopEdge
  if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
  StopGlim
  StartGlim $false
  Note "stopped the dev Glim and started Glim normally"
}

# Nothing fake may show up anywhere it could have gone.
$newResults = if (Test-Path $resultsPath) { (Get-Content $resultsPath -Encoding utf8 | Select-Object -Skip $resultsStart) -join "`n" } else { "" }
$newLog = if (Test-Path $glimLog) { (Get-Content $glimLog -Encoding utf8 | Select-Object -Skip $logStart) -join "`n" } else { "" }
foreach ($place in @(@("debug panel", $script:panelSeen), @("results log", $newResults), @("glim.log", $newLog))) {
  $digitsOnly = ($place[1] -replace "\D", "")
  foreach ($f in $fakes) {
    $fDigits = $f -replace "\D", ""
    if ($place[1].Contains($f) -or ($fDigits.Length -ge 9 -and $digitsOnly.Contains($fDigits))) { Fail "'$f' found in the $($place[0])" }
  }
}
Note ("results log: {0} new lines, {1} skipped (sensitive), {2} skipped (paused)" -f (($newResults -split "`n") | Where-Object { $_ }).Count, ([regex]::Matches($newResults, "skipped \(sensitive\)")).Count, ([regex]::Matches($newResults, "skipped \(paused")).Count)
# Saved for the record: the fixture apps' lines only (other apps in use
# during the run are left out, even though their lines hold no text).
$fixtureLines = ($newResults -split "`n") | Where-Object { $_ -match '"app":"(msedge|KeePass)\.exe"' }
Set-Content -Path (Join-Path $Out "results-log-lines.jsonl") -Value $fixtureLines -Encoding utf8
if ($failures.Count) { Note "FAILED: $($failures.Count)"; $failures | ForEach-Object { Note " - $_" }; exit 1 }
Note "ALL CHECKS PASSED"
