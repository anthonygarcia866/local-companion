# Runs AS glim-agent (check.ps1 passes it as -EncodedCommand, so no copy of it
# sits where glim-agent could change it): tries to reach what it must not, and
# prints what happened as JSON on stdout (check.ps1 redirects that into the
# OWNER's temp folder). Never prints or keeps a file's
# contents: only "readable" / "refused" per path.
param(
    # '|'-separated (a -File argument arrives as one string).
    [string]$MustFail = '',
    [string]$EnvMustBeAbsent = ''
)
$ErrorActionPreference = 'Continue'

function Try-Read([string]$p) {
    try {
        if (Test-Path -LiteralPath $p -PathType Container -ErrorAction Stop) {
            $null = Get-ChildItem -LiteralPath $p -Force -ErrorAction Stop | Select-Object -First 1
            return 'readable'
        }
        # Not a listable folder: try to open it as a file outright (bypass
        # traverse checking means a file can open by full path even when its
        # folder refuses a listing).
        $s = [IO.File]::OpenRead($p); $s.Dispose(); return 'readable'
    } catch [IO.FileNotFoundException], [IO.DirectoryNotFoundException] {
        return 'absent'
    } catch { return 'refused' }
}

$r = [ordered]@{}
$r.whoami = (whoami)
# Its groups (SIDs) and privileges, from its own token.
# whoami /groups, not WindowsIdentity.Groups: the latter leaves out deny-only
# groups, which is exactly how Administrators shows in a non-elevated token.
$r.groups = @((whoami /groups /fo csv | ConvertFrom-Csv) | ForEach-Object { $_.SID })
$r.privileges = @((whoami /priv /fo csv | ConvertFrom-Csv) | ForEach-Object { $_.'Privilege Name' })
$r.paths = [ordered]@{}
foreach ($p in ($MustFail -split '\|' | Where-Object { $_ })) { $r.paths[$p] = Try-Read $p }
$r.drives = [ordered]@{}
foreach ($d in [IO.DriveInfo]::GetDrives()) { $r.drives[$d.Name] = Try-Read $d.Name }
$r.env = [ordered]@{}
foreach ($v in ($EnvMustBeAbsent -split '\|' | Where-Object { $_ })) { $r.env[$v] = if ([Environment]::GetEnvironmentVariable($v)) { 'present' } else { 'absent' } }
# Its own vault: names only, and only whether any Glim/ entry is there.
$list = cmdkey /list 2>&1 | Out-String
$r.glimCredentials = @([regex]::Matches($list, 'Glim/[A-Za-z0-9_-]+') | ForEach-Object { $_.Value } | Sort-Object -Unique)
$r.workWritable = try { $f = Join-Path 'C:\GlimAgent\work' ('probe-' + [guid]::NewGuid().ToString('N')); Set-Content -LiteralPath $f -Value 'x' -ErrorAction Stop; Remove-Item -LiteralPath $f -ErrorAction Stop; 'yes' } catch { 'no' }
'GLIM-PROBE-BEGIN'
$r | ConvertTo-Json -Depth 4 -Compress
'GLIM-PROBE-END'
