<#
  Live check of the writing checker (Phase 1a) in Notepad, on the real app,
  with no mouse or keyboard input.

    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-writing-notepad.ps1 [-Exe <glim.exe>] [-Out <dir>]

  How it works:
  - Starts the built Glim with GLIM_DEV=1 and switches it to Normal (the
    pill: the count is drawn only there), then puts back the visibility the
    owner had saved and starts Glim normally at the end. The checker runs in
    every session for the apps it is on for, Notepad among them.
  - Writes a throwaway text file with two misspellings and opens it in
    Notepad. Refuses to run while Notepad is already open (the file would
    become a tab in the owner's window).
  - Focus moves with UI Automation SetFocus only, after 4 s without input.
  - Reads the suggestion count on the pill (#writing-count) from Glim's own
    window through UI Automation.
  - Then opens a second file with a clean sentence (a new tab) and checks the
    count goes away. Nothing is ever typed or edited: Notepad keeps unsaved
    edits per file path across launches, and each run's files get fresh
    names for the same reason.
  - Waits for the periodic log line ("writing: N checks ...", numbers only)
    and searches glim.log lines written during the run for the test text.
#>
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\glim.exe"),
  [string]$Out = (Join-Path $PSScriptRoot "..\..\docs\verification\writing-notepad")
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
using System; using System.Runtime.InteropServices;
public static class WV {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  public static IntPtr Find(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid) { var sb = new System.Text.StringBuilder(512); GetWindowText(h, sb, 512); if (sb.ToString() == title) { found = h; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
  [StructLayout(LayoutKind.Sequential)] struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
  [DllImport("user32")] static extern bool GetLastInputInfo(ref LASTINPUTINFO i);
  public static uint IdleMs() { var i = new LASTINPUTINFO(); i.cbSize = 8; GetLastInputInfo(ref i); return (uint)Environment.TickCount - i.dwTime; }
}
'@

$local = Join-Path $env:LOCALAPPDATA "Glim"
$glimLog = Join-Path $local "glim.log"
$logStart = if (Test-Path $glimLog) { (Get-Content $glimLog -Encoding utf8).Count } else { 0 }
$work = Join-Path $env:TEMP "glim-verify-writing"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
$stamp = Get-Date -Format "HHmmss"
$file = Join-Path $work "GLIM-VERIFY-writing-$stamp.txt"
$cleanFile = Join-Path $work "GLIM-VERIFY-clean-$stamp.txt"
# Words that must never reach the log.
$secretWords = @("recieved", "tomorow", "deposit", "received the payment")
Set-Content -Path $file -Value "We recieved the payment and will deposit it tomorow." -Encoding utf8
Set-Content -Path $cleanFile -Value "We got the payment and will bank it on Monday." -Encoding utf8
$root = [System.Windows.Automation.AutomationElement]::RootElement
$notepad = $null

function GlimDev([string[]]$argv) {
  $env:GLIM_DEV = "1"
  if ($argv.Count -gt 0) { Start-Process -FilePath $Exe -ArgumentList $argv | Out-Null } else { Start-Process -FilePath $Exe | Out-Null }
  $env:GLIM_DEV = ""
}
function StopGlim { Get-Process glim -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800 }
function WindowNamed([string]$part) {
  for ($i = 0; $i -lt 60; $i++) {
    foreach ($w in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
      if ($w.Current.Name -like "*$part*") { return $w }
    }
    Start-Sleep -Milliseconds 250
  }
  throw "window '$part' not found"
}
# The pill's count, or "" while it isn't showing (display:none leaves the tree).
function Badge {
  $pc = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$script:glimPid)
  $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, "writing-count")
  foreach ($win in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $pc)) {
    $e = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
    if (-not $e) { continue }
    $names = @($e.Current.Name) + @(foreach ($k in $e.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) { $k.Current.Name })
    # The label reads "3 writing suggestions"; the text is the number.
    foreach ($n in $names) { if ($n -match '^(\d+)') { return $Matches[1] } }
    return "?"
  }
  return ""
}
function WaitBadge([scriptblock]$ok, [int]$seconds) {
  $b = ""
  for ($i = 0; $i -lt $seconds * 4; $i++) { $b = Badge; if (& $ok $b) { return $b }; Start-Sleep -Milliseconds 250 }
  return $b
}
function WaitIdle([string]$label) {
  $waited = 0
  while ([WV]::IdleMs() -lt 4000 -and $waited -lt 120) { Start-Sleep -Seconds 1; $waited++ }
  if ([WV]::IdleMs() -lt 4000) { Fail "$label`: INCONCLUSIVE, the computer was in use; focus not moved"; return $false }
  return $true
}

Note "exe: $Exe ($((Get-Item $Exe).Length) bytes, built $((Get-Item $Exe).LastWriteTime))"
if (Get-Process notepad -ErrorAction SilentlyContinue) { throw "Notepad is already open: close it first (the test file would open as a tab in that window)" }
$settingsPath = Join-Path $env:APPDATA "Glim\settings.json"
$savedVis = "normal"
if ((Test-Path $settingsPath) -and ((Get-Content $settingsPath -Raw -Encoding utf8) -match '"visibility"\s*:\s*"([a-z]+)"')) { $savedVis = $Matches[1] }
$sawCount = $false
try {
  StopGlim
  GlimDev @()
  Start-Sleep -Seconds 6
  $script:glimPid = (Get-Process glim | Sort-Object StartTime | Select-Object -First 1).Id
  GlimDev @("--visibility", "normal")
  Start-Sleep -Seconds 2
  Note "dev Glim pid $glimPid, visibility normal (saved: $savedVis)"
  if ((Badge) -ne "") { Fail "a count shows before any text was focused" }

  $notepad = Start-Process -PassThru notepad.exe -ArgumentList "`"$file`""
  $w = WindowNamed "GLIM-VERIFY-writing-$stamp"
  $doc = $null
  foreach ($type in @([System.Windows.Automation.ControlType]::Document, [System.Windows.Automation.ControlType]::Edit)) {
    if ($doc) { break }
    $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
    for ($i = 0; $i -lt 20 -and -not $doc; $i++) { $doc = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c); if (-not $doc) { Start-Sleep -Milliseconds 250 } }
  }
  if (-not $doc) { throw "Notepad's text area not found" }
  Note "Notepad text area: $($doc.Current.ControlType.ProgrammaticName) class=$($doc.Current.ClassName)"

  if (WaitIdle "misspelled paragraph") {
    try { $doc.SetFocus() } catch { Note "SetFocus threw: $($_.Exception.Message)" }
    $b = WaitBadge { param($x) $x -ne "" } 10
    $focusedPid = [System.Windows.Automation.AutomationElement]::FocusedElement.Current.ProcessId
    if ($focusedPid -ne $notepad.Id -and -not (Get-Process -Id $focusedPid -ErrorAction SilentlyContinue | Where-Object Name -eq "notepad")) {
      Fail "misspelled paragraph: INCONCLUSIVE, focus did not reach Notepad"
    } elseif ($b -match '^\d+$' -and [int]$b -ge 2) {
      Note "misspelled paragraph: pill count $b (expected >= 2: recieved, tomorow)"
      $sawCount = $true
    } else {
      Fail "misspelled paragraph: pill count '$b', expected >= 2"
    }
  }

  # A clean sentence in a second file: a new tab in the same window.
  if (-not $sawCount) {
    Note "clean paragraph: SKIPPED, no count was showing to go away"
  } else {
    Start-Process notepad.exe -ArgumentList "`"$cleanFile`"" | Out-Null
    $w = WindowNamed "GLIM-VERIFY-clean-$stamp"
    $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $doc.Current.ControlType)
    $doc2 = $null
    for ($i = 0; $i -lt 20 -and -not $doc2; $i++) { $doc2 = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c); if (-not $doc2) { Start-Sleep -Milliseconds 250 } }
    if (-not $doc2) { Fail "clean paragraph: Notepad's text area not found" }
    elseif (WaitIdle "clean paragraph") {
      try { $doc2.SetFocus() } catch { }
      $b = WaitBadge { param($x) $x -eq "" } 10
      if ($b -eq "") { Note "clean paragraph: count gone" } else { Fail "clean paragraph: count still '$b'" }
    }
  }

  Note "waiting for the periodic writing log line (up to 35 s)"
  $statLine = $null
  for ($i = 0; $i -lt 35 -and -not $statLine; $i++) {
    Start-Sleep -Seconds 1
    $statLine = Get-Content $glimLog -Encoding utf8 | Select-Object -Skip $logStart | Where-Object { $_ -match "writing: \d+ checks" } | Select-Object -Last 1
  }
  if ($statLine) { Note "log: $($statLine -replace '^.*?(writing: )', '$1')" } else { Fail "no 'writing: N checks' line in glim.log" }
} finally {
  if ($notepad) {
    try { $w.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close(); Start-Sleep -Seconds 1 } catch { }
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  }
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  if (Get-Process glim -ErrorAction SilentlyContinue) { GlimDev @("--visibility", $savedVis); Start-Sleep -Seconds 2 }
  StopGlim
  Start-Process -FilePath $Exe | Out-Null
  Note "closed Notepad, removed the test file, visibility back to $savedVis, Glim started normally"
}

$newLog = if (Test-Path $glimLog) { (Get-Content $glimLog -Encoding utf8 | Select-Object -Skip $logStart) -join "`n" } else { "" }
foreach ($word in $secretWords) { if ($newLog -like "*$word*") { Fail "glim.log contains '$word'" } }
Note "glim.log searched for the test text: $($newLog.Split("`n").Count) new lines"

if ($failures.Count -eq 0) { Note "PASS"; exit 0 }
Note "$($failures.Count) failure(s)"
exit 1
