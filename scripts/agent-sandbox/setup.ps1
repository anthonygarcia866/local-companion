# Agent sandbox setup (docs/agent-sandbox-plan.md §1-2; owner decision a, 2026-11-22).
#
# Run once by the owner, in an ELEVATED Windows PowerShell (one UAC prompt):
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\setup.ps1 -DryRun   # shows every step, changes nothing
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\setup.ps1           # does them
#
# What it does (and nothing else):
#   1. Creates the local user glim-agent (member of Users only), with a random
#      32-character password that nobody sees: it goes straight into the
#      OWNER's Credential Manager as Glim/AgentUserPassword (never on screen,
#      disk or a log).
#   2. Hides glim-agent from the sign-in screen.
#   3. Denies it "Log on through Remote Desktop Services" and "Access this
#      computer from the network" (secedit, by SID).
#   4. Creates C:\GlimAgent\work: only SYSTEM, Administrators, the owner
#      (full) and glim-agent (modify).
#   5. Adds a deny ACE for glim-agent on the owner's profile folder.
#   6. Denies glim-agent (only) write + delete on every folder in
#      targets.ps1's list: C:\tests, C:\glimtest, C:\c, C:\Drivers, every
#      top-level folder on a fixed/removable drive and every PATH folder
#      outside your profile that all users can write, and each drive root
#      (that folder only). The dry run prints the exact list. Your access
#      and other users' don't change.
#
# Safe to run again: an existing glim-agent keeps its password (use
# -RotatePassword to make a new one), and each step checks before it acts.
# Undo: scripts\agent-sandbox\remove.ps1.

[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$RotatePassword
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$AgentName = 'glim-agent'
$Root = 'C:\GlimAgent'
$Work = Join-Path $Root 'work'
$CredTarget = 'Glim/AgentUserPassword'

. (Join-Path $PSScriptRoot 'cred.ps1')
. (Join-Path $PSScriptRoot 'targets.ps1')
. (Join-Path $PSScriptRoot 'authz.ps1')

function Step([string]$what) { Write-Host ("- " + $what) }
function Act([string]$what, [scriptblock]$do) {
    if ($DryRun) { Write-Host ("  would: " + $what) -ForegroundColor Yellow }
    else { Write-Host ("  doing: " + $what); & $do }
}

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin -and -not $DryRun) { throw 'Run this from an elevated PowerShell (Run as administrator).' }
$Owner = $id.Name                       # DOMAIN\user of the person running it
$OwnerProfile = $env:USERPROFILE
if ($Owner -match '\\(SYSTEM|glim-agent)$') { throw "Run this as yourself, not as $Owner." }
Write-Host "Owner: $Owner  Profile: $OwnerProfile  DryRun: $DryRun"

# ── 1. The user ──────────────────────────────────────────────────────────────
Step "1. Local user $AgentName"
$user = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
function New-AgentPassword {
    # 32 characters from a 64-symbol alphabet (256 mod 64 = 0: no bias), from
    # the OS's CSPRNG; drawn again until it has upper, lower, digit and symbol,
    # so Windows' complexity policy (if on) never refuses it.
    $alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!#%+=?@'.ToCharArray()
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    while ($true) {
        $bytes = New-Object byte[] 32
        $rng.GetBytes($bytes)
        $s = -join ($bytes | ForEach-Object { $alphabet[$_ % $alphabet.Length] })
        if ($s -cmatch '[A-Z]' -and $s -cmatch '[a-z]' -and $s -match '[0-9]' -and $s -match '[!#%+=?@]') { return $s }
    }
}
if (-not $user) {
    Act "create $AgentName (Users only), random password into your Credential Manager as $CredTarget" {
        $plain = New-AgentPassword
        $pw = ConvertTo-SecureString $plain -AsPlainText -Force
        # Saved first: if creating the user fails, a stale entry is harmless;
        # the other way round would leave a user whose password nobody knows.
        Write-GlimCredential -Target $CredTarget -Secret $plain
        $plain = $null
        New-LocalUser -Name $AgentName -Password $pw -PasswordNeverExpires -UserMayNotChangePassword -AccountNeverExpires -Description 'Glim agent sandbox' | Out-Null
    }
} elseif ($RotatePassword) {
    Act "new random password for $AgentName, saved over $CredTarget" {
        $plain = New-AgentPassword
        Set-LocalUser -Name $AgentName -Password (ConvertTo-SecureString $plain -AsPlainText -Force)
        Write-GlimCredential -Target $CredTarget -Secret $plain
        $plain = $null
    }
} else {
    Write-Host "  exists (password kept)"
    if (-not (Test-GlimCredential -Target $CredTarget)) { Write-Warning "  $CredTarget is missing from your Credential Manager: run again with -RotatePassword" }
}

# A member of Users (New-LocalUser adds it to no group at all, and without
# Users it can't log on locally: found on the owner's first run), and in no
# other local group (Administrators, Backup Operators, Remote Desktop Users,
# Hyper-V Administrators, Remote Management Users ...).
# By the user's own group list (ADSI), which doesn't trip over orphaned SIDs
# in a group the way Get-LocalGroupMember does.
if ($user -or -not $DryRun) {
    $u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
    if ($u) {
        $adsiUser = [ADSI]"WinNT://$env:COMPUTERNAME/$AgentName,user"
        $inUsers = $false
        foreach ($g in @($adsiUser.Groups())) {
            $gs0 = (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value
            if ($gs0 -eq 'S-1-5-32-545') { $inUsers = $true }
        }
        if (-not $inUsers) {
            $usersName = (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-545').Translate([Security.Principal.NTAccount]).Value.Split('\')[-1]
            Act "add $AgentName to $usersName" { ([ADSI]"WinNT://$env:COMPUTERNAME/$usersName,group").Add($adsiUser.Path) }
        }
        foreach ($g in @($adsiUser.Groups())) {
            $gName = $g.GetType().InvokeMember('Name', 'GetProperty', $null, $g, $null)
            $gSid = (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value
            if ($gSid -ne 'S-1-5-32-545') {
                Act "remove $AgentName from $gName ($gSid)" { ([ADSI]"WinNT://$env:COMPUTERNAME/$gName,group").Remove($adsiUser.Path) }
            }
        }
    }
}

# glim-agent's SID, kept in YOUR registry (only you can write it) as soon as
# the user exists, so remove.ps1 can clean up even if setup stops part way.
$u0 = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if ($u0) {
    $k0 = 'HKCU:\Software\Glim\AgentSandbox'
    $saved0 = Get-ItemProperty -Path $k0 -Name Sid -ErrorAction SilentlyContinue
    if (-not $saved0 -or $saved0.Sid -ne $u0.SID.Value) {
        Act "remember $AgentName's SID in $k0" {
            if (-not (Test-Path $k0)) { New-Item -Path $k0 -Force | Out-Null }
            New-ItemProperty -Path $k0 -Name Sid -Value $u0.SID.Value -PropertyType String -Force | Out-Null
        }
    }
}

# ── 1b. glim-agent stopped while its sandbox is (re)built ───────────────────
# Review 3 (H-2): Windows checks access when a handle is OPENED, so a handle a
# glim-agent process opened before this run (WRITE_DAC on an all-users Full
# Control folder, say) would still work after every lock below. So: account
# disabled, every process it runs stopped and none left, BEFORE anything is
# built or locked; re-enabled at the very end, only if everything passed. If
# setup stops part way, glim-agent stays disabled (safe; run setup again).
$agentStopped = $false
if (Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue) {
    Step "1b. Stop $AgentName while its sandbox is built (enabled again at the end, only if everything passes)"
    Act "disable $AgentName and stop every process it runs" {
        Disable-LocalUser -Name $AgentName
        $its = {
            @(Get-CimInstance Win32_Process | Where-Object {
                $o = try { $_ | Invoke-CimMethod -MethodName GetOwner } catch { $null }
                $o -and $o.User -eq $AgentName -and $o.Domain -eq $env:COMPUTERNAME
            })
        }
        foreach ($p in (& $its)) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 500
        $left = @(& $its)
        if ($left.Count) { throw "$($left.Count) $AgentName process(es) wouldn't stop ($((($left | ForEach-Object Name) -join ', '))). Restart Windows, then run setup.ps1 again." }
        $script:agentStopped = $true
    }
}

# ── 2. Hidden from the sign-in screen ────────────────────────────────────────
Step '2. Hidden from the sign-in screen'
$ul = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList'
$cur = (Get-ItemProperty -Path $ul -Name $AgentName -ErrorAction SilentlyContinue)
if (-not $cur -or $cur.$AgentName -ne 0) {
    Act "set $ul\$AgentName = 0" {
        if (-not (Test-Path $ul)) { New-Item -Path $ul -Force | Out-Null }
        New-ItemProperty -Path $ul -Name $AgentName -Value 0 -PropertyType DWord -Force | Out-Null
    }
} else { Write-Host '  already hidden' }

# ── 3. Logon rights ──────────────────────────────────────────────────────────
Step '3. Deny Remote Desktop logon and network logon'
$agentUser = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
$agentSid = if ($agentUser) { $agentUser.SID.Value } else { $null }
if (-not $agentSid) { Write-Host '  (after step 1 creates the user)' }
elseif (-not $isAdmin) { Write-Host '  (not elevated: secedit can only read the rights elevated; skipped in this dry run)' -ForegroundColor Yellow }
else {
    $tmp = Join-Path $env:TEMP ('glim-secedit-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        secedit /export /cfg "$tmp\cur.inf" /areas USER_RIGHTS /quiet | Out-Null
        if (-not (Test-Path -LiteralPath "$tmp\cur.inf")) { throw "secedit /export wrote nothing (exit $LASTEXITCODE)" }
        $lines = Get-Content "$tmp\cur.inf"
        $need = @()
        foreach ($right in 'SeDenyRemoteInteractiveLogonRight', 'SeDenyNetworkLogonRight') {
            $line = $lines | Where-Object { $_ -match "^$right\s*=" }
            $held = if ($line) { @((($line -split '=', 2)[1]) -split ',' | ForEach-Object { $_.Trim() }) } else { @() }
            if ($held -notcontains "*$agentSid" -and $held -notcontains $AgentName -and $held -notcontains "$env:COMPUTERNAME\$AgentName") {
                $existing = if ($line) { ($line -split '=', 2)[1].Trim() } else { '' }
                $value = if ($existing) { "$existing,*$agentSid" } else { "*$agentSid" }
                $need += "$right = $value"
            }
        }
        if ($need.Count -eq 0) { Write-Host '  already denied' }
        else {
            Act ("secedit: " + (($need | ForEach-Object { ($_ -split ' = ')[0] }) -join ', ')) {
                @('[Unicode]', 'Unicode=yes', '[Version]', 'signature="$CHICAGO$"', 'Revision=1', '[Privilege Rights]') + $need | Set-Content -Path "$tmp\new.inf" -Encoding Unicode
                secedit /configure /db "$tmp\new.sdb" /cfg "$tmp\new.inf" /areas USER_RIGHTS /quiet | Out-Null
                if ($LASTEXITCODE -ne 0) { throw "secedit failed ($LASTEXITCODE)" }
            }
        }
    } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
}

# ── 4. The workspace ─────────────────────────────────────────────────────────
Step "4. $Work (SYSTEM, Administrators, you: full; ${AgentName}: modify inside it, nothing more on it or $Root)"
# Review 2026-11-26 (HIGH-A): icacls is never run on an existing $Root: it
# re-stamps everything inside from the folder's ACL, so a hard link glim-agent
# planted there would hand it Modify on the linked file. Both folders are
# created in one step with their final permissions (CreateDirSddl). On them
# glim-agent gets only what it needs to reach and fill $Work: list/traverse
# $Root; list/traverse + add files and folders on $Work, but no delete,
# rename or permission change of either; Modify on what's INSIDE $Work
# (inherit-only). $Work also holds $Keep, a file glim-agent can read but not
# delete, so $Work is never empty and can't be turned into a junction (review
# 3 M-3: an empty folder can, with just add-file rights). $Root holds nothing
# but $Work (review 3 L-6). Re-runs only read all three back; anything else
# stops setup.
$Keep = Join-Path $Work '.glim-keep'
if (-not $agentSid) { Write-Host '  (after step 1 creates the user)' }
else {
    $me = $id.User.Value
    $rootSddl = "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;$me)(A;;0x1200a9;;;$agentSid)"
    $workSddl = "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;$me)(A;;0x1200af;;;$agentSid)(A;OICIIO;0x1301bf;;;$agentSid)"
    $keepSddl = "D:PAI(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;$me)(A;;0x120089;;;$agentSid)"
    $owners = [string[]]@('S-1-5-18', 'S-1-5-32-544', $me)
    function New-Keep {
        # Created by this elevated run (owner: Administrators), then given its
        # own protected DACL through the non-propagating writer.
        $fs = [IO.File]::Open($Keep, 'CreateNew', 'Write', 'None'); $fs.Close()
        $rc = [GlimFs]::SetDaclSddl($Keep, $keepSddl); if ($rc -ne 0) { throw "couldn't set $Keep's permissions (error $rc)" }
    }
    $rootWhy = if (Test-Path -LiteralPath $Root) { [GlimFs]::SameDir($Root, $rootSddl, $owners) } else { 'missing' }
    $workWhy = if (Test-Path -LiteralPath $Work) { [GlimFs]::SameDir($Work, $workSddl, $owners) } else { 'missing' }
    if ($rootWhy -eq '' -and $workWhy -eq '') {
        if (Test-Path -LiteralPath $Keep) { Write-Host '  already set' }
        else { Act "add $Keep (keeps $Work from ever being empty)" { New-Keep } }   # setups before review 3
    }
    elseif ($rootWhy -eq 'missing') {
        Act "create $Root and $Work with their permissions" {
            $rc = [GlimFs]::CreateDirSddl($Root, $rootSddl); if ($rc -ne 0) { throw "couldn't create $Root (error $rc)" }
            $rc = [GlimFs]::CreateDirSddl($Work, $workSddl); if ($rc -ne 0) { throw "couldn't create $Work (error $rc)" }
            New-Keep
        }
    } else {
        # An older setup's layout (glim-agent had Modify on $Root itself) is
        # rebuilt only while there is nothing in it but an empty, real $Work
        # (then no item exists for a permission change to reach), and only
        # after step 1b stopped glim-agent: no process of its is left, and no
        # handle to $Root is open anywhere (review 3 H-1: a handle held from
        # before could turn the emptied $Root into a junction mid-rebuild).
        $kids = @(Get-ChildItem -LiteralPath $Root -Force -ErrorAction Stop)
        $emptyWork = $kids.Count -eq 0 -or ($kids.Count -eq 1 -and $kids[0].Name -eq 'work' -and [GlimFs]::IsRealDir($Work) -and
            @(Get-ChildItem -LiteralPath $Work -Force -ErrorAction Stop).Count -eq 0)
        $rootReal = [GlimFs]::IsRealDir($Root) -and ([GlimFs]::SameDir($Root, $rootSddl, $owners) -notmatch '^(owner is|it is a link|not a folder|can''t read)')
        if (-not ($emptyWork -and $rootReal)) {
            throw "$Root isn't as setup makes it ($Root`: $(if ($rootWhy) { $rootWhy } else { 'ok' }); $Work`: $(if ($workWhy) { $workWhy } else { 'ok' })) and isn't empty. Run remove.ps1 (it also deletes $AgentName), then setup.ps1 again; tell Claude if this keeps happening."
        }
        Act "rebuild $Root and $Work (empty) with their permissions" {
            if (-not $agentStopped) { throw "$AgentName wasn't stopped first (step 1b); not rebuilding $Root" }
            if (-not [GlimFs]::NoOtherHandles($Root) -or ((Test-Path -LiteralPath $Work) -and -not [GlimFs]::NoOtherHandles($Work))) {
                throw "something still has $Root or $Work open; restart Windows, then run setup.ps1 again"
            }
            if (Test-Path -LiteralPath $Work) { [IO.Directory]::Delete($Work, $false) }   # an empty folder only
            $rc = [GlimFs]::SetDaclSddl($Root, $rootSddl); if ($rc -ne 0) { throw "couldn't set $Root's permissions (error $rc)" }
            if (-not [GlimFs]::IsRealDir($Root)) { throw "$Root changed during the rebuild" }
            $rc = [GlimFs]::CreateDirSddl($Work, $workSddl); if ($rc -ne 0) { throw "couldn't create $Work (error $rc)" }
            New-Keep
        }
    }
    if (-not $DryRun) {
        $rootWhy = [GlimFs]::SameDir($Root, $rootSddl, $owners); $workWhy = [GlimFs]::SameDir($Work, $workSddl, $owners)
        $keepWhy = [GlimFs]::SameFile($Keep, $keepSddl, $owners)
        $others = @(Get-ChildItem -LiteralPath $Root -Force | Where-Object { $_.Name -ne 'work' })
        if ($rootWhy -or $workWhy -or $keepWhy -or $others.Count) {
            throw "after step 4: $Root $rootWhy; $Work $workWhy; $Keep $keepWhy; $(if ($others.Count) { "$Root also holds: " + (($others | ForEach-Object Name) -join ', ') })"
        }
    }
}

# ── 5. Your profile ──────────────────────────────────────────────────────────
Step "5. Deny $AgentName on $OwnerProfile"
$hasDeny = $false
if ($agentSid) {
    $hasDeny = [bool]((Get-Acl $OwnerProfile).Access | Where-Object { $_.AccessControlType -eq 'Deny' -and $(try { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq $agentSid } catch { $false }) })
}
if ($hasDeny) { Write-Host '  already denied' }
else {
    # icacls re-stamps the whole profile (review 3 L-5): only while nothing in
    # it lets all users write (a hard link planted there would be re-stamped).
    if (-not $DryRun) {
        $bw = @(Get-GlimExplicitWritable $OwnerProfile)
        if ($bw.Count) { throw "$($bw.Count) item(s) in your profile let all users write (e.g. $($bw[0].Path)); tell Claude before setup puts its deny on your profile." }
    }
    Act "icacls $OwnerProfile /deny ${AgentName}:(OI)(CI)F" {
        # icacls puts the deny on the folder and propagates it to every child
        # that inherits (it can take a while on a big profile). A file or
        # folder with inheritance turned off doesn't get it: check.ps1 computes
        # glim-agent's access to the actual login files to catch that.
        icacls $OwnerProfile /deny "${AgentName}:(OI)(CI)F" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls $OwnerProfile failed ($LASTEXITCODE)" }
    }
}

# ── 6. Lock what everyone can write (owner 2026-11-22) ──────────────────────
Step "6. Deny $AgentName write + delete $GlimDenyRights on these folders (nobody else's access changes)"
$targets = @(Get-GlimLockTargets -OwnerProfile $OwnerProfile)
foreach ($x in $GlimPathUnresolved) { Write-Warning "  PATH entry that doesn't resolve to a real folder (NOT verified): $x" }
if ($GlimUnlistable.Count) {
    Write-Warning ("  $($GlimUnlistable.Count) item(s) couldn't be listed or read while finding what to lock, so anything below or in them isn't on the list (check.ps1 stays INCOMPLETE until they're readable; run elevated). E.g. " + (($GlimUnlistable | Select-Object -First 5) -join '; '))
}
if ($GlimLinkFindings.Count) {
    $GlimLinkFindings | ForEach-Object { Write-Host "  LINK all users could change (not locked automatically: icacls would change its target): $_" -ForegroundColor Red }
    if (-not $DryRun) { throw "$($GlimLinkFindings.Count) link(s) all users could change. Tell Claude the list; they need fixing by hand before setup can finish." }
}
$lockFailed = New-Object System.Collections.Generic.List[string]
function Add-LockFailed([string]$x) { if (-not $lockFailed.Contains($x)) { $lockFailed.Add($x) } }
function Get-LockWhy([int]$rc) {
    switch ($rc) {
        -2 { 'a file with more than one hard link: its other names may be anywhere, so it is never changed' }
        -3 { 'a null DACL (everyone allowed): never changed' }
        default { "Windows error $rc" }
    }
}
foreach ($t in $targets) {
    $need = if ($t.Deny -eq 'all') { 0x1F01FF } else { 0xD0156 }
    $isTree = $t.Scope -eq 'tree'
    $label = "{0}  [{1}; deny {2}]  ({3})" -f $t.Path, $t.Scope, $(if ($t.Deny -eq 'all') { 'everything' } else { 'write+delete' }), $t.Why
    if ($agentSid -and [GlimFs]::HasDeny($t.Path, $agentSid, [uint32]$need, $isTree)) { Write-Host "  already locked: $label"; continue }
    Act "lock $label" {
        # Not icacls (review 2026-11-26, H1): it goes through the Windows layer
        # that re-stamps every item below from its parent, which a hard link
        # planted in an all-users folder turns into "Users: Modify" on the
        # linked file, wherever it is. AddDeny writes this one object's DACL
        # only (owner and flags unchanged, nothing passed down); the walk below
        # then gives each item inside that glim-agent can still write its own.
        $sidNow = (Get-LocalUser -Name $AgentName).SID.Value
        $rc = [GlimFs]::AddDeny($t.Path, $sidNow, [uint32]$need, $isTree)
        if ($rc -ne 0 -or -not [GlimFs]::HasDeny($t.Path, $sidNow, [uint32]$need, $isTree)) {
            Add-LockFailed "$($t.Path) ($(Get-LockWhy $rc))"
            Write-Host "    NOT LOCKED: $($t.Path) ($(Get-LockWhy $rc)); carrying on with the rest" -ForegroundColor Red
        }
    }
}
if ($targets.Count -eq 0) { Write-Host '  nothing to lock' }
# Every run, by result: every target, trees walked in full, glim-agent's real
# access computed on this side (AuthZ). Each item it can still write gets its
# own deny, then everything is walked again (review M3/L1: an item a past run
# missed is found again, never hidden behind an "already locked" root).
$uNow = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue   # StrictMode: never .SID on a missing user
if ($uNow) { $agentSid = $uNow.SID.Value }
if ($DryRun) { Write-Host '  (then: every folder inside without an inheritable deny, and every item still writable by it, gets its own deny; then all is walked again)' }
elseif ($agentSid) {
    # Review M-a: every real folder in a tree must pass a deny on to what is
    # created in it later (a service's new file would otherwise be writable).
    Write-Host '  checking: every folder in the locked trees passes the deny on...'
    $nd = 0
    foreach ($t in $targets) {
        if ($t.Scope -ne 'tree') { continue }
        $need = if ($t.Deny -eq 'all') { 0x1F01FF } else { 0xD0156 }
        $r = [GlimFs]::Walk($t.Path, 'nodeny', [string[]]@($agentSid), [uint32]$need, $null)
        foreach ($h in $r.Hits) {
            $rc = [GlimFs]::AddDeny($h, $agentSid, [uint32]$need, $true)
            if ($rc -eq 0) { $nd++ } else { Add-LockFailed "$h ($(Get-LockWhy $rc))" }
        }
    }
    if ($nd) { Write-Host "  gave $nd folder(s) inside an inheritable deny" }
    Write-Host '  checking: walking every locked tree (about a minute)...'
    $v = Get-GlimStillWritable -Sid $agentSid -Targets $targets
    if ($v.Writable.Count) {
        Write-Host "  $($v.Writable.Count) item(s) inside are still writable by it: giving each its own deny"
        foreach ($p in $v.Writable) {
            # The target it's in decides the deny (the longest path that holds it).
            $own = $null
            foreach ($t in $targets) {
                $b = $t.Path.TrimEnd('\')
                if ($p -ieq $b -or $p.StartsWith("$b\", [StringComparison]::OrdinalIgnoreCase)) { if (-not $own -or $b.Length -gt $own.Path.TrimEnd('\').Length) { $own = $t } }
            }
            $need = if ($own -and $own.Deny -eq 'all') { 0x1F01FF } else { 0xD0156 }
            # The item's own attributes, a link never followed (review LOW).
            $isDir = [GlimFs]::IsRealDir($p)   # read literally, never a link (review 3 L-7)
            $rc = [GlimFs]::AddDeny($p, $agentSid, [uint32]$need, $isDir)
            if ($rc -ne 0) { Add-LockFailed "$p ($(Get-LockWhy $rc))" }
        }
        Write-Host '  verifying: walking every locked tree again...'
        $v = Get-GlimStillWritable -Sid $agentSid -Targets $targets
    }
    if ($lockFailed.Count) { $lockFailed | ForEach-Object { Write-Host "  NOT LOCKED: $_" -ForegroundColor Red } }
    if ($v.Unreadable.Count) { Write-Warning ("  $($v.Unreadable.Count) item(s) couldn't be read or listed (NOT verified), e.g. " + (($v.Unreadable | Select-Object -First 5) -join '; ')) }
    if ($v.Writable.Count) {
        $v.Writable | Select-Object -First 50 | ForEach-Object { Write-Host "  STILL WRITABLE: $_" -ForegroundColor Red }
        throw "$($v.Writable.Count) item(s) are still writable by $AgentName after locking. No sandbox session until they're fixed (tell Claude the list; setup is safe to run again). An item $AgentName OWNS can't be locked by a deny (its owner may always change its permissions): those need their owner changed by hand."
    }
    if ($v.Unreadable.Count) { Write-Host "  checked: nothing readable in the locked folders is writable by it, but $($v.Unreadable.Count) item(s) were NOT verified (see above; check.ps1 will end INCOMPLETE until they are)" -ForegroundColor Yellow }
    else { Write-Host '  verified: nothing in the locked folders is writable by it' }
}
if ($lockFailed.Count) {
    throw "$($lockFailed.Count) item(s) couldn't be locked (listed above). Setup isn't finished; tell Claude the list (setup is safe to run again)."
}

# ── 7. No powerful user rights (read back, elevated) ─────────────────────────
Step "7. $AgentName holds no user right beyond the harmless allowlist"
if ($agentSid -and $isAdmin) {
    $gs = @(foreach ($g in @(([ADSI]"WinNT://$env:COMPUTERNAME/$AgentName,user").Groups())) { (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value })
    $pow = @(Get-GlimPowerfulRights -Sid $agentSid -GroupSids $gs)
    if ($pow.Count) { throw ("$AgentName holds: " + ($pow -join ', ') + ". Remove them in secpol.msc (Local Policies > User Rights Assignment), then run setup.ps1 again. No sandbox session until then.") }
    else { Write-Host '  none' }
} else { Write-Host '  (checked after step 1 creates the user, in an elevated run)' }

Write-Host ''
# Everything passed: glim-agent may run again (step 1b stopped it).
if (-not $DryRun -and $agentStopped) { Act "enable $AgentName again" { Enable-LocalUser -Name $AgentName } }
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
else { Write-Host 'Done. Next: scripts\agent-sandbox\check.ps1 (no UAC needed).' -ForegroundColor Green }
