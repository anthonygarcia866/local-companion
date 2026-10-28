# whisper.cpp for the voice end-to-end test (CI only; Glim itself downloads
# nothing, and the owner's copy is put in %LOCALAPPDATA%\Glim\whisper by hand).
# The same release and model as on the owner's PC: whisper.cpp b5454's
# whisper-bin-x64.zip (GitHub's published SHA-256) and ggml-base.en-q5_1.bin
# (SHA-256 of the owner's copy, 2026-10-28; 59,721,011 bytes, as Hugging Face
# lists it). A file that doesn't match is deleted and the step fails.
#
#   powershell -File scripts/fetch-whisper.ps1 -Dir <folder>

param([Parameter(Mandatory = $true)][string]$Dir)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$zipUrl = 'https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip'
$zipSha = '6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb'
$modelUrl = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin'
$modelSha = '4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f'

function Test-Sha([string]$path, [string]$sha) {
    (Test-Path $path) -and ((Get-FileHash $path -Algorithm SHA256).Hash.ToLower() -eq $sha)
}

New-Item -ItemType Directory -Force -Path $Dir | Out-Null

$model = Join-Path $Dir 'ggml-base.en-q5_1.bin'
if (-not (Test-Sha $model $modelSha)) {
    Invoke-WebRequest -Uri $modelUrl -OutFile $model -UseBasicParsing
    if (-not (Test-Sha $model $modelSha)) { Remove-Item $model -Force; throw 'ggml-base.en-q5_1.bin: SHA-256 mismatch' }
    'model: downloaded, SHA-256 ok'
} else { 'model: cached, SHA-256 ok' }

# The zip's hash is checked on download; the files taken from it are noted
# with their own hashes, checked again on a cached run.
$list = Join-Path $Dir 'from-zip.sha256'
$cached = (Test-Path $list) -and (@(Get-Content $list | Where-Object { $_ } | ForEach-Object {
    $sha, $name = $_ -split ' ', 2
    Test-Sha (Join-Path $Dir $name) $sha
}) -notcontains $false)
if (-not $cached) {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "whisper-b5454-$PID"
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $zip = Join-Path $tmp 'whisper-bin-x64.zip'
    Invoke-WebRequest -Uri $zipUrl -OutFile $zip -UseBasicParsing
    if (-not (Test-Sha $zip $zipSha)) { throw 'whisper-bin-x64.zip: SHA-256 mismatch' }
    Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
    $server = Get-ChildItem (Join-Path $tmp 'x') -Recurse -Filter 'whisper-server.exe' | Select-Object -First 1
    if (-not $server) { throw 'whisper-server.exe is not in the zip' }
    $lines = foreach ($f in Get-ChildItem $server.DirectoryName -File | Where-Object { $_.Extension -in '.exe', '.dll' }) {
        Copy-Item $f.FullName (Join-Path $Dir $f.Name) -Force
        "$((Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()) $($f.Name)"
    }
    Set-Content -Path $list -Value $lines
    Remove-Item $tmp -Recurse -Force
    "whisper b5454: downloaded, zip SHA-256 ok, $(@($lines).Count) files"
} else { 'whisper b5454: cached, file hashes ok' }
"whisper-server.exe SHA-256: $((Get-FileHash (Join-Path $Dir 'whisper-server.exe') -Algorithm SHA256).Hash.ToLower())"
