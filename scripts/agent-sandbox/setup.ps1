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

# Users only: in no other local group (Administrators, Backup Operators,
# Remote Desktop Users, Hyper-V Administrators, Remote Management Users ...).
# By the user's own group list (ADSI), which doesn't trip over orphaned SIDs
# in a group the way Get-LocalGroupMember does.
if ($user -or -not $DryRun) {
    $u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
    if ($u) {
        $adsiUser = [ADSI]"WinNT://$env:COMPUTERNAME/$AgentName,user"
        foreach ($g in @($adsiUser.Groups())) {
            $gName = $g.GetType().InvokeMember('Name', 'GetProperty', $null, $g, $null)
            $gSid = (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value
            if ($gSid -ne 'S-1-5-32-545') {
                Act "remove $AgentName from $gName ($gSid)" { ([ADSI]"WinNT://$env:COMPUTERNAME/$gName,group").Remove($adsiUser.Path) }
            }
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
else {
    $tmp = Join-Path $env:TEMP ('glim-secedit-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        secedit /export /cfg "$tmp\cur.inf" /areas USER_RIGHTS /quiet | Out-Null
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
Act "create $Work and set its ACL" {
    New-Item -ItemType Directory -Path $Work -Force | Out-Null
    icacls $Root /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' "${Owner}:(OI)(CI)F" "${AgentName}:(OI)(CI)M" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls $Root failed ($LASTEXITCODE)" }
}

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
foreach ($t in $targets) {
    $flags = if ($t.Scope -eq 'tree') { '(OI)(CI)' } else { '' }   # folder or file: that object only
    $done = $false
    if ($agentSid) {
        $done = [bool]((Get-Acl -LiteralPath $t.Path).Access | Where-Object {
            $_.AccessControlType -eq 'Deny' -and -not $_.IsInherited -and
            $(try { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq $agentSid } catch { $false }) -and
            ((([int64]$_.FileSystemRights) -band 0xD0056) -eq 0xD0056) -and
            ($t.Scope -ne 'tree' -or (($_.InheritanceFlags -band 3) -eq 3))  # both ContainerInherit and ObjectInherit
        })
    }
    $label = "{0}  [{1}]  ({2})" -f $t.Path, $t.Scope, $t.Why
    if ($done) { Write-Host "  already locked: $label"; continue }
    Act "lock $label" {
        # /C: keep going past a file it can't change; the count is reported.
        $outText = icacls $t.Path /deny "${AgentName}:$flags$GlimDenyRights" /C 2>&1 | Out-String
        $code = $LASTEXITCODE
        $m = [regex]::Match($outText, 'Failed processing (\d+) files')
        $failed = if ($m.Success) { [int]$m.Groups[1].Value } else { -1 }
        if ($code -ne 0 -and $failed -le 0) { throw "icacls $($t.Path) failed ($code): $($outText.Trim())" }
        if ($failed -gt 0) { Write-Warning "  icacls couldn't change $failed item(s) under $($t.Path); the walk below finds any that stay writable" }
    }
}
if ($targets.Count -eq 0) { Write-Host '  nothing to lock' }
# Verified by result, not by icacls's messages: every target, trees walked in
# full, with glim-agent's real access computed on this side (AuthZ).
if ($agentSid -and -not $DryRun) {
    Write-Host '  verifying: walking every locked tree (about a minute)...'
    $v = Get-GlimStillWritable -Sid $agentSid -Targets $targets
    if ($v.Unreadable.Count) { Write-Warning ("  $($v.Unreadable.Count) item(s) whose permissions couldn't be read (not verified), e.g. " + (($v.Unreadable | Select-Object -First 5) -join '; ')) }
    if ($v.Writable.Count) {
        $v.Writable | Select-Object -First 50 | ForEach-Object { Write-Host "  STILL WRITABLE: $_" -ForegroundColor Red }
        throw "$($v.Writable.Count) item(s) are still writable by $AgentName after locking. No sandbox session until they're fixed (tell Claude the list)."
    }
    Write-Host '  verified: nothing in the locked folders is writable by it'
}

# glim-agent's SID, kept in YOUR registry (only you can write it), so
# remove.ps1 can clean up even after the user is gone.
if ($agentSid) {
    Act "remember $AgentName's SID in HKCU:\Software\Glim\AgentSandbox" {
        $k = 'HKCU:\Software\Glim\AgentSandbox'
        if (-not (Test-Path $k)) { New-Item -Path $k -Force | Out-Null }
        New-ItemProperty -Path $k -Name Sid -Value $agentSid -PropertyType String -Force | Out-Null
    }
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
