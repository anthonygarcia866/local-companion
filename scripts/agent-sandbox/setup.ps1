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
#   6. Lists folders on fixed drives (two levels down) where Users,
#      Authenticated Users or Everyone can write, for the owner to review.
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
            if ($held -notcontains "*$agentSid") {
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
    $hasDeny = [bool]((Get-Acl $OwnerProfile).Access | Where-Object { $_.AccessControlType -eq 'Deny' -and $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq $agentSid })
}
if ($hasDeny) { Write-Host '  already denied' }
else {
    Act "icacls $OwnerProfile /deny ${AgentName}:(OI)(CI)F" {
        # icacls puts the deny on the folder and propagates it to every child
        # that inherits (it can take a while on a big profile). A file or
        # folder with inheritance turned off doesn't get it: check.ps1 tries
        # to open the actual login files to catch that.
        icacls $OwnerProfile /deny "${AgentName}:(OI)(CI)F" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls $OwnerProfile failed ($LASTEXITCODE)" }
    }
}

# ── 6. Writable by everyone: for you to review ──────────────────────────────
Step '6. Folders (2 levels) on fixed drives that Users / Authenticated Users / Everyone can write'
$broad = 'S-1-5-32-545', 'S-1-5-11', 'S-1-1-0'
# Write-only bits (Modify/FullControl include read bits, so they can't be the
# mask): create files, create folders, delete, change permissions, take
# ownership, plus GENERIC_WRITE / GENERIC_ALL as stored in inheritable ACEs.
$writeBits = [int64](2 -bor 4 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000)
$drives = Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object { $_.DeviceID + '\' }
$found = foreach ($d in $drives) {
    $dirs = @(Get-Item -LiteralPath $d) + @(Get-ChildItem -LiteralPath $d -Directory -Force -ErrorAction SilentlyContinue)
    $dirs += $dirs | Where-Object { $_.FullName -ne $d -and $_.FullName -notlike "$OwnerProfile*" } | ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Directory -Force -ErrorAction SilentlyContinue }
    foreach ($dir in $dirs) {
        try { $acl = Get-Acl -LiteralPath $dir.FullName } catch { continue }
        foreach ($a in $acl.Access) {
            if ($a.AccessControlType -ne 'Allow') { continue }
            try { $sid = $a.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { continue }
            if ($broad -contains $sid -and (([int64]$a.FileSystemRights) -band $writeBits)) { $dir.FullName; break }
        }
    }
}
$found = $found | Sort-Object -Unique
if ($found) { $found | ForEach-Object { Write-Host "  $_" } } else { Write-Host '  none' }
Write-Host ''
Write-Host 'Normal ones: C:\Users\Public, C:\ProgramData (and some of its children), C:\Windows\Temp-like folders. Tell Claude about any others you did not expect.'
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
else { Write-Host 'Done. Next: scripts\agent-sandbox\check.ps1 (no UAC needed).' -ForegroundColor Green }
