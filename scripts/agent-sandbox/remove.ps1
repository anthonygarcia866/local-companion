# Undoes setup.ps1. Run by the owner, ELEVATED (one UAC prompt):
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1 -DryRun
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1
# Removes: the deny ACE on your profile, the logon-right denials, the
# sign-in hiding value, the glim-agent user, its profile folder, C:\GlimAgent
# and Glim/AgentUserPassword from your Credential Manager.

[CmdletBinding()]
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'cred.ps1')
$AgentName = 'glim-agent'
function Act([string]$what, [scriptblock]$do) {
    if ($DryRun) { Write-Host ("would: " + $what) -ForegroundColor Yellow } else { Write-Host ("doing: " + $what); & $do }
}
$u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if ($u) {
    Act "stop every $AgentName process" {
        Get-CimInstance Win32_Process | Where-Object { try { ($_ | Invoke-CimMethod -MethodName GetOwner).User -eq $AgentName } catch { $false } } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    $sid = $u.SID.Value
    Act "remove the deny ACE for $AgentName on $env:USERPROFILE" { icacls $env:USERPROFILE /remove:d $AgentName | Out-Null; if ($LASTEXITCODE -ne 0) { throw "icacls failed ($LASTEXITCODE)" } }
    Act 'remove its logon-right denials' {
        $tmp = Join-Path $env:TEMP ('glim-secedit-' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tmp | Out-Null
        try {
            secedit /export /cfg "$tmp\cur.inf" /areas USER_RIGHTS /quiet | Out-Null
            $set = foreach ($right in 'SeDenyRemoteInteractiveLogonRight', 'SeDenyNetworkLogonRight') {
                $line = Get-Content "$tmp\cur.inf" | Where-Object { $_ -match "^$right\s*=" }
                if ($line) {
                    $keep = @((($line -split '=', 2)[1].Trim() -split ',') | Where-Object { $_.Trim() -and $_.Trim() -ne "*$sid" })
                    "$right = " + ($keep -join ',')
                }
            }
            if ($set) {
                @('[Unicode]', 'Unicode=yes', '[Version]', 'signature="$CHICAGO$"', 'Revision=1', '[Privilege Rights]') + $set | Set-Content "$tmp\new.inf" -Encoding Unicode
                secedit /configure /db "$tmp\new.sdb" /cfg "$tmp\new.inf" /areas USER_RIGHTS /quiet | Out-Null
                if ($LASTEXITCODE -ne 0) { throw "secedit failed ($LASTEXITCODE)" }
            }
        } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    }
    $prof = (Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $sid })
    if ($prof) { Act "remove its profile $($prof.LocalPath)" { $prof | Remove-CimInstance } }
    Act "remove user $AgentName" { Remove-LocalUser -Name $AgentName }
}
Act 'remove the sign-in hiding value' { Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList' -Name $AgentName -ErrorAction SilentlyContinue }
# After the user is gone (nothing of it runs). rd /s /q removes a junction or
# symlink glim-agent may have planted as the link itself, never following it
# (Remove-Item -Recurse in Windows PowerShell 5.1 can follow one).
if (Test-Path 'C:\GlimAgent') { Act 'remove C:\GlimAgent (links removed, never followed)' { cmd /c rd /s /q 'C:\GlimAgent'; if (Test-Path 'C:\GlimAgent') { throw 'C:\GlimAgent is still there' } } }
if (Test-GlimCredential 'Glim/AgentUserPassword') { Act 'remove Glim/AgentUserPassword from your Credential Manager' { Remove-GlimCredential 'Glim/AgentUserPassword' } }
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
