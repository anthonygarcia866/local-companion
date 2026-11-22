# What glim-agent must not write (owner 2026-11-22): one list, shared by
# setup.ps1 (which locks it), check.ps1 (which proves it) and remove.ps1
# (which unlocks it). Dot-source it.
#
# Get-GlimLockTargets: the folders to lock with a deny for glim-agent only
# (write + delete: WD,AD,WEA,WA,D,DC; never plain "W", which also denies
# SYNCHRONIZE and so breaks reading). Each has a Scope:
#   tree   = the folder and everything in it ((OI)(CI))
#   folder = the folder itself only (it can't create or delete entries there,
#            nothing below changes)
# It is: the owner's named folders (C:\tests, C:\glimtest, C:\c, C:\Drivers),
# every top-level folder on a fixed or removable drive that Users,
# Authenticated Users or Everyone can write, every PATH folder (system and
# the owner's) outside the owner's profile that they can write, and each
# drive root (folder only: a tree deny on C:\ would touch the whole drive).
# Exceptions, both folder-only: C:\$Recycle.Bin (its per-user folders belong
# to other accounts) and nothing else. C:\GlimAgent is never a target.
#
# Get-GlimPathFolders: every PATH folder (system + the owner's), existing.

$GlimBroad = 'S-1-5-32-545', 'S-1-5-11', 'S-1-1-0'
# Write-only bits (Modify/FullControl include read bits, so they can't be the
# mask): create files, create folders, delete, change permissions, take
# ownership, plus GENERIC_WRITE / GENERIC_ALL as stored in inheritable ACEs.
$GlimWriteBits = [int64](2 -bor 4 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000)
$GlimNamed = 'C:\tests', 'C:\glimtest', 'C:\c', 'C:\Drivers'
$GlimDenyRights = '(WD,AD,WEA,WA,D,DC)'

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

function Get-GlimLockTargets([string]$OwnerProfile) {
    $out = [ordered]@{}
    function Add-Target([string]$p, [string]$scope, [string]$why) {
        $k = $p.ToLowerInvariant()
        if ($p -like 'C:\GlimAgent*') { return }
        if (-not $out.Contains($k)) { $out[$k] = [pscustomobject]@{ Path = $p; Scope = $scope; Why = $why } }
    }
    foreach ($n in $GlimNamed) { if (Test-Path -LiteralPath $n -PathType Container) { Add-Target $n 'tree' 'named by you' } }
    foreach ($d in (Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -in 2, 3 })) {
        $root = $d.DeviceID + '\'
        if (-not (Test-Path -LiteralPath $root)) { continue }
        if (Test-GlimBroadWrite $root) { Add-Target $root 'folder' 'drive root, writable by all users' }
        foreach ($c in (Get-ChildItem -LiteralPath $root -Directory -Force -ErrorAction SilentlyContinue)) {
            if ($c.FullName -like "$OwnerProfile*") { continue }
            if (Test-GlimBroadWrite $c.FullName) {
                $scope = if ($c.Name -eq '$Recycle.Bin') { 'folder' } else { 'tree' }
                Add-Target $c.FullName $scope 'top-level folder, writable by all users'
            }
        }
    }
    foreach ($p in (Get-GlimPathFolders)) {
        if ($p -like "$OwnerProfile*") { continue }
        if (Test-GlimBroadWrite $p) { Add-Target $p 'tree' 'on PATH, writable by all users' }
    }
    $out.Values
}
