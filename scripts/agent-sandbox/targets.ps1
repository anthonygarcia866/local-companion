# What glim-agent must not write (owner 2026-11-22): one list, shared by
# setup.ps1 (which locks it), check.ps1 (which proves it) and remove.ps1
# (which unlocks it). Dot-source it.
#
# Get-GlimLockTargets: the folders to lock with a deny for glim-agent only
# (write + delete: WD,AD,WEA,WA,D,DC,WDAC,WO; never plain "W", which also denies
# SYNCHRONIZE and so breaks reading). Each has a Scope:
#   tree   = the folder and everything in it ((OI)(CI))
#   folder = the folder itself only (it can't create or delete entries there,
#            nothing below changes)
#   file   = one file (at a drive root)
# It is: the owner's named folders (C:\tests, C:\glimtest, C:\c, C:\Drivers),
# every top-level folder on a fixed or removable drive that any of those
# logon SIDs can write (allow entries; a deny added by setup doesn't take it
# off the list, so check and remove still see it), every PATH folder (system and
# the owner's) outside the owner's profile that they can write, and each
# drive root (folder only: a tree deny on C:\ would touch the whole drive).
# Exceptions, both folder-only: C:\$Recycle.Bin (its per-user folders belong
# to other accounts) and nothing else. C:\GlimAgent is never a target.
#
# Get-GlimPathFolders: every PATH folder (system + the owner's), existing.

# Every SID an interactive glim-agent logon carries besides its own and its
# groups' (the same set authz.ps1 adds): Everyone, Users, INTERACTIVE, CONSOLE
# LOGON, LOCAL, Authenticated Users, This Organization, Local account, NTLM.
$GlimBroad = 'S-1-1-0', 'S-1-5-32-545', 'S-1-5-4', 'S-1-2-1', 'S-1-2-0', 'S-1-5-11', 'S-1-5-15', 'S-1-5-113', 'S-1-5-64-10'
# Write-only bits (Modify/FullControl include read bits, so they can't be the
# mask): create files, create folders, delete, change permissions, take
# ownership, plus GENERIC_WRITE / GENERIC_ALL as stored in inheritable ACEs.
$GlimWriteBits = [int64](2 -bor 4 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000)
$GlimNamed = 'C:\tests', 'C:\glimtest', 'C:\c', 'C:\Drivers'
# + WDAC (change permissions) and WO (take ownership): without them glim-agent
# could rewrite a Full Control folder's ACL and delete its own deny. Neither
# touches reading.
$GlimDenyRights = '(WD,AD,WEA,WA,D,DC,WDAC,WO)'

function Test-GlimBroadWrite([string]$Path) {
    try { $acl = Get-Acl -LiteralPath $Path } catch { return $false }
    foreach ($a in $acl.Access) {
        if ($a.AccessControlType -ne 'Allow') { continue }
        if ($a.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) { continue }
        try { $sid = $a.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { continue }
        if ($GlimBroad -contains $sid -and (([int64]$a.FileSystemRights) -band $GlimWriteBits)) { return $true }
    }
    $false
}

function Get-GlimPathFolders {
    $seen = @{}
    foreach ($scope in 'Machine', 'User') {
        foreach ($e in ([Environment]::GetEnvironmentVariable('Path', $scope) -split ';')) {
            if (-not $e.Trim()) { continue }
            $x = [Environment]::ExpandEnvironmentVariables($e.Trim()).TrimEnd('\')
            if ($x -match '^[A-Za-z]:$') { $x += '\' }
            if (-not $seen.ContainsKey($x.ToLowerInvariant()) -and (Test-Path -LiteralPath $x -PathType Container)) {
                $seen[$x.ToLowerInvariant()] = $true
                $x
            }
        }
    }
}

# $Path is $Base itself or inside it (not merely sharing a prefix:
# C:\Users\antho2 is not inside C:\Users\antho).
function Test-GlimUnder([string]$Path, [string]$Base) {
    $b = $Base.TrimEnd('\')
    $Path -ieq $b -or $Path -like "$b\*"
}

# Every file and folder under $Root, walked by hand, never following a
# junction or symlink (it is yielded, not entered).
function Get-GlimTreeItems([string]$Root) {
    $queue = New-Object System.Collections.Generic.Queue[string]
    $queue.Enqueue($Root)
    while ($queue.Count) {
        $dir = $queue.Dequeue()
        foreach ($c in (Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)) {
            $c
            if ($c.PSIsContainer -and -not ($c.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $queue.Enqueue($c.FullName) }
        }
    }
}

# Items under $Root with an EXPLICIT (not inherited) allow entry that lets a
# logon SID write: Windows applies an object's explicit allows before the
# deny it inherits, so the tree's deny doesn't stop them (review 2026-11-22;
# inheritance being off is only one way to get there). Each needs its own.
function Get-GlimExplicitWritable([string]$Root) {
    foreach ($c in (Get-GlimTreeItems $Root)) {
        if ($c.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
        $acl = $null
        try { $acl = Get-Acl -LiteralPath $c.FullName } catch { continue }
        foreach ($a in $acl.Access) {
            if ($a.IsInherited -or $a.AccessControlType -ne 'Allow') { continue }
            try { $sid = $a.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { continue }
            if ($GlimBroad -contains $sid -and (([int64]$a.FileSystemRights) -band $GlimWriteBits)) { $c; break }
        }
    }
}

# After locking: everything in the targets (trees walked in full) that
# glim-agent can still write, and everything whose permissions couldn't be
# read (so it couldn't be verified). Needs authz.ps1. Returns @{ Writable; Unreadable }.
function Get-GlimStillWritable([string]$Sid, $Targets) {
    $writable = New-Object System.Collections.Generic.List[string]
    $unreadable = New-Object System.Collections.Generic.List[string]
    $seen = @{}
    function Probe([string]$p) {
        $k = $p.ToLowerInvariant()
        if ($seen.ContainsKey($k)) { return }
        $seen[$k] = $true
        $m = $null
        try { $m = Get-GlimAccess $Sid $p } catch { }
        if ($null -eq $m) { $unreadable.Add($p) }
        elseif ($m -band $GlimWrite) { $writable.Add($p) }
    }
    foreach ($t in $Targets) {
        Probe $t.Path
        if ($t.Scope -eq 'tree') {
            foreach ($c in (Get-GlimTreeItems $t.Path)) {
                if ($c.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
                Probe $c.FullName
            }
        }
    }
    @{ Writable = @($writable); Unreadable = @($unreadable) }
}

function Get-GlimLockTargets([string]$OwnerProfile) {
    $out = [ordered]@{}
    function Add-Target([string]$p, [string]$scope, [string]$why) {
        $k = $p.ToLowerInvariant()
        if (Test-GlimUnder $p 'C:\GlimAgent') { return }
        if (Test-GlimUnder $p $OwnerProfile) { return }
        if (-not $out.Contains($k)) { $out[$k] = [pscustomobject]@{ Path = $p; Scope = $scope; Why = $why } }
    }
    foreach ($n in $GlimNamed) { if (Test-Path -LiteralPath $n -PathType Container) { Add-Target $n 'tree' 'named by you' } }
    foreach ($d in (Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -in 2, 3 })) {
        $root = $d.DeviceID + '\'
        if (-not (Test-Path -LiteralPath $root)) { continue }
        if (Test-GlimBroadWrite $root) { Add-Target $root 'folder' 'drive root, writable by all users' }
        foreach ($c in (Get-ChildItem -LiteralPath $root -Directory -Force -ErrorAction SilentlyContinue)) {
            if (Test-GlimBroadWrite $c.FullName) {
                $scope = if ($c.Name -eq '$Recycle.Bin') { 'folder' } else { 'tree' }
                Add-Target $c.FullName $scope 'top-level folder, writable by all users'
            }
        }
        # Files directly at the root (a folder-only deny on the root doesn't reach them).
        foreach ($f in (Get-ChildItem -LiteralPath $root -File -Force -ErrorAction SilentlyContinue)) {
            if (Test-GlimBroadWrite $f.FullName) { Add-Target $f.FullName 'file' 'file at the drive root, writable by all users' }
        }
    }
    foreach ($p in (Get-GlimPathFolders)) {
        if (Test-GlimBroadWrite $p) { Add-Target $p 'tree' 'on PATH, writable by all users' }
    }
    # Inside every tree: files and folders whose own (explicit) entry lets all
    # users write: the tree's inherited deny doesn't beat it, so each gets its own.
    foreach ($t in @($out.Values | Where-Object { $_.Scope -eq 'tree' })) {
        foreach ($sub in (Get-GlimExplicitWritable $t.Path)) {
            $scope = if ($sub.PSIsContainer) { 'tree' } else { 'file' }
            Add-Target $sub.FullName $scope "inside $($t.Path), its own entry lets all users write"
        }
    }
    $out.Values
}
