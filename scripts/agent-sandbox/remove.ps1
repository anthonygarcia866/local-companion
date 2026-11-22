# Undoes setup.ps1. Run by the owner, ELEVATED (one UAC prompt):
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1 -DryRun
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\remove.ps1
# Removes: the deny ACE on your profile, the logon-right denials, the
# sign-in hiding value, the glim-agent user, its profile folder, C:\GlimAgent
# and Glim/AgentUserPassword from your Credential Manager.

[CmdletBinding()]
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'cred.ps1')
$AgentName = 'glim-agent'
function Act([string]$what, [scriptblock]$do) {
    if ($DryRun) { Write-Host ("would: " + $what) -ForegroundColor Yellow } else { Write-Host ("doing: " + $what); & $do }
}
$u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if ($u) {
    $sid = $u.SID.Value
    Act "remove the deny ACE for $AgentName on $env:USERPROFILE" { icacls $env:USERPROFILE /remove:d $AgentName | Out-Null }
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
            }
        } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    }
    $prof = (Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $sid })
    if ($prof) { Act "remove its profile $($prof.LocalPath)" { $prof | Remove-CimInstance } }
    Act "remove user $AgentName" { Remove-LocalUser -Name $AgentName }
}
Act 'remove the sign-in hiding value' { Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList' -Name $AgentName -ErrorAction SilentlyContinue }
if (Test-Path 'C:\GlimAgent') { Act 'remove C:\GlimAgent' { Remove-Item -Recurse -Force 'C:\GlimAgent' } }
if (Test-GlimCredential 'Glim/AgentUserPassword') { Act 'remove Glim/AgentUserPassword from your Credential Manager' { Remove-GlimCredential 'Glim/AgentUserPassword' } }
if ($DryRun) { Write-Host 'Dry run: nothing was changed.' -ForegroundColor Yellow }
