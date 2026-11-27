# Agent sandbox network check (docs/agent-sandbox-plan.md §2): proves, by
# trying, that glim-agent reaches GitHub and api.anthropic.com through the
# proxy and nothing else. Run by the owner (no elevation needed) after
# setup.ps1 step 8 made the firewall rules:
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\net-check.ps1
#
# The attempts run AS glim-agent (the firewall only acts on its connections),
# with System32's curl.exe and `powershell -NoProfile`. Where a result can be
# seen from YOUR side it is: a connection to a listener this script opens on
# 127.0.0.1 counts as reached even if the probe says otherwise, and every
# allowed tunnel must also appear in the proxy's own log. glim-agent has run
# nothing before this check, so nothing of its own can bend the probe yet;
# run it again before trusting sandbox sessions later.
#
# It FAILS unless:
#   - the four "Glim agent sandbox" firewall rules exist, are enabled, block,
#     and name glim-agent's SID (read back);
#   - direct connections (no proxy) to example.com, 1.1.1.1 and github.com
#     all fail;
#   - a connection to another port on 127.0.0.1 fails (your listener sees
#     nothing);
#   - through the proxy: example.com is refused; api.github.com,
#     api.anthropic.com and `git ls-remote` on GitHub work, and the proxy
#     logged each.
# DNS lookups are reported, not failed: Windows' DNS Client service makes
# them, not glim-agent, so the firewall can't stop them (plan §2: GitHub is
# reachable anyway, so a lookup carries out nothing GitHub couldn't).
# Nothing is changed (a proxy started for the check is stopped after).
param([int]$Port = 47643)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'cred.ps1')
$AgentName = 'glim-agent'
$Group = 'Glim agent sandbox'
$fail = 0; $checked = 0
function Report([bool]$ok, [string]$what) {
    $script:checked++
    if ($ok) { Write-Host "PASS  $what" -ForegroundColor Green } else { Write-Host "FAIL  $what" -ForegroundColor Red; $script:fail++ }
}

$u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
if (-not $u) { throw "$AgentName doesn't exist: run setup.ps1 first." }
$sid = $u.SID.Value

# ── The firewall rules (read back) ───────────────────────────────────────────
$rules = @(Get-NetFirewallRule -Group $Group -ErrorAction SilentlyContinue)
Report ($rules.Count -eq 4) "four '$Group' firewall rules (found $($rules.Count))"
foreach ($r in $rules) {
    $sec = $r | Get-NetFirewallSecurityFilter
    $ok = $r.Enabled -eq 'True' -and $r.Action -eq 'Block' -and "$($sec.LocalUser)" -match [regex]::Escape($sid)
    Report $ok ("rule '{0}': enabled, block, {1}, for {2}" -f $r.DisplayName, $r.Direction, $AgentName)
}
$prof = @(Get-NetFirewallProfile | Where-Object { -not $_.Enabled })
Report ($prof.Count -eq 0) ('Windows Firewall on for every profile' + $(if ($prof.Count) { ' (off: ' + (($prof | ForEach-Object Name) -join ', ') + ')' } else { '' }))

# ── The proxy ────────────────────────────────────────────────────────────────
$log = Join-Path $env:LOCALAPPDATA 'Glim\agent-proxy.log'
$started = $null
$probe = New-Object Net.Sockets.TcpClient
try { $probe.Connect('127.0.0.1', $Port); $running = $true } catch { $running = $false } finally { $probe.Close() }
if (-not $running) {
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $started = Start-Process -FilePath $ps -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $PSScriptRoot 'proxy.ps1')`"", '-Port', $Port -WindowStyle Hidden -PassThru
    for ($i = 0; $i -lt 40 -and -not $running; $i++) {
        Start-Sleep -Milliseconds 250
        $probe = New-Object Net.Sockets.TcpClient
        try { $probe.Connect('127.0.0.1', $Port); $running = $true } catch { } finally { $probe.Close() }
    }
}
if (-not $running) { throw "the proxy isn't listening on 127.0.0.1:$Port" }
$logStart = if (Test-Path -LiteralPath $log) { (Get-Item -LiteralPath $log).Length } else { 0 }

# Your listener: glim-agent must not reach it.
$listener = New-Object Net.Sockets.TcpListener ([Net.IPAddress]::Loopback, 0)
$listener.Start()
$lport = $listener.LocalEndpoint.Port

# ── The attempts, as glim-agent ──────────────────────────────────────────────
$curl = Join-Path $env:SystemRoot 'System32\curl.exe'
$git = (Get-Command git.exe -ErrorAction SilentlyContinue | Select-Object -First 1).Source
$px = "http://127.0.0.1:$Port"
$probeScript = @"
`$ErrorActionPreference = 'Continue'
function C([string]`$name, [string[]]`$a) {
    `$o = & '$curl' -s -o NUL --max-time 12 -w '%{http_code}:%{http_connect}' @a 2>`$null
    "`$name=`${LASTEXITCODE}:`$o"
}
C 'direct-example' @('--noproxy', '*', 'https://example.com/')
C 'direct-ip' @('--noproxy', '*', 'https://1.1.1.1/')
C 'direct-github' @('--noproxy', '*', 'https://api.github.com/zen')
C 'direct-loopback' @('--noproxy', '*', '--connect-timeout', '5', 'http://127.0.0.1:$lport/')
C 'proxy-example' @('-x', '$px', '--proxytunnel', 'https://example.com/')
C 'proxy-github' @('-x', '$px', '--proxytunnel', 'https://api.github.com/zen')
C 'proxy-anthropic' @('-x', '$px', '--proxytunnel', 'https://api.anthropic.com/')
if ('$git') {
    `$env:GIT_TERMINAL_PROMPT = '0'
    `$r = & '$git' -c http.proxy=$px ls-remote https://github.com/anthonygarcia866/local-companion HEAD 2>`$null
    "git=`${LASTEXITCODE}:`$(([string]`$r).Length -gt 40)"
} else { 'git=none' }
`$d = try { [Net.Dns]::GetHostAddresses('example.com').Count } catch { 0 }
"dns=`$d"
"@
$enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probeScript))
$plain = Read-GlimCredential 'Glim/AgentUserPassword'
if (-not $plain) { throw 'Glim/AgentUserPassword is not in your Credential Manager.' }
$cred = New-Object Management.Automation.PSCredential (".\$AgentName", (ConvertTo-SecureString $plain -AsPlainText -Force))
$plain = $null
$out = Join-Path $env:TEMP ('glim-netprobe-' + [guid]::NewGuid().ToString('N') + '.txt')
$res = @{}
try {
    Write-Host "  running the attempts as $AgentName (about a minute)..."
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    Start-Process -FilePath $ps -ArgumentList '-NoProfile', '-NonInteractive', '-EncodedCommand', $enc -Credential $cred -WorkingDirectory 'C:\GlimAgent\work' -WindowStyle Hidden -RedirectStandardOutput $out -Wait
    foreach ($line in @(Get-Content -LiteralPath $out -ErrorAction SilentlyContinue)) {
        $k, $v = $line -split '=', 2
        if ($v) { $res[$k] = $v }
    }
    $reached = $listener.Pending()
} finally {
    Remove-Item $out -Force -ErrorAction SilentlyContinue
    $listener.Stop()
    if ($started) { Stop-Process -Id $started.Id -Force -ErrorAction SilentlyContinue }
}
Report ($res.Count -ge 8) "the probe ran as $AgentName and answered ($($res.Count) results)"
function Code([string]$k) { if ($res.ContainsKey($k)) { $res[$k] } else { 'none' } }
# curl: exit 0 = it got an HTTP answer; a blocked connect is 7 (refused) or 28 (timeout).
foreach ($k in 'direct-example', 'direct-ip', 'direct-github', 'direct-loopback') {
    $c = Code $k
    Report ($c -ne 'none' -and -not $c.StartsWith('0:')) "$k blocked (curl $c)"
}
Report (-not $reached) "your listener on 127.0.0.1:$lport saw no connection from $AgentName"
$pe = Code 'proxy-example'
Report ($pe -ne 'none' -and $pe.EndsWith(':403')) "proxy refuses example.com (curl $pe)"
Report ((Code 'proxy-github') -like '0:200:200') "api.github.com through the proxy ($(Code 'proxy-github'))"
Report ((Code 'proxy-anthropic') -like '0:*:200') "api.anthropic.com through the proxy ($(Code 'proxy-anthropic'))"
Report ((Code 'git') -eq '0:True') "git ls-remote on GitHub through the proxy ($(Code 'git'))"
$newLog = if (Test-Path -LiteralPath $log) {
    $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite'); try { $fs.Seek($logStart, 'Begin') | Out-Null; (New-Object IO.StreamReader $fs).ReadToEnd() } finally { $fs.Close() }
} else { '' }
foreach ($h in 'api.github.com', 'api.anthropic.com', 'github.com') {
    Report ($newLog -match ("allowed " + [regex]::Escape("${h}:443"))) "the proxy's own log shows the tunnel to $h"
}
Write-Host ("info  DNS lookup of example.com as {0}: {1} (Windows' DNS Client makes it, so the firewall doesn't apply; plan §2)" -f $AgentName, $(if ((Code 'dns') -ne '0' -and (Code 'dns') -ne 'none') { 'answered' } else { 'no answer' })) -ForegroundColor DarkGray

Write-Host ''
if ($checked -lt 15) { Write-Host "Only $checked checks ran: not trusted." -ForegroundColor Red; exit 1 }
if ($fail -gt 0) { Write-Host "$fail of $checked check(s) failed: the network limit does NOT hold; sandbox sessions stay off." -ForegroundColor Red; exit 1 }
Write-Host "All $checked checks passed." -ForegroundColor Green
exit 0
