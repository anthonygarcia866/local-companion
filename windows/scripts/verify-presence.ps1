<#
  Live verification of the glim-presence work (pill sizes, startup ignite, the
  attention pop, docks, visibility modes, the chat palette, and the focus /
  click-through guarantees), on the real app, with no mouse or keyboard input.

    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-presence.ps1 [-Exe <glim.exe>] [-Out <dir>] [-SkipChat]

  How it works:
  - Glim is driven only by its dev switches (GLIM_DEV=1 in the running Glim;
    a second glim.exe forwards --dock / --visibility / --fullscreen /
    --mascot-state / --dev-chat to it). Pill sizes are set in settings.json
    while Glim is stopped (backed up first, restored at the end).
  - Screenshots are PrintWindow captures of Glim's own island window only
    (never the screen). Transparent parts come out black.
  - After every step it reads the island window's real state: visible,
    WS_EX_NOACTIVATE (never takes focus), WS_EX_TRANSPARENT (click-through),
    and whose window is in the foreground (never Glim's, except the chat step,
    where the chat field takes focus on purpose; it must be released after).
  - --dev-chat sends one prompt to the local Ollama (localhost only).
  Ends by stopping the dev Glim, restoring settings.json, and starting Glim
  normally (no GLIM_DEV).
#>
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\glim.exe"),
  [string]$Out = (Join-Path $PSScriptRoot "..\..\docs\verification\presence"),
  [switch]$SkipChat
)
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$log = Join-Path $Out "verify-log.txt"
Set-Content -Path $log -Value "" -Encoding utf8
function Note([string]$s) { $line = "{0:HH:mm:ss.fff} {1}" -f (Get-Date), $s; Write-Host $line; Add-Content -Path $log -Value $line -Encoding utf8 }

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System; using System.Collections.Generic; using System.Drawing; using System.Drawing.Imaging; using System.Runtime.InteropServices;
public static class GW {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32", EntryPoint = "GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
  [DllImport("user32")] static extern bool GetCursorPos(out POINT p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static List<IntPtr> WindowsOf(uint pid, string title) {
    var o = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid) { var sb = new System.Text.StringBuilder(256); GetWindowText(h, sb, 256); if (sb.ToString() == title) o.Add(h); }
      return true; }, IntPtr.Zero);
    return o;
  }
  public static int[] Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return new[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
  public static long ExStyle(IntPtr h) { return GetWindowLongPtr(h, -20).ToInt64(); }
  public static uint PidOf(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
  public static int[] Cursor() { POINT p; GetCursorPos(out p); return new[] { p.X, p.Y }; }
  // PrintWindow with PW_RENDERFULLCONTENT (WebView2 content), scaled up `zoom` times.
  public static int[] Capture(IntPtr h, string path, int zoom) {
    var r = Rect(h); if (r[2] <= 0 || r[3] <= 0) return new[] { 0, 0 };
    using (var bmp = new Bitmap(r[2], r[3], PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
      // The lantern's drawn height: rows with a bright pixel (the island itself is black).
      int top = -1, bottom = -1;
      for (int y = 0; y < bmp.Height; y++) for (int x = 0; x < bmp.Width; x += 2) {
        var c = bmp.GetPixel(x, y); if (c.R + c.G + c.B > 180) { if (top < 0) top = y; bottom = y; break; } }
      if (zoom <= 1) bmp.Save(path, ImageFormat.Png);
      else using (var big = new Bitmap(bmp.Width * zoom, bmp.Height * zoom)) {
        using (var g = Graphics.FromImage(big)) { g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.NearestNeighbor; g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.Half; g.DrawImage(bmp, 0, 0, big.Width, big.Height); }
        big.Save(path, ImageFormat.Png); }
      return new[] { top, bottom < 0 ? 0 : bottom - top + 1 };
    }
  }
}
'@

$WS_EX_TRANSPARENT = 0x20; $WS_EX_NOACTIVATE = 0x08000000
$settingsPath = Join-Path $env:APPDATA "Glim\settings.json"
$backup = "$settingsPath.verify-bak"
$failures = New-Object System.Collections.Generic.List[string]
function Fail([string]$s) { $failures.Add($s); Note "FAIL: $s" }

function GlimPid { (Get-Process glim -ErrorAction SilentlyContinue | Where-Object Path -eq $Exe | Select-Object -First 1).Id }
# The island and the desktop character window are both titled "Glim". The
# island is the one laid out as the full panel (>= 600 px wide) once Glim has
# started; its handle is kept, so it is still found as the 28 px ember or hidden.
$script:island = [IntPtr]::Zero
function Island {
  $p = GlimPid; if (-not $p) { return [IntPtr]::Zero }
  if ($script:island -ne [IntPtr]::Zero -and [GW]::PidOf($script:island) -eq $p) { return $script:island }
  foreach ($h in [GW]::WindowsOf([uint32]$p, "Glim")) { if ([GW]::Rect($h)[2] -ge 600) { $script:island = $h; return $h } }
  return [IntPtr]::Zero
}
function Forward([string[]]$a) { Start-Process -FilePath $Exe -ArgumentList $a -Wait; Start-Sleep -Milliseconds 150 }
function Shot([string]$name, [int]$zoom = 1) {
  $h = Island; if ($h -eq [IntPtr]::Zero) { Fail "no island window for $name"; return $null }
  $m = [GW]::Capture($h, (Join-Path $Out "$name.png"), $zoom); $r = [GW]::Rect($h)
  Note ("shot {0}: window {1}x{2} at {3},{4}; lantern rows {5}" -f $name, $r[2], $r[3], $r[0], $r[1], $m[1])
  return $m
}
# The ba8d079 guarantees, read from the real window.
function Check([string]$step, [bool]$expectVisible = $true, [bool]$chatFocus = $false) {
  $h = Island; $fg = [GW]::GetForegroundWindow(); $fgPid = [GW]::PidOf($fg); $glim = GlimPid
  if ($h -eq [IntPtr]::Zero) { Fail "${step}: island window missing"; return }
  $vis = [GW]::IsWindowVisible($h); $ex = [GW]::ExStyle($h); $r = [GW]::Rect($h); $c = [GW]::Cursor()
  $noact = ($ex -band $WS_EX_NOACTIVATE) -ne 0; $through = ($ex -band $WS_EX_TRANSPARENT) -ne 0
  $over = $c[0] -ge $r[0] -and $c[0] -lt $r[0] + $r[2] -and $c[1] -ge $r[1] -and $c[1] -lt $r[1] + $r[3]
  Note ("check {0}: visible={1} noactivate={2} clickthrough={3} rect={4}x{5}@{6},{7} cursor-over-window={8} foreground-is-glim={9}" -f $step, $vis, $noact, $through, $r[2], $r[3], $r[0], $r[1], $over, ($fgPid -eq $glim))
  if ($vis -ne $expectVisible) { Fail "${step}: visible=$vis, expected $expectVisible" }
  if (-not $chatFocus) {
    if (-not $noact) { Fail "${step}: WS_EX_NOACTIVATE missing" }
    if ($fgPid -eq $glim) { Fail "${step}: Glim is the foreground window" }
  }
  return @{ through = $through; over = $over; rect = $r }
}
function StopGlim { Get-Process glim -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800 }
function StartGlim([bool]$dev = $true) {
  $script:island = [IntPtr]::Zero
  $env:GLIM_DEV = if ($dev) { "1" } else { "" }
  Start-Process -FilePath $Exe | Out-Null
  $env:GLIM_DEV = ""
}
function SetSetting([string]$key, $value) {
  $j = Get-Content $settingsPath -Raw -Encoding utf8 | ConvertFrom-Json
  $j | Add-Member -NotePropertyName $key -NotePropertyValue $value -Force
  [IO.File]::WriteAllText($settingsPath, ($j | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding $false))
}

Note "exe: $Exe ($((Get-Item $Exe).Length) bytes, built $((Get-Item $Exe).LastWriteTime))"
$startFg = [GW]::GetForegroundWindow()
Note "foreground at start: pid $([GW]::PidOf($startFg))"
StopGlim
Copy-Item $settingsPath $backup -Force
try {
  # ── 1. Pill sizes, and the startup ignite (frames of every launch, kept for large) ──
  foreach ($size in "small", "medium", "large") {
    SetSetting "pillSize" $size
    SetSetting "visibility" "normal"
    StartGlim
    $t0 = Get-Date; while ((Island) -eq [IntPtr]::Zero -and ((Get-Date) - $t0).TotalSeconds -lt 10) { Start-Sleep -Milliseconds 20 }
    $seen = Get-Date
    $frames = @()
    while (((Get-Date) - $seen).TotalMilliseconds -lt 1600) {
      $ms = [int]((Get-Date) - $seen).TotalMilliseconds
      if ($size -eq "large") { $m = Shot ("ignite-{0:D4}ms" -f $ms) 2 } else { $m = $null }
      $frames += $ms; Start-Sleep -Milliseconds 90
    }
    Start-Sleep -Milliseconds 1500
    Shot "size-$size" 2 | Out-Null
    Check "size-$size" | Out-Null
    if ($size -ne "large") { StopGlim }
  }

  # ── Hotkeys: the startup log line says what registered ──
  $glimLog = Join-Path $env:LOCALAPPDATA "Glim\glim.log"
  $line = Get-Content $glimLog -Tail 400 | Where-Object { $_ -like "*shortcuts: registered*" } | Select-Object -Last 1
  Note "hotkeys at startup: $line"
  foreach ($k in "openChat=Ctrl+Alt+Shift+C", "cycleVisibility=Ctrl+Alt+Shift+Space") {
    if (-not $line -or -not $line.Contains($k) -or $line.IndexOf($k) -gt $line.IndexOf("; not registered")) { Fail "hotkey not registered at startup: $k" }
  }

  # ── 2. The attention pop: a burst around one state change ──
  Forward @("--mascot-state", "idle"); Start-Sleep -Milliseconds 900
  $rest = Shot "pop-before" 3
  $job = Start-Process -FilePath $Exe -ArgumentList "--mascot-state", "think" -PassThru
  $best = 0; $t0 = Get-Date; $i = 0
  while (((Get-Date) - $t0).TotalMilliseconds -lt 1500) {
    $m = [GW]::Capture((Island), (Join-Path $Out ("pop-burst-{0:D2}.png" -f $i)), 3)
    if ($m[1] -gt $best) { $best = $m[1]; Copy-Item (Join-Path $Out ("pop-burst-{0:D2}.png" -f $i)) (Join-Path $Out "pop-peak.png") -Force }
    $i++; Start-Sleep -Milliseconds 25
  }
  $job.WaitForExit()
  Start-Sleep -Milliseconds 800
  $after = Shot "pop-after" 3
  Note ("pop: lantern height at rest {0}px, peak {1}px, after {2}px (x3 captures)" -f $rest[1], $best, $after[1])
  if ($best -le $after[1]) { Fail "pop: no frame taller than the settled lantern" }
  Get-ChildItem $Out -Filter "pop-burst-*.png" | Remove-Item
  Forward @("--mascot-state", "auto")

  # ── 3. Docks ──
  foreach ($d in "top-left", "top-right", "left-vertical", "right-vertical", "top-center") {
    Forward @("--dock", $d); Start-Sleep -Milliseconds 1200
    Shot "dock-$d" | Out-Null
    Check "dock-$d" | Out-Null
  }

  # ── 4. Visibility modes, fullscreen auto-hide, and the click-through fixes ──
  Forward @("--visibility", "ember"); Start-Sleep -Milliseconds 900
  Shot "vis-ember" 8 | Out-Null
  $e = Check "vis-ember"
  if ($e.through) { Fail "ember is click-through: it must take the mouse over its whole window" }
  Forward @("--visibility", "hidden"); Start-Sleep -Milliseconds 900
  Check "vis-hidden" $false | Out-Null
  Forward @("--visibility", "normal"); Start-Sleep -Milliseconds 1200
  Shot "vis-normal" | Out-Null
  $n = Check "vis-normal"
  # The panel must not swallow clicks just because the cursor didn't move since it came back.
  if (-not $n.over -and -not $n.through) { Fail "back to normal with the cursor elsewhere: the panel is not click-through" }
  Forward @("--fullscreen", "on"); Start-Sleep -Milliseconds 900
  Check "fullscreen-on" $false | Out-Null
  Forward @("--fullscreen", "off"); Start-Sleep -Milliseconds 1200
  Check "fullscreen-off" | Out-Null
  Forward @("--fullscreen", "auto"); Start-Sleep -Milliseconds 600

  # ── 5. Palette (the chat glow) and focus released after the chat ──
  if (-not $SkipChat) {
    Forward @("--dev-chat", "Reply with the single word: ok")
    Start-Sleep -Seconds 12
    Shot "palette-after-chat" | Out-Null
    Check "chat (focus on purpose)" $true $true | Out-Null
    Forward @("--visibility", "ember"); Start-Sleep -Milliseconds 900
    $h = Island; $ex = [GW]::ExStyle($h)
    Note ("after chat -> ember: noactivate={0}" -f (($ex -band $WS_EX_NOACTIVATE) -ne 0))
    if (($ex -band $WS_EX_NOACTIVATE) -eq 0) { Fail "focus not released after the chat (no WS_EX_NOACTIVATE in ember)" }
    Forward @("--visibility", "normal"); Start-Sleep -Milliseconds 900
    # The chat field had the keyboard on purpose; Windows keeps Glim in front
    # until something else is clicked (a script may not take the foreground
    # back). What must hold: Glim no longer *takes* focus.
    $h = Island; $ex = [GW]::ExStyle($h)
    Note ("after chat -> normal: noactivate={0} (foreground stays where the chat left it)" -f (($ex -band $WS_EX_NOACTIVATE) -ne 0))
    if (($ex -band $WS_EX_NOACTIVATE) -eq 0) { Fail "focus not released after the chat (back to normal)" }
  }
} finally {
  StopGlim
  Copy-Item $backup $settingsPath -Force; Remove-Item $backup
  StartGlim $false
  Note "restored settings.json and started Glim normally"
}
if ($failures.Count) { Note "FAILED: $($failures.Count)"; $failures | ForEach-Object { Note " - $_" }; exit 1 }
Note "ALL CHECKS PASSED"
