# Agent sandbox quick check (owner 2026-11-26): the short version of
# check.ps1 that Glim runs, NOT elevated, as you, before it starts any
# glim-agent session, when a lease is granted, and once a day. Apps recreate
# folders (C:\OneDriveTemp lost its lock that way), so a lock that held at
# setup time is proven again each time, never assumed.
#   powershell -NoProfile -ExecutionPolicy Bypass -File quick-check.ps1 [-Repair] [-Json]
#
# It FAILS unless:
#   - glim-agent exists, is the account setup recorded (HKCU SID), and is in
#     no local group but Users;
#   - your profile carries setup's deny of everything for it;
#   - C:\GlimAgent (holding only work), work and work\.glim-keep are exactly
#     as setup.ps1 step 4 makes them (real items, owner, protected, exact
#     entries); glim-agent is enabled (setup disables it while it runs);
#   - every lock target (the list setup saved, plus a fresh look at drive
#     roots, top-level folders, PATH folders and the named folders, where
#     a recreated folder turns up) is denied to it, and it can't write the
#     target or anything directly inside a locked tree;
#   - the four "Glim agent sandbox" firewall rules exist, are enabled, block
#     and name it (plan §2; setup.ps1 step 8).
# Deep trees are walked in full only by setup.ps1 and check.ps1.
#
# -Repair: a target found writable again gets its deny back, but only where
# that is safe from here, without elevation: a real folder or file (never a
# link; AddDeny itself refuses a hard-linked file or a null DACL) that you may
# change the permissions of (typically one an app just recreated under your
# account), through the same non-propagating writer as setup; a tree is then
# walked once (at most 20,000 items) and fixed the same way. Anything else is
# a failure: re-run setup.ps1 elevated. Every repair is listed.
#
# Exit 0 = passed, 1 = failed. -Json prints one line:
#   {"ok":bool,"installed":bool,"failures":[...],"repaired":[...],"unverified":[...],"ms":n}
# (unverified: items directly inside a locked tree that won't show you their
# permissions without elevation; setup.ps1 / check.ps1 verify them elevated)
# with paths and reasons only; nothing secret is read or printed.
param([switch]$Repair, [switch]$Json)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
$sw = [Diagnostics.Stopwatch]::StartNew()
$AgentName = 'glim-agent'
$failures = New-Object System.Collections.Generic.List[string]
$repaired = New-Object System.Collections.Generic.List[string]
$unverified = New-Object System.Collections.Generic.List[string]
# Set up at all? Neither the account nor setup's record: nothing to check (Glim's daily check then stays quiet).
$installed = [bool](Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue) -or [bool](Get-ItemProperty -Path 'HKCU:\Software\Glim\AgentSandbox' -Name Sid -ErrorAction SilentlyContinue)
function Fail([string]$x) { if (-not $failures.Contains($x)) { $failures.Add($x) } }

function Finish {
    $ok = $failures.Count -eq 0
    if ($Json) {
        [pscustomobject]@{ ok = $ok; installed = $installed; failures = @($failures); repaired = @($repaired); unverified = @($unverified); ms = [int]$sw.ElapsedMilliseconds } | ConvertTo-Json -Compress -Depth 3
    } else {
        foreach ($r in $repaired) { Write-Host "REPAIRED  $r" -ForegroundColor Yellow }
        foreach ($v in $unverified) { Write-Host "info  not verified here (setup.ps1 checks it elevated): $v" -ForegroundColor DarkGray }
        foreach ($f in $failures) { Write-Host "FAIL  $f" -ForegroundColor Red }
        if ($ok) { Write-Host "Quick check passed ($([int]$sw.ElapsedMilliseconds) ms)." -ForegroundColor Green }
    }
    exit $(if ($ok) { 0 } else { 1 })
}

try {
    . (Join-Path $PSScriptRoot 'targets.ps1')
    . (Join-Path $PSScriptRoot 'authz.ps1')

    # ── The account ─────────────────────────────────────────────────────────
    $u = Get-LocalUser -Name $AgentName -ErrorAction SilentlyContinue
    if (-not $u) { Fail "$AgentName doesn't exist (run setup.ps1)"; Finish }
    $sid = $u.SID.Value
    if (-not $u.Enabled) { Fail "$AgentName is disabled: a setup.ps1 run stopped part way (run it again)" }
    $saved = Get-ItemProperty -Path 'HKCU:\Software\Glim\AgentSandbox' -ErrorAction SilentlyContinue
    if (-not $saved -or -not ($saved.PSObject.Properties.Name -contains 'Sid') -or $saved.Sid -ne $sid) { Fail "$AgentName isn't the account setup.ps1 recorded (run setup.ps1)" }
    $adsi = [ADSI]"WinNT://$env:COMPUTERNAME/$AgentName,user"
    $groups = @(foreach ($g in @($adsi.Groups())) { (New-Object Security.Principal.SecurityIdentifier ($g.GetType().InvokeMember('objectSid', 'GetProperty', $null, $g, $null)), 0).Value })
    if ($groups -notcontains 'S-1-5-32-545') { Fail "$AgentName isn't in Users" }
    foreach ($g in $groups) { if ($g -ne 'S-1-5-32-545') { Fail "$AgentName is in another local group ($g)" } }

    # ── Your profile, its workspace ─────────────────────────────────────────
    if (-not [GlimFs]::HasDeny($env:USERPROFILE, $sid, [uint32]0x1F01FF, $true)) { Fail "your profile ($env:USERPROFILE) lost its deny for $AgentName" }
    $me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $own = [string[]]@('S-1-5-18', 'S-1-5-32-544', $me)
    $w1 = [GlimFs]::SameDir('C:\GlimAgent', "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;$me)(A;;0x1200a9;;;$sid)", $own)
    if ($w1) { Fail "C:\GlimAgent isn't as setup makes it ($w1)" }
    $w2 = [GlimFs]::SameDir('C:\GlimAgent\work', "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;$me)(A;;0x1200af;;;$sid)(A;OICIIO;0x1301bf;;;$sid)", $own)
    if ($w2) { Fail "C:\GlimAgent\work isn't as setup makes it ($w2)" }
    $w3 = [GlimFs]::SameFile('C:\GlimAgent\work\.glim-keep', "D:PAI(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;$me)(A;;0x120089;;;$sid)", $own)
    if ($w3) { Fail "C:\GlimAgent\work\.glim-keep isn't as setup makes it ($w3)" }
    $extra = @(Get-ChildItem -LiteralPath 'C:\GlimAgent' -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'work' })
    if ($extra.Count) { Fail ("C:\GlimAgent holds more than work: " + (($extra | ForEach-Object Name) -join ', ')) }

    # ── The locked folders ──────────────────────────────────────────────────
    $targets = [ordered]@{}
    if ($saved -and ($saved.PSObject.Properties.Name -contains 'Targets')) {
        foreach ($line in @($saved.Targets)) {
            $scope, $deny, $path = $line -split '\|', 3
            if ($path) { $targets[$path.ToLowerInvariant()] = [pscustomobject]@{ Path = $path; Scope = $scope; Deny = $deny } }
        }
    } else { Fail 'setup.ps1 has no saved lock list yet (run setup.ps1)' }
    foreach ($t in @(Get-GlimLockTargets -OwnerProfile $env:USERPROFILE -Quick)) {
        $k = $t.Path.ToLowerInvariant()
        if (-not $targets.Contains($k)) { $targets[$k] = [pscustomobject]@{ Path = $t.Path; Scope = $t.Scope; Deny = $t.Deny } }
    }
    # Items setup.ps1 verified elevated and found owned by SYSTEM /
    # TrustedInstaller / Administrators: the only ones that may stay "not
    # verified here" when Windows won't show you their permissions.
    $sysOwned = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    if ($saved -and ($saved.PSObject.Properties.Name -contains 'SystemOwned')) { foreach ($x in @($saved.SystemOwned)) { if ($x) { [void]$sysOwned.Add($x) } } }
    function Skip-Unreadable([string]$p, [string]$why) {
        if ($why -notlike "can't read its permissions (error 5)") { return $false }
        if ($sysOwned.Contains($p)) { $unverified.Add($p); return $true }
        Fail "$p`: Windows won't show you its permissions and setup.ps1 didn't verify it as a system folder (run setup.ps1 elevated)"
        return $true
    }
    foreach ($l in $GlimLinkFindings) { Fail "a link all users could change: $l (fix by hand; tell Claude)" }

    $checker = New-Object GlimAuthz+Checker $sid
    try {
        function Test-Item([string]$p, $t) {
            # Returns the reason it fails, or '' when it holds.
            $mask = if ($t.Deny -eq 'all') { $GlimRead -bor $GlimWrite } else { $GlimWrite }
            $e = 0; $sd = [GlimFs]::ReadSd($p, [ref]$e)
            if ($null -eq $sd) { if ($e -in 2, 3) { return '' } else { return "can't read its permissions (error $e)" } }
            $g = $checker.Granted($sd) -band $mask
            if ($g) { return ('{0} can write it (0x{1:X})' -f $AgentName, $g) }
            return ''
        }
        function Repair-Item([string]$p, $t, [bool]$tree) {
            # The deny back, only where safe from here. '' = repaired.
            if ([GlimFs]::IsLink($p)) { return 'it is a link' }
            $need = if ($t.Deny -eq 'all') { 0x1F01FF } else { 0xD0156 }
            $rc = [GlimFs]::AddDeny($p, $sid, [uint32]$need, $tree)
            switch ($rc) { 0 { return '' } -2 { return 'a hard-linked file' } -3 { return 'a null DACL' } 5 { return 'you may not change its permissions without elevation' } default { return "error $rc" } }
        }
        foreach ($t in $targets.Values) {
            if (-not (Test-Path -LiteralPath $t.Path)) { continue }   # gone: nothing to write in (a recreated one is found by discovery)
            $tree = $t.Scope -eq 'tree'
            $need = if ($t.Deny -eq 'all') { 0x1F01FF } else { 0xD0156 }
            $why = Test-Item $t.Path $t
            # A system folder that won't show you its permissions unelevated
            # (C:\Windows\Temp ...): only one setup verified (security review).
            if (Skip-Unreadable $t.Path $why) { continue }
            if (-not $why -and -not [GlimFs]::HasDeny($t.Path, $sid, [uint32]$need, $tree)) { $why = 'its deny is gone' }
            if ($why) {
                $fix = if ($Repair) { Repair-Item $t.Path $t $tree } else { 'not repaired (no -Repair)' }
                if ($fix -eq '') {
                    $repaired.Add("$($t.Path): $why; deny put back")
                    if ($tree) {
                        # The tree once, fixed the same way (bounded).
                        $r = [GlimFs]::Walk($t.Path, 'nodeny', [string[]]@($sid), [uint32]$need, $null)
                        if ($r.Items -gt 20000) { Fail "$($t.Path): too big to repair here ($($r.Items) items): run setup.ps1 elevated"; continue }
                        foreach ($h in $r.Hits) { $f2 = Repair-Item $h $t $true; if ($f2) { Fail "$h`: no inheritable deny, $f2 (run setup.ps1 elevated)" } }
                        $mask = if ($t.Deny -eq 'all') { $GlimRead -bor $GlimWrite } else { $GlimWrite }
                        $r = [GlimFs]::Walk($t.Path, 'access', $null, [uint32]$mask, $checker)
                        for ($i = 0; $i -lt $r.Hits.Count; $i++) { $f2 = Repair-Item $r.Hits[$i] $t $r.HitIsDir[$i]; if ($f2) { Fail "$($r.Hits[$i]): writable by $AgentName, $f2 (run setup.ps1 elevated)" } }
                    }
                    $why = Test-Item $t.Path $t
                    if ($why) { Fail "$($t.Path): $why after the repair" }
                } else { Fail "$($t.Path): $why; $fix" }
                continue
            }
            if ($tree) {
                # One level down: an item that took its own permissions (an app
                # recreating a subfolder) is the common way back to writable.
                foreach ($c in @(Get-ChildItem -LiteralPath $t.Path -Force -ErrorAction SilentlyContinue)) {
                    $cw = Test-Item $c.FullName $t
                    # The same for an item directly inside (C:\ProgramData\Packages ...).
                    if (Skip-Unreadable $c.FullName $cw) { continue }
                    if (-not $cw) { continue }
                    $isDir = $c.PSIsContainer -and -not [GlimFs]::IsLink($c.FullName)
                    $fix = if ($Repair) { Repair-Item $c.FullName $t $isDir } else { 'not repaired (no -Repair)' }
                    if ($fix -eq '' -and -not (Test-Item $c.FullName $t)) { $repaired.Add("$($c.FullName): $cw; deny put back") }
                    else { Fail "$($c.FullName): $cw; $(if ($fix) { $fix } else { 'still writable' })" }
                }
            }
        }
    } finally { $checker.Dispose() }

    # ── The firewall (setup.ps1 step 8) ─────────────────────────────────────
    $rules = @(Get-NetFirewallRule -Group 'Glim agent sandbox' -ErrorAction SilentlyContinue)
    if ($rules.Count -ne 4) { Fail "$($rules.Count) of the 4 'Glim agent sandbox' firewall rules (run setup.ps1)" }
    foreach ($r in $rules) {
        $sf = $r | Get-NetFirewallSecurityFilter
        if ($r.Enabled -ne 'True' -or $r.Action -ne 'Block' -or "$($sf.LocalUser)" -ne "D:(A;;CC;;;$sid)") { Fail "firewall rule '$($r.DisplayName)' changed" }
    }
    foreach ($p in @(Get-NetFirewallProfile | Where-Object { -not $_.Enabled })) { Fail "Windows Firewall is off for the $($p.Name) profile" }
} catch {
    Fail "the check itself failed: $($_.Exception.Message)"
}
Finish
