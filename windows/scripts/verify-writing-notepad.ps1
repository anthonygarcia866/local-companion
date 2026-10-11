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
    count goes away.
  - Then Ember, with a twelve-mistake paragraph: the ember's label counts
    them; a UI Automation Invoke on the ember (a click, no mouse) opens the
    list on "1-3 of N" without saving a mode, Next shows "4-6 of N", and
    Minimize brings the ember back.
  - Then a chat app: a tiny text box compiled here as slack.exe, holding chat
    slang and one real misspelling: exactly one suggestion. Nothing is ever typed or edited: Notepad keeps unsaved
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
$manyFile = Join-Path $work "GLIM-VERIFY-many-$stamp.txt"
# Words that must never reach the log.
$secretWords = @("recieved", "tomorow", "deposit", "received the payment", "notise", "kichen", "maintanance", "tmrw", "lol idk")
Set-Content -Path $file -Value "We recieved the payment and will deposit it tomorow." -Encoding utf8
Set-Content -Path $cleanFile -Value "We got the payment and will bank it on Monday." -Encoding utf8
# Twelve misspellings in one paragraph (for the pages and the ember).
Set-Content -Path $manyFile -Value "Teh tenant recieved the notise and will pay the balence tomorow. Plese send the reciept to the ownr and adress the maintanance reqest for the kichen sink." -Encoding utf8
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
# The first element in Glim's own windows that $match accepts.
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

  # Ember with many suggestions: the ember's label counts them, a click
  # (UI Automation Invoke, no mouse) opens the list without saving a mode,
  # Next turns the page, and closing the island brings the ember back.
  GlimDev @("--visibility", "ember")
  Start-Sleep -Seconds 2
  Start-Process notepad.exe -ArgumentList "`"$manyFile`"" | Out-Null
  $w = WindowNamed "GLIM-VERIFY-many-$stamp"
  $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $doc.Current.ControlType)
  $doc3 = $null
  for ($i = 0; $i -lt 20 -and -not $doc3; $i++) { $doc3 = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c); if (-not $doc3) { Start-Sleep -Milliseconds 250 } }
  if (-not $doc3) { Fail "ember: Notepad's text area not found" }
  elseif (WaitIdle "ember") {
    try { $doc3.SetFocus() } catch { }
    $ember = $null; $n = 0
    for ($i = 0; $i -lt 40 -and $n -lt 10; $i++) {
      $ember = GlimFind { param($e) $e.Current.AutomationId -eq "ember" }
      if ($ember -and $ember.Current.Name -match '^Glim: (\d+) writing suggestion') { $n = [int]$Matches[1] }
      if ($n -lt 10) { Start-Sleep -Milliseconds 250 }
    }
    if ($n -lt 10) { Fail "ember: label '$(if ($ember) { $ember.Current.Name })', expected 10+ writing suggestions" }
    else {
      Note "ember: label 'Glim: $n writing suggestions'"
      $dash = [string][char]0x2013
      $ember.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
      $first = "1${dash}3 of $n"
      $range = WaitGlim { param($e) $e.Current.Name -eq $first } 10
      if (-not $range) { Fail "ember click: the list with '$first' did not open" }
      else {
        Note "ember click: island open on the list, '$first'"
        $saved = (Get-Content $settingsPath -Raw -Encoding utf8) -match '"visibility"\s*:\s*"ember"'
        if (-not $saved) { Fail "ember click: the saved visibility changed (a peek must not save)" }
        $next = GlimFind { param($e) $e.Current.Name -eq "Next suggestions" }
        $next.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        $second = "4${dash}6 of $n"
        if (WaitGlim { param($e) $e.Current.Name -eq $second } 5) { Note "Next: '$second'" } else { Fail "Next: '$second' not shown" }
        $min = GlimFind { param($e) $e.Current.Name -eq "Minimize" }
        $min.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        $back = WaitGlim { param($e) $e.Current.AutomationId -eq "ember" -and $e.Current.BoundingRectangle.Width -gt 0 -and $e.Current.BoundingRectangle.Width -le 40 } 10
        if ($back) { Note "island closed: back to the ember ($($back.Current.Name))" } else { Fail "island closed: the ember did not come back" }
      }
    }
  }

  # A chat app: a tiny text box compiled here as slack.exe (as the capture
  # script's KeePass.exe). Chat slang and lowercase are let through; the one
  # real misspelling is still found. Read from the ember's label.
  Add-Type -OutputType WindowsApplication -OutputAssembly "$work\slack.exe" -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @'
using System; using System.Windows.Forms; using System.Drawing;
public static class P { [STAThread] public static void Main() {
  Application.EnableVisualStyles();
  var f = new Form { Text = "GLIM-VERIFY chat", StartPosition = FormStartPosition.Manual, Location = new Point(-2400, -2400) };
  var t = new TextBox { Width = 500, Text = "hey u coming tmrw? lol idk, i recieved it btw. ok ty" };
  f.Controls.Add(t); Application.Run(f); } }
'@
  $chat = Start-Process -PassThru "$work\slack.exe"
  try {
    $w = WindowNamed "GLIM-VERIFY chat"
    $box = $null
    for ($i = 0; $i -lt 40 -and -not $box; $i++) {
      $box = $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.ClassName -like "WindowsForms10.EDIT*" } | Select-Object -First 1
      if (-not $box) { Start-Sleep -Milliseconds 250 }
    }
    if (-not $box) { Fail "chat: text box not found" }
    elseif (WaitIdle "chat") {
      try { $box.SetFocus() } catch { }
      # Wait for the label to settle on this field (the previous field's
      # count clears on the focus change first).
      $label = ""
      for ($i = 0; $i -lt 40; $i++) {
        $e = GlimFind { param($x) $x.Current.AutomationId -eq "ember" }
        $label = if ($e) { $e.Current.Name } else { "" }
        if ($label -eq "Glim: 1 writing suggestion") { break }
        Start-Sleep -Milliseconds 250
      }
      if ($label -eq "Glim: 1 writing suggestion") { Note "chat (slack.exe): 1 suggestion (recieved); u, tmrw, lol, idk, i, btw, ok, ty let through" }
      else { Fail "chat (slack.exe): ember label '$label', expected 'Glim: 1 writing suggestion'" }
    }
  } finally { Stop-Process -Id $chat.Id -Force -ErrorAction SilentlyContinue }

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
