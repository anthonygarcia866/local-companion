# Undoes setup.ps1. Run by the owner, ELEVATED (one UAC prompt):
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1 -DryRun
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1
#
# Order (review 2026-11-22): first the account goes (disabled, its processes
# stopped, deleted), then its folders are deleted without following any link
# it may have planted (rd /s /q), and only then are the protections lifted
# (the deny entries on your profile and the locked folders, the logon-right
# denials), so a failure part way never leaves a live glim-agent unfenced.
# Its SID comes from the account, else from HKCU:\Software\Glim\AgentSandbox
# (written by setup.ps1), else -Sid, so a half-removed setup still cleans up.

[CmdletBinding()]
param([switch]$DryRun, [string]$Sid = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'cred.ps1')
. (Join-Path $PSScriptRoot 'targets.ps1')
. (Join-Path $PSScriptRoot 'authz.ps1')
$AgentName = 'glim-agent'
$RegKey = 'HKCU:\Software\Glim\AgentSandbox'
function Act([string]$what, [scriptblock]$do) {
    if ($DryRun) { Write-Host ("would: " + $what) -ForegroundColor Yellow } else { Write-Host ("doing: " + $what); & $do }
}

$u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if ($u) { $Sid = $u.SID.Value }
elseif (-not $Sid) {
    $saved = Get-ItemProperty -Path $RegKey -Name Sid -ErrorAction SilentlyContinue
    if ($saved) { $Sid = $saved.Sid }
}
if ($Sid -and $Sid -notmatch '^S-1-5-21-\d+-\d+-\d+-\d+$') { throw "Not a local account SID: $Sid" }
# A SID given by hand (or remembered) must not belong to anyone else still here.
if ($Sid -and -not $u) {
    $owner = Get-LocalUser | Where-Object { $_.SID.Value -eq $Sid }
    if ($owner) { throw "SID $Sid belongs to the existing account '$($owner.Name)', not glim-agent: refusing." }
    if ($Sid -eq ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value)) { throw 'That is your own SID: refusing.' }
}
Write-Host ("glim-agent: " + $(if ($u) { "exists ($Sid)" } elseif ($Sid) { "gone; cleaning up after SID $Sid" } else { 'gone and no SID known: only the non-SID parts are cleaned' }))

# ── 1. The account ──────────────────────────────────────────────────────────
$profilePath = $null
if ($Sid) {
    $prof = Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $Sid }
    if ($prof) { $profilePath = $prof.LocalPath }
}
if ($u) {
    Act "disable $AgentName" { Disable-LocalUser -Name $AgentName }
    Act "stop every $AgentName process" {
        Get-CimInstance Win32_Process | Where-Object { try { ($_ | Invoke-CimMethod -MethodName GetOwner).User -eq $AgentName } catch { $false } } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    Act "delete user $AgentName" { Remove-LocalUser -Name $AgentName }
}

# ── 2. Its folders, links removed and never followed ───────────────────────
# Only a profile folder named after glim-agent (glim-agent, glim-agent.PC, ...).
if ($profilePath -and $profilePath -match '^C:\\Users\\glim-agent(\.[^\\]+)?$' -and $profilePath -ine $env:USERPROFILE) {
    if (Test-Path -LiteralPath $profilePath) {
        Act "delete its profile folder $profilePath (rd /s /q)" {
            cmd /c rd /s /q "$profilePath"
            if (Test-Path -LiteralPath $profilePath) { Write-Warning "  $profilePath is still there (a file in use?): run remove.ps1 again after a restart" }
        }
    }
    Act 'remove its profile entry' { Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $Sid } | ForEach-Object { try { $_ | Remove-CimInstance } catch { Write-Warning "  profile entry: $($_.Exception.Message)" } } }
}
if (Test-Path 'C:\GlimAgent') {
    Act 'delete C:\GlimAgent (rd /s /q)' { cmd /c rd /s /q 'C:\GlimAgent'; if (Test-Path 'C:\GlimAgent') { throw 'C:\GlimAgent is still there' } }
}

# ── 3. Lift the protections ─────────────────────────────────────────────────
if ($Sid) {
    function Test-ExplicitDeny([string]$p) {
        foreach ($ace in (Get-Acl -LiteralPath $p).Access) {
            if ($ace.AccessControlType -ne 'Deny' -or $ace.IsInherited) { continue }
            $v = try { $ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { $ace.IdentityReference.Value }
            if ($v -eq $Sid) { return $true }
        }
        $false
    }
    # Your profile only (review M-b): setup's step 5 put this deny with icacls,
    # so icacls takes it and its copies off; only you can write in there. The
    # named folders (Public, Windows\Tasks) are lock targets, cleared below.
    # Only while nothing in it lets all users write (review 3 L-5: icacls
    # re-stamps it all); else the deny stays: once the account is deleted it
    # names nobody.
    $bwp = @(Get-GlimExplicitWritable $env:USERPROFILE)
    if ($bwp.Count) { Write-Warning "  $($bwp.Count) item(s) in your profile let all users write (e.g. $($bwp[0].Path)): its deny for the deleted account is left in place (harmless)" }
    foreach ($p in @($(if (-not $bwp.Count) { $env:USERPROFILE }))) {
        if (Test-ExplicitDeny $p) {
            Act "remove its deny entry on $p" {
                $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
                try { icacls $p /remove:d "*$Sid" /C 2>&1 | Out-Null } finally { $ErrorActionPreference = $eap }
                if (Test-ExplicitDeny $p) { Write-Warning "  couldn't remove it from $p" }
            }
        }
    }
    # The locked folders: setup's own writer (review 2026-11-26) changes one
    # object's DACL at a time, nothing re-stamped or passed down, so the root
    # and every item inside are cleared one by one; failures are counted and
    # shown (review L6). A deny an older setup run left as an INHERITED copy
    # (icacls passed it down) names a deleted account once step 1 has run, so
    # it denies nobody; it goes when Windows next re-applies inheritance there.
    foreach ($t in @(Get-GlimLockTargets -OwnerProfile $env:USERPROFILE)) {
        $p = $t.Path
        $items = @($p)
        if ($t.Scope -eq 'tree' -and (Test-Path -LiteralPath $p -PathType Container)) { $items += @(Get-GlimTreeItems $p | ForEach-Object { $_.FullName }) }
        $with = @($items | Where-Object { try { Test-ExplicitDeny $_ } catch { $true } })
        if ($with.Count) {
            Act "remove its deny entries on $p ($($with.Count) item(s))" {
                $bad = 0
                foreach ($i in $with) { $rc = [GlimFs]::RemoveDeny($i, $Sid); if ($rc -ne 0) { $bad++; if ($bad -le 5) { Write-Warning "  couldn't remove it from $i (error $rc)" } } }
                if ($bad) { Write-Warning "  $bad item(s) under $p still carry its deny (harmless once the account is gone)" }
            }
        }
    }
    if ($GlimUnlistable.Count) { Write-Warning "  $($GlimUnlistable.Count) folder(s) couldn't be listed, so any deny inside them stays (harmless once the account is gone)" }
    if (@(Get-NetFirewallRule -Group 'Glim agent sandbox' -ErrorAction SilentlyContinue).Count) {
        Act "remove the 'Glim agent sandbox' firewall rules" { Remove-NetFirewallRule -Group 'Glim agent sandbox' }
    }
    Act 'remove its logon-right denials' {
        $tmp = Join-Path $env:TEMP ('glim-secedit-' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tmp | Out-Null
        try {
            secedit /export /cfg "$tmp\cur.inf" /areas USER_RIGHTS /quiet | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "secedit export failed ($LASTEXITCODE)" }
            $set = foreach ($right in 'SeDenyRemoteInteractiveLogonRight', 'SeDenyNetworkLogonRight') {
                $line = Get-Content "$tmp\cur.inf" | Where-Object { $_ -match "^$right\s*=" }
                if ($line) {
                    $all = @((($line -split '=', 2)[1].Trim() -split ',') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
                    $keep = @($all | Where-Object { $_ -ne "*$Sid" -and $_ -ne $AgentName -and $_ -ne "$env:COMPUTERNAME\$AgentName" })
                    if ($keep.Count -ne $all.Count) { "$right = " + ($keep -join ',') }
                }
            }
            if ($set) {
                @('[Unicode]', 'Unicode=yes', '[Version]', 'signature="$CHICAGO$"', 'Revision=1', '[Privilege Rights]') + $set | Set-Content "$tmp\new.inf" -Encoding Unicode
                secedit /configure /db "$tmp\new.sdb" /cfg "$tmp\new.inf" /areas USER_RIGHTS /quiet | Out-Null
                if ($LASTEXITCODE -ne 0) { throw "secedit failed ($LASTEXITCODE)" }
            }
        } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    }
}
Act 'remove the sign-in hiding value' { Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList' -Name $AgentName -ErrorAction SilentlyContinue }
if (Test-GlimCredential 'Glim/AgentUserPassword') { Act 'remove Glim/AgentUserPassword from your Credential Manager' { Remove-GlimCredential 'Glim/AgentUserPassword' } }
if (Test-Path $RegKey) { Act "remove $RegKey" { Remove-Item -Path $RegKey -Recurse -Force } }
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
