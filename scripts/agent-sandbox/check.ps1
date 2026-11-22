# Agent sandbox check (docs/agent-sandbox-plan.md §2): proves what glim-agent
# can't reach, by trying it AS glim-agent. Run by the owner, NOT elevated:
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\check.ps1
#
# It reads glim-agent's password from YOUR Credential Manager
# (Glim/AgentUserPassword, written by setup.ps1), starts probe.ps1 as
# glim-agent (Windows' secondary logon, its own profile and environment), and
# reports PASS/FAIL per check. Nothing is changed. The probe never sits on
# disk where glim-agent can write (it goes in as -EncodedCommand) and its
# answer comes back on stdout into YOUR temp folder, which glim-agent can't
# open; any glim-agent process still running is stopped first so nothing of
# its can race the check.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'cred.ps1')

$AgentName = 'glim-agent'
$plain = Read-GlimCredential 'Glim/AgentUserPassword'
if (-not $plain) { throw 'Glim/AgentUserPassword is not in your Credential Manager: run setup.ps1 first.' }
$cred = New-Object Management.Automation.PSCredential (".\$AgentName", (ConvertTo-SecureString $plain -AsPlainText -Force))
$plain = $null

# What must fail (owner's logins and vault-adjacent folders; Google Drive if mounted).
$mustFail = @(
    $env:USERPROFILE,
    (Join-Path $env:USERPROFILE '.config\gh'),
    (Join-Path $env:APPDATA 'xdg.config\.wrangler'),
    (Join-Path $env:APPDATA 'GitHub CLI'),
    (Join-Path $env:USERPROFILE '.claude'),
    (Join-Path $env:USERPROFILE '.claude2'),
    (Join-Path $env:LOCALAPPDATA 'Glim'),
    (Join-Path $env:APPDATA 'Glim')
)
if (Test-Path 'G:\') { $mustFail += 'G:\' }
$envMustBeAbsent = 'CLOUDFLARE_API_TOKEN', 'SUPABASE_ACCESS_TOKEN', 'ANTHROPIC_API_KEY', 'GH_TOKEN', 'GITHUB_TOKEN'

# Nothing of glim-agent's may be running while it is checked.
Get-CimInstance Win32_Process | Where-Object { try { ($_ | Invoke-CimMethod -MethodName GetOwner).User -eq $AgentName } catch { $false } } | ForEach-Object {
    Write-Host "Stopping glim-agent process $($_.Name) ($($_.ProcessId))"
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}

$q = { param($s) "'" + ($s -replace "'", "''") + "'" }
$probe = Get-Content -Raw (Join-Path $PSScriptRoot 'probe.ps1')
$cmd = '& {' + "`n" + $probe + "`n" + '} -MustFail ' + (& $q ($mustFail -join '|')) + ' -EnvMustBeAbsent ' + (& $q ($envMustBeAbsent -join '|'))
$enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cmd))
$out = Join-Path $env:TEMP ('glim-probe-' + [guid]::NewGuid().ToString('N') + '.txt')
$err = "$out.err"
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
try {
    $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-NonInteractive', '-EncodedCommand', $enc) -Credential $cred -LoadUserProfile -WorkingDirectory 'C:\GlimAgent\work' -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err -Wait -PassThru
    $text = if (Test-Path $out) { Get-Content $out -Raw } else { '' }
} finally { Remove-Item $out, $err -Force -ErrorAction SilentlyContinue }
$m = [regex]::Match([string]$text, 'GLIM-PROBE-BEGIN\s*(\{.*\})\s*GLIM-PROBE-END', 'Singleline')
if (-not $m.Success) { throw "The probe gave no result (exit $($p.ExitCode))." }
$r = $m.Groups[1].Value | ConvertFrom-Json
if (@($r.paths.PSObject.Properties).Count -ne $mustFail.Count) { throw 'The probe checked a different number of paths than asked: not trusted.' }

$fail = 0
function Report([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "PASS  $what" -ForegroundColor Green } else { Write-Host "FAIL  $what" -ForegroundColor Red; $script:fail++ }
}
Report ($r.whoami -match "\\$AgentName$") "runs as $AgentName ($($r.whoami))"
foreach ($prop in $r.paths.PSObject.Properties) { Report ($prop.Value -ne 'readable') "can't read $($prop.Name) ($($prop.Value))" }
foreach ($prop in $r.env.PSObject.Properties) { Report ($prop.Value -eq 'absent') "no $($prop.Name) in its environment" }
Report (@($r.glimCredentials).Count -eq 0) "its Credential Manager has no Glim/ entries"
Report ($r.workWritable -eq 'yes') "can write C:\GlimAgent\work"
Write-Host ''
Write-Host 'Drive roots it can list (info: C:\ is normal; review any other):'
foreach ($prop in $r.drives.PSObject.Properties) { Write-Host "  $($prop.Name)  $($prop.Value)" }
Write-Host ''
if ($fail -eq 0) { Write-Host 'All checks passed.' -ForegroundColor Green; exit 0 }
Write-Host "$fail check(s) failed: sandbox sessions must stay off." -ForegroundColor Red
exit 1
