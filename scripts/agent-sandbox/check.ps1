# Agent sandbox check (docs/agent-sandbox-plan.md §2): proves what glim-agent
# can't reach. Run by the owner, NOT elevated:
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
#     yours), any drive root or top-level folder on any drive, any folder
#     setup.ps1 locks (targets.ps1), nor in a locked tree's immediate
#     subfolders;
#   - (when run elevated) it holds no powerful user right (backup, restore,
#     debug, take ownership, impersonate, create symbolic links ...) directly
#     or through any group or logon SID; not elevated, this is skipped and
#     said so;
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

# ── Powerful user rights (LSA; needs elevation) ─────────────────────────────
$elevated = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($elevated) {
    $pow = @(Get-GlimPowerfulRights -Sid $sid -GroupSids $groupSids)
    Report ($pow.Count -eq 0) ('holds no powerful user right' + $(if ($pow) { ': ' + ($pow -join ', ') } else { '' }))
} else {
    Write-Host 'skip  powerful user rights: listing them needs elevation (setup.ps1 checks them; run check.ps1 elevated to include them). Not counted as a pass.' -ForegroundColor Yellow
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
}

# ── Must not write ───────────────────────────────────────────────────────────
$mustNotWrite = [ordered]@{}
$mustNotWrite[$env:USERPROFILE.ToLowerInvariant()] = $env:USERPROFILE
foreach ($p in (Get-GlimPathFolders)) { $mustNotWrite[$p.ToLowerInvariant()] = $p }
$targets = @(Get-GlimLockTargets -OwnerProfile $env:USERPROFILE)
foreach ($t in $targets) {
    $mustNotWrite[$t.Path.ToLowerInvariant()] = $t.Path
    if ($t.Scope -eq 'tree') {
        foreach ($c in (Get-ChildItem -LiteralPath $t.Path -Directory -Force -ErrorAction SilentlyContinue)) { $mustNotWrite[$c.FullName.ToLowerInvariant()] = $c.FullName }
    }
}
# Every top-level folder on every fixed/removable drive, whatever the lock
# list says (so a folder the list's own test missed still fails here).
foreach ($d in (Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -in 2, 3 })) {
    $root = $d.DeviceID + '\'
    if (-not (Test-Path -LiteralPath $root)) { continue }
    $mustNotWrite[$root.ToLowerInvariant()] = $root
    foreach ($c in (Get-ChildItem -LiteralPath $root -Directory -Force -ErrorAction SilentlyContinue)) {
        if ($c.FullName -like 'C:\GlimAgent*') { continue }
        $mustNotWrite[$c.FullName.ToLowerInvariant()] = $c.FullName
    }
}
# The named folders must be on the list even if they're no longer writable by all.
foreach ($n in $GlimNamed) { if (Test-Path -LiteralPath $n) { $mustNotWrite[$n.ToLowerInvariant()] = $n } }
foreach ($p in $mustNotWrite.Values) {
    if ($p -like 'C:\GlimAgent*') { continue }
    $m = Access $p
    if ($null -eq $m) { Report $false "can't write $p (its permissions couldn't be read: not verified)"; continue }
    $w = $m -band $GlimWrite
    Report ($w -eq 0) ("can't write or delete in $p" + $(if ($w) { ' (has 0x{0:X})' -f $w } else { '' }))
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
if ($fail -eq 0) { Write-Host "All $checked checks passed." -ForegroundColor Green; exit 0 }
Write-Host "$fail of $checked check(s) failed: sandbox sessions must stay off." -ForegroundColor Red
exit 1
