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
Step "4. $Work (SYSTEM, Administrators, you: full; ${AgentName}: modify; nobody else)"
# Only while it isn't set up yet: icacls passes the ACL down through
# everything inside, and once glim-agent has worked there, a hard link it
# planted would get the folder's ACL (review 2026-11-26, H1). Already set =
# protected, and exactly these four allows, nothing else.
$rootOk = $false
if (Test-Path -LiteralPath $Work) {
    $ra = Get-Acl -LiteralPath $Root
    $want = @('S-1-5-18', 'S-1-5-32-544', $id.User.Value) + @($(if ($agentSid) { $agentSid }))
    $got = @($ra.Access | ForEach-Object { try { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { $_.IdentityReference.Value } })
    $rootOk = $ra.AreAccessRulesProtected -and $agentSid -and @($ra.Access | Where-Object { $_.AccessControlType -ne 'Allow' -or $_.IsInherited }).Count -eq 0 -and
        @($got | Sort-Object -Unique).Count -eq 4 -and @($want | Where-Object { $got -notcontains $_ }).Count -eq 0 -and
        @($ra.Access | Where-Object { $_.InheritanceFlags -ne 'ContainerInherit, ObjectInherit' -or $_.PropagationFlags -ne 'None' }).Count -eq 0 -and
        @($ra.Access | Where-Object {
            $s = try { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { '' }
            $r = [int64]$_.FileSystemRights -band 0x1F01FF
            if ($s -eq $agentSid) { $r -ne 0x1301BF } else { $r -ne 0x1F01FF }   # glim-agent: Modify exactly; the rest: Full
        }).Count -eq 0
}
if ($rootOk) { Write-Host "  already set" }
else { Act "create $Work and set its ACL" {
    New-Item -ItemType Directory -Path $Work -Force | Out-Null
    icacls $Root /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' "${Owner}:(OI)(CI)F" "${AgentName}:(OI)(CI)M" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls $Root failed ($LASTEXITCODE)" }
} }

# ── 5. Your profile ──────────────────────────────────────────────────────────
Step "5. Deny $AgentName on $OwnerProfile"
$hasDeny = $false
if ($agentSid) {
    $hasDeny = [bool]((Get-Acl $OwnerProfile).Access | Where-Object { $_.AccessControlType -eq 'Deny' -and $(try { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq $agentSid } catch { $false }) })
}
if ($hasDeny) { Write-Host '  already denied' }
else {
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
            $lockFailed.Add("$($t.Path) ($(Get-LockWhy $rc))")
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
if ($DryRun) { Write-Host '  (then: every item inside still writable by it gets its own deny, and all is walked again)' }
elseif ($agentSid) {
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
            $isDir = [bool](Test-Path -LiteralPath $p -PathType Container)
            $rc = [GlimFs]::AddDeny($p, $agentSid, [uint32]$need, $isDir)
            if ($rc -ne 0) { $lockFailed.Add("$p ($(Get-LockWhy $rc))") }
        }
        Write-Host '  verifying: walking every locked tree again...'
        $v = Get-GlimStillWritable -Sid $agentSid -Targets $targets
    }
    if ($lockFailed.Count) { $lockFailed | ForEach-Object { Write-Host "  NOT LOCKED: $_" -ForegroundColor Red } }
    if ($v.Unreadable.Count) { Write-Warning ("  $($v.Unreadable.Count) item(s) couldn't be read or listed (NOT verified), e.g. " + (($v.Unreadable | Select-Object -First 5) -join '; ')) }
    if ($v.Writable.Count) {
        $v.Writable | Select-Object -First 50 | ForEach-Object { Write-Host "  STILL WRITABLE: $_" -ForegroundColor Red }
        throw "$($v.Writable.Count) item(s) are still writable by $AgentName after locking. No sandbox session until they're fixed (tell Claude the list; setup is safe to run again)."
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
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
else { Write-Host 'Done. Next: scripts\agent-sandbox\check.ps1 (no UAC needed).' -ForegroundColor Green }
