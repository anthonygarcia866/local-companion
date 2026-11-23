# Agent sandbox check (docs/agent-sandbox-plan.md §2): proves what glim-agent
# can't reach. Run by the owner, ELEVATED (Run as administrator), or user
# rights and folders only SYSTEM can open stay unverified and the result is
# INCOMPLETE:
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\check.ps1
#
# Every PASS/FAIL is computed on YOUR side, from glim-agent's SID and each
# object's own permissions, with Windows' AuthZ access check (authz.ps1).
# Nothing runs as glim-agent for these, so nothing glim-agent left in its own
# profile (a profiler DLL, a planted module, a running process) can change
# the answer (review 2026-11-22: an earlier probe that ran as glim-agent
# could be made to lie). The one thing run as glim-agent is `whoami`, to show
# its logon works; that line is information only and never counts as a PASS.
#
# It FAILS unless:
#   - glim-agent is in no local group but Users;
#   - it can't read your profile, your gh / wrangler / Claude / Claude2 / Glim
#     folders, the login files in them, or G:\;
#   - it can't write or delete in your profile, any PATH folder (system or
#     yours, quotes stripped, links followed to the real folder), any folder
#     above one, any program directly in one, any drive root, top-level folder or root file on any drive, any
#     folder setup.ps1 locks (targets.ps1, which includes subfolders deep in
#     a locked tree whose inheritance is off), nor in a locked tree's
#     immediate subfolders;
#   - no local group but Users includes Everyone, INTERACTIVE or another SID
#     every logon carries;
#   - it holds no user right beyond a short harmless allowlist (change
#     notify, working set, time zone, shutdown, undock, the ordinary logon
#     rights, any deny right), directly or through any group or logon SID.
#     This needs elevation: not elevated, the run ends INCOMPLETE (exit 2),
#     never as a pass;
#   - none of CLOUDFLARE_API_TOKEN, SUPABASE_ACCESS_TOKEN, ANTHROPIC_API_KEY,
#     GH_TOKEN, GITHUB_TOKEN is set machine-wide (every user would get it);
#   - it CAN write C:\GlimAgent\work.
# Nothing is changed.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'cred.ps1')
. (Join-Path $PSScriptRoot 'authz.ps1')
. (Join-Path $PSScriptRoot 'targets.ps1')

$AgentName = 'glim-agent'
$u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if (-not $u) { throw "$AgentName doesn't exist: run setup.ps1 first." }
$sid = $u.SID.Value

$fail = 0
$checked = 0
$incomplete = $false
function Report([bool]$ok, [string]$what) {
    $script:checked++
    if ($ok) { Write-Host "PASS  $what" -ForegroundColor Green } else { Write-Host "FAIL  $what" -ForegroundColor Red; $script:fail++ }
}
function Access([string]$p) {
    try { return Get-GlimAccess $sid $p } catch { return $null }
}

# ── Groups (from the account database, not from a token it controls) ───────
$adsiUser = [ADSI]"WinNT://$env:COMPUTERNAME/$AgentName,user"
$groupSids = @(foreach ($g in @($adsiUser.Groups())) {
    (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value
})
Report ($groupSids -contains 'S-1-5-32-545') 'is a member of Users'
$other = @($groupSids | Where-Object { $_ -ne 'S-1-5-32-545' })
Report ($other.Count -eq 0) ('is in no other local group' + $(if ($other) { ': ' + ($other -join ', ') } else { '' }))
# A group can also hold a SID every logon carries (Everyone, INTERACTIVE ...),
# which makes every user a member: only Users may.
$logonSids = [GlimAuthz]::LogonSids()
$viaLogon = @(foreach ($g in @(([ADSI]"WinNT://$env:COMPUTERNAME").Children | Where-Object { $_.SchemaClassName -eq 'group' })) {
    $gSid = (New-Object Security.Principal.SecurityIdentifier ($g.objectSid.Value), 0).Value
    if ($gSid -eq 'S-1-5-32-545') { continue }
    foreach ($m in @($g.Invoke('Members'))) {
        try { $mSid = (New-Object Security.Principal.SecurityIdentifier ($m.GetType().InvokeMember('objectSid', 'GetProperty', $null, $m, $null)), 0).Value } catch { continue }
        if ($logonSids -contains $mSid) { "$($g.Name.Value) holds $mSid" }
    }
})
Report ($viaLogon.Count -eq 0) ('no other local group includes Everyone, INTERACTIVE or another logon SID' + $(if ($viaLogon) { ': ' + ($viaLogon -join '; ') } else { '' }))

# ── Powerful user rights (LSA; needs elevation) ─────────────────────────────
$elevated = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($elevated) {
    $pow = @(Get-GlimPowerfulRights -Sid $sid -GroupSids $groupSids)
    Report ($pow.Count -eq 0) ('holds no powerful user right' + $(if ($pow) { ': ' + ($pow -join ', ') } else { '' }))
} else {
    Write-Host 'NOT VERIFIED  user rights: listing them needs elevation (run check.ps1 elevated). The result below is INCOMPLETE, never a pass.' -ForegroundColor Yellow
    $script:incomplete = $true
}

# ── Must not read ────────────────────────────────────────────────────────────
$mustNotRead = @(
    $env:USERPROFILE,
    (Join-Path $env:USERPROFILE '.config\gh'),
    (Join-Path $env:USERPROFILE '.config\gh\hosts.yml'),
    (Join-Path $env:APPDATA 'GitHub CLI'),
    (Join-Path $env:APPDATA 'GitHub CLI\hosts.yml'),
    (Join-Path $env:APPDATA 'xdg.config\.wrangler'),
    (Join-Path $env:APPDATA 'xdg.config\.wrangler\config\default.toml'),
    (Join-Path $env:USERPROFILE '.claude'),
    (Join-Path $env:USERPROFILE '.claude\.credentials.json'),
    (Join-Path $env:USERPROFILE '.claude2'),
    (Join-Path $env:USERPROFILE '.claude2\.credentials.json'),
    (Join-Path $env:USERPROFILE '.claude.json'),
    (Join-Path $env:LOCALAPPDATA 'Glim'),
    (Join-Path $env:APPDATA 'Glim')
)
if (Test-Path 'G:\') { $mustNotRead += 'G:\' }
foreach ($p in $mustNotRead) {
    if (-not (Test-Path -LiteralPath $p)) { Write-Host "n/a   $p (not on this PC)" -ForegroundColor DarkGray; continue }
    $m = Access $p
    if ($null -eq $m) { Report $false "can't read $p (its permissions couldn't be read: not verified)"; continue }
    Report (($m -band $GlimRead) -eq 0) "can't read $p"
    # A link (review 9): what it leads to must be closed too.
    if ([GlimFs]::IsLink($p)) {
        $to = [GlimAuthz]::FinalPath($p)
        if (-not $to) { Report $false "can't read $p (a link whose target couldn't be resolved: not verified)"; continue }
        $mt = Access $to
        Report ($null -ne $mt -and ($mt -band $GlimRead) -eq 0) "can't read $p's target $to"
    }
}

# Every file and folder in your profile (fix 2): glim-agent may read or
# write none of them (walked in full, ~1M items here, a few minutes).
Write-Host 'walking your profile (a few minutes)...' -ForegroundColor DarkGray
$pc = New-Object GlimAuthz+Checker $sid
try { $pw = [GlimFs]::Walk($env:USERPROFILE, 'access', $null, [uint32]($GlimRead -bor $GlimWrite), $pc) } finally { $pc.Dispose() }
Report ($pw.Hits.Count -eq 0) ("nothing in your profile is readable or writable by it ($($pw.Items) items)" + $(if ($pw.Hits.Count) { ": $($pw.Hits.Count), e.g. " + (($pw.Hits | Select-Object -First 10) -join '; ') } else { '' }))
$brokenUnverified = @()
# Links in your profile aren't entered; one that leads OUTSIDE it is checked
# at its target (review 9: a secrets folder turned into a link elsewhere).
$outside = @(foreach ($l in $pw.Links) {
    $to = [GlimAuthz]::FinalPath($l)
    if (-not $to) {
        # A broken link: what it points at doesn't exist. If glim-agent could
        # create that path, whoever follows the link would read its files, so
        # the nearest existing folder on the way must be closed to it.
        $raw = [GlimFs]::LinkTarget($l)   # read from the link itself (PowerShell's .Target can't always)
        if (-not $raw -or $raw -notmatch '^[A-Za-z]:\\') { $script:brokenUnverified += $l; continue }
        if (Test-GlimUnder $raw $env:USERPROFILE) { continue }
        $par = $raw
        while ($par -and -not (Test-Path -LiteralPath $par)) { $par = Split-Path -Parent $par }
        $mp = if ($par) { Access $par } else { $null }
        if ($null -eq $mp) { $script:brokenUnverified += $l; continue }
        if ($mp -band $GlimWrite) { "$l -> $raw (broken; it could create it under $par)" }
        continue
    }
    if (Test-GlimUnder $to $env:USERPROFILE) { continue }
    $mt = Access $to
    if ($null -eq $mt) { "$l -> $to (permissions unreadable)" }
    elseif ($mt -band ($GlimRead -bor $GlimWrite)) { "$l -> $to (0x{0:X})" -f ($mt -band ($GlimRead -bor $GlimWrite)) }
})
Report ($outside.Count -eq 0) ("no link in your profile leads to something it can read or write ($($pw.Links.Count) links)" + $(if ($outside) { ': ' + ($outside -join '; ') } else { '' }))
if ($brokenUnverified.Count) {
    Write-Host ("NOT VERIFIED  $($brokenUnverified.Count) broken link(s) in your profile whose target couldn't be read, e.g. " + (($brokenUnverified | Select-Object -First 5) -join '; ')) -ForegroundColor Yellow
    $script:incomplete = $true
}
if ($pw.Unreadable.Count -or $pw.Unlistable.Count) {
    Write-Host ("NOT VERIFIED  in your profile: $($pw.Unreadable.Count) item(s) whose permissions couldn't be read, $($pw.Unlistable.Count) folder(s) that couldn't be listed, e.g. " + ((@($pw.Unreadable) + @($pw.Unlistable) | Select-Object -First 10) -join '; ')) -ForegroundColor Yellow
    $script:incomplete = $true
}

# ── Must not write ───────────────────────────────────────────────────────────
$mustNotWrite = [ordered]@{}
$mustNotWrite[$env:USERPROFILE.ToLowerInvariant()] = $env:USERPROFILE
foreach ($p in (Get-GlimPathFolders)) { $mustNotWrite[$p.ToLowerInvariant()] = $p }
foreach ($p in (Get-GlimMissingPathParents)) { $mustNotWrite[$p.ToLowerInvariant()] = $p }
foreach ($p in (Get-GlimPathAncestors)) { $mustNotWrite[$p.ToLowerInvariant()] = $p }
foreach ($p in (Get-GlimPathFiles)) { $mustNotWrite[$p.ToLowerInvariant()] = $p }
foreach ($x in $GlimPathUnresolved) {
    Write-Host "NOT VERIFIED  PATH entry that doesn't resolve to a real folder: $x" -ForegroundColor Yellow
    $script:incomplete = $true
}
$targets = @(Get-GlimLockTargets -OwnerProfile $env:USERPROFILE)
foreach ($l in $GlimLinkFindings) { Report $false "no link that all users can change: $l" }
foreach ($t in $targets) {
    $mustNotWrite[$t.Path.ToLowerInvariant()] = $t.Path
    if ($t.Scope -eq 'tree') {
        # (protected subfolders deeper down are targets of their own, so they're on this list too)
        foreach ($c in (Get-ChildItem -LiteralPath $t.Path -Directory -Force -ErrorAction SilentlyContinue)) { $mustNotWrite[$c.FullName.ToLowerInvariant()] = $c.FullName }
    }
}
# Every top-level folder on every fixed/removable drive, whatever the lock
# list says (so a folder the list's own test missed still fails here).
foreach ($d in (Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -in 2, 3 })) {
    $root = $d.DeviceID + '\'
    if (-not (Test-Path -LiteralPath $root)) { continue }
    $mustNotWrite[$root.ToLowerInvariant()] = $root
    $errs = $null
    $rootKids = @(Get-ChildItem -LiteralPath $root -Force -ErrorAction SilentlyContinue -ErrorVariable errs)
    if ($errs) { Write-Host "NOT VERIFIED  couldn't list $root" -ForegroundColor Yellow; $script:incomplete = $true }
    foreach ($c in $rootKids) {
        if ($c.FullName -ieq 'C:\GlimAgent') { continue }
        $mustNotWrite[$c.FullName.ToLowerInvariant()] = $c.FullName
    }
}
# The named folders must be on the list even if they're no longer writable by all.
foreach ($n in $GlimNamed) { if (Test-Path -LiteralPath $n) { $mustNotWrite[$n.ToLowerInvariant()] = $n } }
foreach ($p in $mustNotWrite.Values) {
    if ($p -ieq 'C:\GlimAgent' -or $p -like 'C:\GlimAgent\*') { continue }
    if (Test-GlimInUse $p) {
        $e0 = 0
        if ($null -eq [GlimFs]::ReadSd($p, [ref]$e0) -and $e0 -eq 32) { Write-Host "info  $p is in use by Windows (nobody, glim-agent included, can open it)" -ForegroundColor DarkGray; continue }
    }
    $m = Access $p
    if ($null -eq $m) { Report $false "can't write $p (its permissions couldn't be read: not verified)"; continue }
    $w = $m -band $GlimWrite
    Report ($w -eq 0) ("can't write or delete in $p" + $(if ($w) { ' (has 0x{0:X})' -f $w } else { '' }))
}

# Every file and folder inside every locked tree, walked in full (an item
# with its own allow entry beats the tree's inherited deny).
Write-Host 'walking every locked tree (about a minute)...' -ForegroundColor DarkGray
$walk = Get-GlimStillWritable -Sid $sid -Targets $targets
Report ($walk.Writable.Count -eq 0) ('nothing inside the locked folders is writable' + $(if ($walk.Writable.Count) { ": $($walk.Writable.Count), e.g. " + (($walk.Writable | Select-Object -First 10) -join '; ') } else { '' }))
if ($walk.Unreadable.Count) {
    Write-Host ("NOT VERIFIED  $($walk.Unreadable.Count) item(s) whose permissions couldn't be read or whose contents couldn't be listed, e.g. " + (($walk.Unreadable | Select-Object -First 10) -join '; ')) -ForegroundColor Yellow
    $script:incomplete = $true
}

# ── Its own environment can't hold your tokens machine-wide ─────────────────
foreach ($v in 'CLOUDFLARE_API_TOKEN', 'SUPABASE_ACCESS_TOKEN', 'ANTHROPIC_API_KEY', 'GH_TOKEN', 'GITHUB_TOKEN') {
    Report (-not [Environment]::GetEnvironmentVariable($v, 'Machine')) "$v is not set machine-wide"
}

# ── Its work folder ──────────────────────────────────────────────────────────
$wm = Access 'C:\GlimAgent\work'
Report ($null -ne $wm -and ($wm -band 0x6) -eq 0x6) 'can write C:\GlimAgent\work'

# ── Information only: its logon works ───────────────────────────────────────
$plain = Read-GlimCredential 'Glim/AgentUserPassword'
if ($plain) {
    $cred = New-Object Management.Automation.PSCredential (".\$AgentName", (ConvertTo-SecureString $plain -AsPlainText -Force))
    $plain = $null
    $out = Join-Path $env:TEMP ('glim-whoami-' + [guid]::NewGuid().ToString('N') + '.txt')
    try {
        Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\whoami.exe') -Credential $cred -WorkingDirectory 'C:\GlimAgent\work' -WindowStyle Hidden -RedirectStandardOutput $out -Wait
        $who = if (Test-Path $out) { (Get-Content $out -Raw).Trim() } else { '' }
        Write-Host "info  logon as $AgentName works: $who" -ForegroundColor DarkGray
    } catch { Write-Host "info  logon as $AgentName failed: $($_.Exception.Message)" -ForegroundColor Yellow }
    finally { Remove-Item $out -Force -ErrorAction SilentlyContinue }
} else { Write-Host "info  Glim/AgentUserPassword is not in your Credential Manager" -ForegroundColor Yellow }

Write-Host ''
if ($checked -lt 10) { Write-Host "Only $checked checks ran: not trusted." -ForegroundColor Red; exit 1 }
if ($fail -gt 0) { Write-Host "$fail of $checked check(s) failed: sandbox sessions must stay off." -ForegroundColor Red; exit 1 }
if ($incomplete) { Write-Host "INCOMPLETE: $checked checks passed, but something above is NOT VERIFIED (run check.ps1 elevated; tell Claude about any item still unreadable)." -ForegroundColor Yellow; exit 2 }
Write-Host "All $checked checks passed." -ForegroundColor Green
exit 0
