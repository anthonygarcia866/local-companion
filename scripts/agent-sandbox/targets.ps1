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
# LOGON, LOCAL, Authenticated Users, This Organization, Local account, NTLM,
# and BATCH (in case it is ever started as a batch job: more SIDs is the
# stricter side).
$GlimBroad = 'S-1-1-0', 'S-1-5-32-545', 'S-1-5-4', 'S-1-2-1', 'S-1-2-0', 'S-1-5-11', 'S-1-5-15', 'S-1-5-113', 'S-1-5-64-10', 'S-1-5-3'
# Write-only bits (Modify/FullControl include read bits, so they can't be the
# mask): create files, create folders, write EAs, delete child, write
# attributes, delete, change permissions, take ownership, plus GENERIC_WRITE /
# GENERIC_ALL as stored in inheritable ACEs.
$GlimWriteBits = [int64](2 -bor 4 -bor 0x10 -bor 0x40 -bor 0x100 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000)
$GlimNamed = 'C:\tests', 'C:\glimtest', 'C:\c', 'C:\Drivers', 'C:\Users\Public', 'C:\Windows\Tasks',
    # Inside C:\Windows, where all users may write (review 3 M-4, 2026-11-26):
    'C:\Windows\Temp', 'C:\Windows\tracing', 'C:\Windows\Registration\CRMLog',
    'C:\Windows\System32\Tasks', 'C:\Windows\SysWOW64\Tasks', 'C:\Windows\System32\com\dmp',
    'C:\Windows\System32\spool\PRINTERS', 'C:\Windows\System32\spool\drivers\color',
    'C:\Windows\System32\Microsoft\Crypto\RSA\MachineKeys'
# + WDAC (change permissions) and WO (take ownership): without them glim-agent
# could rewrite a Full Control folder's ACL and delete its own deny. Neither
# touches reading.
$GlimDenyRights = '(WD,AD,WEA,WA,D,DC,WDAC,WO)'

# What lets all users write $Path ("" if nothing): its allow entries for a
# logon SID that apply to it, read from the object itself (a link: its own).
# Needs authz.ps1.
function Get-GlimBroadWrite([string]$Path) {
    $e = 0
    $sd = [GlimFs]::ReadSd($Path, [ref]$e)
    if ($null -eq $sd) { return '' }
    [GlimFs]::Grant($sd, [string[]]$GlimBroad, [uint32]$GlimWriteBits, $false)
}
function Test-GlimBroadWrite([string]$Path) { [bool](Get-GlimBroadWrite $Path) }

# Read or write, for the profile (fix 2, owner 2026-11-22: glim-agent may
# read nothing in it): FILE_READ_DATA, GENERIC_READ and the write bits.
$GlimReadWriteBits = [int64]($GlimWriteBits -bor 0x1 -bor [int64]2147483648)

# Folders a walk couldn't list (review 5: skipping them silently hid
# everything below). Get-GlimTreeItems adds to it; callers report it.
$GlimUnlistable = New-Object System.Collections.Generic.List[string]

# PATH entries that couldn't be resolved to a real folder: NOT verified
# (check.ps1 ends INCOMPLETE). Review 6: a quoted entry used to be dropped.
$GlimPathUnresolved = New-Object System.Collections.Generic.List[string]

# Each PATH entry (system, then the owner's) as cmd.exe would use it:
# variables expanded, surrounding quotes removed (cmd strips them).
function Get-GlimPathEntries {
    foreach ($scope in 'Machine', 'User') {
        foreach ($e in ([Environment]::GetEnvironmentVariable('Path', $scope) -split ';')) {
            $x = [Environment]::ExpandEnvironmentVariables($e).Trim().Trim('"').Trim().TrimEnd('\')
            if (-not $x) { continue }
            if ($x -match '^[A-Za-z]:$') { $x += '\' }
            # Only a full path: drive + root, or \\server\share. A relative one
            # ("." or "bin", or an unexpanded %VAR%) depends on the folder the
            # lookup starts in, so it can't be checked: NOT verified (review 7).
            if ($x -notmatch '^[A-Za-z]:\\' -and $x -notmatch '^\\\\[^\\]+\\[^\\]+') {
                if (-not $GlimPathUnresolved.Contains($x)) { $GlimPathUnresolved.Add($x) }
                continue
            }
            $x
        }
    }
}

# Every existing PATH folder, as the REAL folder it leads to (a junction or
# symlink, at the end or on the way, is followed: review 6, a link's own ACL
# says nothing about its target's). Needs authz.ps1.
function Get-GlimPathFolders {
    $seen = @{}
    foreach ($x in (Get-GlimPathEntries)) {
        $ok = $false
        try { $ok = Test-Path -LiteralPath $x -PathType Container } catch { }
        if (-not $ok) { continue }
        $final = [GlimAuthz]::FinalPath($x)
        if (-not $final) { if (-not $GlimPathUnresolved.Contains($x)) { $GlimPathUnresolved.Add($x) }; continue }
        if (-not $seen.ContainsKey($final.ToLowerInvariant())) { $seen[$final.ToLowerInvariant()] = $true; $final }
    }
}

# The item at $Path if anything is there, a broken link included (Test-Path
# says False for a link whose target is gone; Get-Item -Force still sees it).
function Get-GlimItemOrNull([string]$Path) {
    try { return Get-Item -LiteralPath $Path -Force -ErrorAction Stop } catch { return $null }
}

# PATH entries whose folder doesn't exist yet: if glim-agent could create
# one, it could plant programs on PATH. Returns the real path of the nearest
# existing folder on the way (where it would be created), which must not be
# writable. A link on the way is resolved; a broken one leaves the entry NOT
# verified (its target is wherever the link says, and nothing is there yet).
function Get-GlimMissingPathParents {
    $seen = @{}
    foreach ($x in (Get-GlimPathEntries)) {
        $exists = $false
        try { $exists = Test-Path -LiteralPath $x } catch { }
        if ($exists) { continue }
        $par = $x
        $final = $null
        while ($par) {
            $it = Get-GlimItemOrNull $par
            if ($it) { $final = [GlimAuthz]::FinalPath($par); break }   # null for a broken link
            $par = Split-Path -Parent $par
        }
        if (-not $final) { if (-not $GlimPathUnresolved.Contains($x)) { $GlimPathUnresolved.Add($x) }; continue }
        if (-not $seen.ContainsKey($final.ToLowerInvariant())) { $seen[$final.ToLowerInvariant()] = $true; $final }
    }
}

# Every folder a PATH lookup passes through, both as written and as really
# reached (review 7): each literal part of every PATH entry from the entry up
# to its drive root (a link there is judged by its own ACL, since retargeting
# or replacing it redirects the lookup), and every folder above each real
# PATH folder and above each missing entry's nearest existing folder. With
# write or delete-child on any of them glim-agent could swap what's below.
function Get-GlimPathAncestors {
    $seen = @{}
    $emit = { param($q) if ($q -and -not $seen.ContainsKey($q.ToLowerInvariant())) { $seen[$q.ToLowerInvariant()] = $true; $q } }
    foreach ($x in (Get-GlimPathEntries)) {
        $q = $x
        while ($q) {
            if (Get-GlimItemOrNull $q) { & $emit $q }
            $q = Split-Path -Parent $q
        }
    }
    foreach ($f in @(Get-GlimPathFolders) + @(Get-GlimMissingPathParents)) {
        $q = Split-Path -Parent $f
        while ($q) { & $emit $q; $q = Split-Path -Parent $q }
    }
}

# The files directly in each PATH folder (what PATH lookup runs).
function Get-GlimPathFiles {
    foreach ($f in (Get-GlimPathFolders)) {
        $errs = $null
        foreach ($c in @(Get-ChildItem -LiteralPath $f -File -Force -ErrorAction SilentlyContinue -ErrorVariable errs)) { $c.FullName }
        if ($errs) { if (-not $GlimUnlistable.Contains($f)) { $GlimUnlistable.Add($f) } }
    }
}

# $Path is $Base itself or inside it (not merely sharing a prefix:
# C:\Users\antho2 is not inside C:\Users\antho).
function Test-GlimUnder([string]$Path, [string]$Base) {
    $b = $Base.TrimEnd('\')
    $Path -ieq $b -or $Path -like "$b\*"
}

# Links (junctions, symlinks) that all users could write or retarget. They
# are never lock targets (icacls would change the link's TARGET, wherever it
# points); setup lists them and stops, to be fixed by hand.
$GlimLinkFindings = New-Object System.Collections.Generic.List[string]

# Every file and folder under $Root, walked by hand, never following a
# junction or symlink (it is yielded, so its own ACL is checked, but not
# entered). A folder it can't list goes on $GlimUnlistable.
function Get-GlimTreeItems([string]$Root) {
    $queue = New-Object System.Collections.Generic.Queue[string]
    $queue.Enqueue($Root)
    while ($queue.Count) {
        $dir = $queue.Dequeue()
        $errs = $null
        $kids = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue -ErrorVariable errs)
        if ($errs) { if (-not $GlimUnlistable.Contains($dir)) { $GlimUnlistable.Add($dir) } }
        foreach ($c in $kids) {
            $c
            if ($c.PSIsContainer -and -not ($c.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $queue.Enqueue($c.FullName) }
        }
    }
}

# Items under $Root with an EXPLICIT (not inherited) allow entry that lets a
# logon SID write: Windows applies an object's explicit allows before the
# deny it inherits, so the tree's deny doesn't stop them (review 2026-11-22;
# inheritance being off is only one way to get there). Each needs its own.
# Walks $Root with the native walker (folders listed by handle, links'
# own ACLs read, never entered; the backup privilege when elevated) and
# returns @{ Path; Dir; Why } for each item whose own entry gives a logon SID
# a bit of $Mask. Folders it can't list go on $GlimUnlistable.
function Get-GlimExplicitWritable([string]$Root, [int64]$Mask = $GlimWriteBits) {
    $r = [GlimFs]::Walk($Root, 'explicit', [string[]]$GlimBroad, [uint32]$Mask, $null)
    foreach ($u in $r.Unlistable) { if (-not $GlimUnlistable.Contains($u)) { $GlimUnlistable.Add($u) } }
    # Items whose permissions couldn't be read can't be judged: reported, not skipped (review 8).
    foreach ($u in $r.Unreadable) { $x = "$u (permissions unreadable)"; if (-not $GlimUnlistable.Contains($x)) { $GlimUnlistable.Add($x) } }
    for ($i = 0; $i -lt $r.Hits.Count; $i++) { [pscustomobject]@{ Path = $r.Hits[$i]; Dir = $r.HitIsDir[$i]; Why = $r.Details[$i] } }
}

# After locking: everything in the targets (trees walked in full) that
# glim-agent can still write, and everything whose permissions couldn't be
# read (so it couldn't be verified). Needs authz.ps1. Returns @{ Writable; Unreadable }.
function Get-GlimStillWritable([string]$Sid, $Targets) {
    $writable = New-Object System.Collections.Generic.List[string]
    $unreadable = New-Object System.Collections.Generic.List[string]
    $seen = @{}
    $checker = New-Object GlimAuthz+Checker $Sid
    try {
        foreach ($t in $Targets) {
            # A target denied everything (in your profile) must not be readable either.
            $mask = if ($t.Deny -eq 'all') { $GlimRead -bor $GlimWrite } else { $GlimWrite }
            $k = $t.Path.ToLowerInvariant()
            if (-not $seen.ContainsKey($k)) {
                $seen[$k] = $true
                $e = 0
                $sd = [GlimFs]::ReadSd($t.Path, [ref]$e)
                if ($null -eq $sd) { $unreadable.Add("$($t.Path) (error $e)") }
                elseif ($checker.Granted($sd) -band $mask) { $writable.Add($t.Path) }
            }
            if ($t.Scope -eq 'tree') {
                $r = [GlimFs]::Walk($t.Path, 'access', $null, [uint32]$mask, $checker)
                foreach ($h in $r.Hits) { if (-not $seen.ContainsKey($h.ToLowerInvariant())) { $seen[$h.ToLowerInvariant()] = $true; $writable.Add($h) } }
                foreach ($u in $r.Unreadable) { $unreadable.Add($u) }
                foreach ($u in $r.Unlistable) { $unreadable.Add("$u (couldn't list its contents)") }
            }
        }
    } finally { $checker.Dispose() }
    foreach ($d in $GlimUnlistable) { $x = "$d (couldn't list its contents)"; if (-not $unreadable.Contains($x)) { $unreadable.Add($x) } }
    @{ Writable = @($writable | Sort-Object -Unique); Unreadable = @($unreadable | Sort-Object -Unique) }
}

function Get-GlimLockTargets([string]$OwnerProfile) {
    $out = [ordered]@{}
    function Add-Target([string]$p, [string]$scope, [string]$why, [string]$deny = 'write') {
        $k = $p.ToLowerInvariant()
        if (Test-GlimUnder $p 'C:\GlimAgent') { return }
        if ($deny -ne 'all' -and (Test-GlimUnder $p $OwnerProfile)) { return }
        # Only a real link (junction, mount point, symlink) is diverted: icacls
        # would change its target. A cloud placeholder is locked like any folder.
        if ([GlimFs]::IsLink($p)) {
            if (-not $GlimLinkFindings.Contains($p)) { $GlimLinkFindings.Add($p) }
            return
        }
        if (-not $out.Contains($k)) { $out[$k] = [pscustomobject]@{ Path = $p; Scope = $scope; Why = $why; Deny = $deny } }
    }
    foreach ($n in $GlimNamed) {
        if (Test-Path -LiteralPath $n -PathType Container) {
            $w = Get-GlimBroadWrite $n
            Add-Target $n 'tree' ('on the named list' + $(if ($w) { "; writable now by $w" } else { '' }))
        }
    }
    foreach ($d in (Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -in 2, 3 })) {
        $root = $d.DeviceID + '\'
        if (-not (Test-Path -LiteralPath $root)) { continue }
        $w = Get-GlimBroadWrite $root
        if ($w) { Add-Target $root 'folder' "drive root; writable by $w" }
        $errs = $null
        $rootKids = @(Get-ChildItem -LiteralPath $root -Force -ErrorAction SilentlyContinue -ErrorVariable errs)
        if ($errs) { if (-not $GlimUnlistable.Contains($root)) { $GlimUnlistable.Add($root) } }
        foreach ($c in @($rootKids | Where-Object { $_.PSIsContainer })) {
            $w = Get-GlimBroadWrite $c.FullName
            if ($w) {
                $scope = if ($c.Name -eq '$Recycle.Bin') { 'folder' } else { 'tree' }
                Add-Target $c.FullName $scope "top-level folder; writable by $w"
            }
        }
        # Files directly at the root (a folder-only deny on the root doesn't reach them).
        foreach ($f in @($rootKids | Where-Object { -not $_.PSIsContainer })) {
            $w = Get-GlimBroadWrite $f.FullName
            if ($w) { Add-Target $f.FullName 'file' "file at the drive root; writable by $w" }
        }
    }
    foreach ($p in (Get-GlimPathFolders)) {
        $w = Get-GlimBroadWrite $p
        if ($w) { Add-Target $p 'tree' "on PATH; writable by $w" }
    }
    foreach ($p in (Get-GlimMissingPathParents)) {
        $w = Get-GlimBroadWrite $p
        if ($w) { Add-Target $p 'folder' "parent of a PATH folder that does not exist yet; writable by $w" }
    }
    foreach ($p in (Get-GlimPathAncestors)) {
        # A literal PATH entry itself is in this set too: a tree if it's a real
        # folder all users can write (it's also a PATH folder), else 'folder'.
        # A link is diverted to $GlimLinkFindings by Add-Target.
        $w = Get-GlimBroadWrite $p
        if ($w) { Add-Target $p 'folder' "on the way to a PATH folder; writable by $w" }
    }
    foreach ($p in (Get-GlimPathFiles)) {
        $w = Get-GlimBroadWrite $p
        if ($w) { Add-Target $p 'file' "program in a PATH folder; writable by $w" }
    }
    # Inside every tree: files and folders whose own (explicit) entry lets all
    # users write: the tree's inherited deny doesn't beat it, so each gets its own.
    foreach ($t in @($out.Values | Where-Object { $_.Scope -eq 'tree' })) {
        foreach ($sub in (Get-GlimExplicitWritable $t.Path)) {
            $scope = if ($sub.Dir) { 'tree' } else { 'file' }
            Add-Target $sub.Path $scope "inside $($t.Path); its own entry lets write: $($sub.Why)"
        }
    }
    # Your profile (fix 2, owner 2026-11-22): every file or folder in it whose
    # own entry lets all users read or write escapes the profile's inherited
    # deny, so it gets its own deny of everything for glim-agent.
    foreach ($sub in (Get-GlimExplicitWritable $OwnerProfile $GlimReadWriteBits)) {
        $scope = if ($sub.Dir) { 'tree' } else { 'file' }
        Add-Target $sub.Path $scope "in your profile; its own entry lets all users in: $($sub.Why)" 'all'
    }
    $out.Values
}
