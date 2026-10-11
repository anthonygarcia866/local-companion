<#
  Typed check of 1b-apply (applying a writing fix) in Notepad, on the real
  app. THIS TYPES: it clicks a fix button through UI Automation, which makes
  Glim type into Notepad, then sends Ctrl+Z and Ctrl+S to that Notepad window.
  Run it ONLY after the owner has said in chat that they are away from the
  keyboard (CLAUDE.md). Notepad only.

    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-fix-notepad.ps1 [-Exe <glim.exe>] [-Out <dir>]

  Steps:
  - Refuses to start while Notepad is open, while the computer is in use
    (less than 4 s idle), or when the foreground window isn't Glim, Notepad,
    the desktop or this console.
  - Starts Glim (GLIM_DEV=1, visibility Normal), writes a fresh file
    "We recieved the payment today." and opens it in Notepad; focus moves
    with UI Automation SetFocus.
  - Waits for the count, then opens the list and clicks the fix button
    "Replace with <received>" with the dev switch `glim.exe --dev-fix
    received`: DOM events inside the island's page, as a mouse click on the
    non-activating island would be. Nothing in Glim's window is invoked
    through UI Automation first: a UIA Invoke activates Glim's window
    (Chromium), Windows' foreground lock then keeps it in front, and Glim
    rightly refuses the fix (the field's app isn't in front).
  - Reads Notepad's text back through UIA: "received" must be there and
    nothing else changed; the caret must be where it was.
  - Checks glim.log for "writing: fix applied (Notepad)".
  - Sends Ctrl+Z to Notepad (only while it is in front) and reads the text
    back: the typo must be back. Then Ctrl+S, so Notepad holds no unsaved
    edit, closes Notepad, removes the file, and starts Glim normally.
  - glim.log must not contain the test text.
#>
param(
  [string]$Exe = (Join-Path $env:LOCALAPPDATA "Glim\glim.exe"),
  [string]$Out = (Join-Path $PSScriptRoot "..\..\docs\verification\fix-notepad")
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
public static class FX {
  [StructLayout(LayoutKind.Sequential)] struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
  [DllImport("user32")] static extern bool GetLastInputInfo(ref LASTINPUTINFO i);
  public static uint IdleMs() { var i = new LASTINPUTINFO(); i.cbSize = 8; GetLastInputInfo(ref i); return (uint)Environment.TickCount - i.dwTime; }
  [DllImport("user32")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public static uint ForegroundPid() { uint p; GetWindowThreadProcessId(GetForegroundWindow(), out p); return p; }
  // Brings a window forward without any keystroke (UIA SetFocus alone can
  // move focus while the window stays behind Windows' foreground lock).
  [DllImport("user32")] public static extern void SwitchToThisWindow(IntPtr h, bool altTab);
  [DllImport("user32")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  // Ctrl + a letter key, down and up (keybd_event: test code only).
  public static void Ctrl(char key) {
    keybd_event(0x11, 0, 0, UIntPtr.Zero); keybd_event((byte)key, 0, 0, UIntPtr.Zero);
    keybd_event((byte)key, 0, 2, UIntPtr.Zero); keybd_event(0x11, 0, 2, UIntPtr.Zero);
  }
}
'@

$local = Join-Path $env:LOCALAPPDATA "Glim"
$glimLog = Join-Path $local "glim.log"
$logStart = if (Test-Path $glimLog) { (Get-Content $glimLog -Encoding utf8).Count } else { 0 }
$work = Join-Path $env:TEMP "glim-verify-fix"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
$stamp = Get-Date -Format "HHmmss"
$file = Join-Path $work "GLIM-FIX-$stamp.txt"
$original = "We recieved the payment today."
$expected = "We received the payment today."
$secretWords = @("recieved", "received the payment", "payment today")
# No BOM, no newline: the field holds exactly the sentence.
[System.IO.File]::WriteAllText($file, $original, (New-Object System.Text.UTF8Encoding $false))
$fixName = "Replace with " + [char]0x201C + "received" + [char]0x201D
$root = [System.Windows.Automation.AutomationElement]::RootElement
$notepad = $null; $w = $null

function GlimDev([string[]]$argv) {
  $env:GLIM_DEV = "1"
  if ($argv.Count -gt 0) { Start-Process -FilePath $Exe -ArgumentList $argv | Out-Null } else { Start-Process -FilePath $Exe | Out-Null }
  $env:GLIM_DEV = ""
}
function StopGlim { Get-Process glim -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800 }
function WindowNamed([string]$part) {
  for ($i = 0; $i -lt 60; $i++) {
    foreach ($x in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
      if ($x.Current.Name -like "*$part*") { return $x }
    }
    Start-Sleep -Milliseconds 250
  }
  throw "window '$part' not found"
}
function GlimFind([scriptblock]$match) {
  $pc = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$script:glimPid)
  foreach ($win in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $pc)) {
    foreach ($e in $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
      if (& $match $e) { return $e }
    }
  }
  return $null
}
function WaitGlim([scriptblock]$match, [int]$seconds) {
  for ($i = 0; $i -lt $seconds * 4; $i++) { $e = GlimFind $match; if ($e) { return $e }; Start-Sleep -Milliseconds 250 }
  return $null
}
function Badge {
  $e = GlimFind { param($x) $x.Current.AutomationId -eq "writing-count" }
  if (-not $e) { return "" }
  $names = @($e.Current.Name) + @(foreach ($k in $e.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) { $k.Current.Name })
  foreach ($n in $names) { if ($n -match '^(\d+)') { return $Matches[1] } }
  return "?"
}
function WaitIdle([string]$label) {
  $waited = 0
  while ([FX]::IdleMs() -lt 4000 -and $waited -lt 120) { Start-Sleep -Seconds 1; $waited++ }
  if ([FX]::IdleMs() -lt 4000) { Fail "$label`: the computer was in use; stopped"; return $false }
  return $true
}
# Notepad's text and caret, through UI Automation (read only).
function DocText { $doc.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern).DocumentRange.GetText(-1) }
function Caret {
  $tp = $doc.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
  $sel = $tp.GetSelection()
  if ($sel.Length -lt 1) { return -1 }
  $r = $tp.DocumentRange.Clone()
  $r.MoveEndpointByRange([System.Windows.Automation.Text.TextPatternRangeEndpoint]::End, $sel[0], [System.Windows.Automation.Text.TextPatternRangeEndpoint]::Start)
  return $r.GetText(-1).Length
}
function FrontName { (Get-Process -Id ([FX]::ForegroundPid()) -ErrorAction SilentlyContinue).Name }
function NotepadInFront { (FrontName) -eq "Notepad" }
# Notepad in front, by UIA SetFocus on its window, then SwitchToThisWindow;
# never a key or a click.
function BringNotepad {
  if (NotepadInFront) { return $true }
  try { $w.SetFocus() } catch { }
  Start-Sleep -Milliseconds 400
  if (NotepadInFront) { return $true }
  [FX]::SwitchToThisWindow([IntPtr]$w.Current.NativeWindowHandle, $true)
  Start-Sleep -Milliseconds 600
  if (NotepadInFront) { Note "Notepad brought to the front with SwitchToThisWindow" }
  return (NotepadInFront)
}

Note "exe: $Exe ($((Get-Item $Exe).Length) bytes, built $((Get-Item $Exe).LastWriteTime))"
if (Get-Process notepad -ErrorAction SilentlyContinue) { throw "Notepad is already open: close it first" }
$front = (Get-Process -Id ([FX]::ForegroundPid()) -ErrorAction SilentlyContinue).Name
Note "in front before the test: $front; idle $([FX]::IdleMs()) ms"
if ($front -notin @("Idle", "glim", "explorer", "WindowsTerminal", "powershell", "pwsh", "OpenConsole", "conhost", "claude")) { throw "another app ($front) is in front: stopping" }
if ([FX]::IdleMs() -lt 4000) { throw "the computer is in use: stopping" }

$settingsPath = Join-Path $env:APPDATA "Glim\settings.json"
$savedVis = "normal"
if ((Test-Path $settingsPath) -and ((Get-Content $settingsPath -Raw -Encoding utf8) -match '"visibility"\s*:\s*"([a-z]+)"')) { $savedVis = $Matches[1] }
try {
  StopGlim
  GlimDev @()
  Start-Sleep -Seconds 6
  $script:glimPid = (Get-Process glim | Sort-Object StartTime | Select-Object -First 1).Id
  GlimDev @("--visibility", "normal")
  Start-Sleep -Seconds 2
  Note "dev Glim pid $glimPid, visibility normal (saved: $savedVis)"

  $notepad = Start-Process -PassThru notepad.exe -ArgumentList "`"$file`""
  $w = WindowNamed "GLIM-FIX-$stamp"
  $doc = $null
  foreach ($type in @([System.Windows.Automation.ControlType]::Document, [System.Windows.Automation.ControlType]::Edit)) {
    if ($doc) { break }
    $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
    for ($i = 0; $i -lt 20 -and -not $doc; $i++) { $doc = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c); if (-not $doc) { Start-Sleep -Milliseconds 250 } }
  }
  if (-not $doc) { throw "Notepad's text area not found" }
  Note "Notepad text: '$(DocText)'"
  if (-not (WaitIdle "focus")) { throw "stopped" }
  if (-not (BringNotepad)) { throw "Notepad can't be brought to the front ($(FrontName) is): not applying" }
  $doc.SetFocus()
  $b = ""
  for ($i = 0; $i -lt 40 -and $b -eq ""; $i++) { $b = Badge; if ($b -eq "") { Start-Sleep -Milliseconds 250 } }
  if ($b -notmatch '^\d+$') { throw "no count on the pill ('$b')" }
  Note "pill count $b"

  if (-not (WaitIdle "apply")) { throw "stopped" }
  if (-not (BringNotepad)) { throw "Notepad is not in front ($(FrontName) is): not applying" }
  $doc.SetFocus()
  Start-Sleep -Milliseconds 400
  $caretBefore = Caret
  Note "caret before: $caretBefore"
  GlimDev @("--dev-fix", "received")
  $fixedNote = WaitGlim { param($e) $e.Current.Name -eq "Fixed. Ctrl+Z in Notepad undoes it." } 6
  Start-Sleep -Milliseconds 500
  $after = DocText
  $caretAfter = Caret
  if ($after -eq $expected) { Note "applied: text reads '$after'" } else { Fail "applied: text reads '$after', expected '$expected'" }
  if ($caretAfter -eq $caretBefore) { Note "caret back where it was ($caretAfter)" } else { Fail "caret $caretAfter, was $caretBefore" }
  if ($fixedNote) { Note "island note: '$($fixedNote.Current.Name)'" } else { Fail "island note 'Fixed. Ctrl+Z in Notepad undoes it.' not shown" }
  $newLog = (Get-Content $glimLog -Encoding utf8 | Select-Object -Skip $logStart) -join "`n"
  if ($newLog -match 'writing: fix applied \(Notepad\)') { Note "glim.log: 'writing: fix applied (Notepad)'" } else { Fail "glim.log has no 'writing: fix applied (Notepad)'" }

  if ((WaitIdle "undo") -and (BringNotepad)) {
    [FX]::Ctrl([char]'Z')
    Start-Sleep -Milliseconds 800
    $undone = DocText
    if ($undone -eq $original) { Note "Ctrl+Z (one press): text back to '$undone'" } else { Fail "after one Ctrl+Z the text reads '$undone'" }
    [FX]::Ctrl([char]'S')
    Start-Sleep -Milliseconds 800
    Note "Ctrl+S: saved, so Notepad keeps no unsaved edit"
  } else { Fail "undo: Notepad not in front or the computer in use; Ctrl+Z not sent" }
} catch {
  Fail "stopped: $($_.Exception.Message)"
} finally {
  if ($w) {
    try { $w.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close(); Start-Sleep -Seconds 1 } catch { }
  }
  if ($notepad) { Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }
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
