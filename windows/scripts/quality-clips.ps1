# Speech clips for the dictation quality test set (tests/data/dictation-quality.json,
# src-tauri/src/voice/quality_tests.rs): every case spoken by up to -Voices
# of Windows' English SAPI voices (2-3 on GitHub's runners), 16 kHz mono
# 16-bit WAV, named <case id>__v<n>.wav. Made-up content only; nothing is
# recorded and no clip is kept in the repo: CI makes them on each run.
#
#   powershell -File scripts/quality-clips.ps1 -Out <folder> [-Voices 3]

param([Parameter(Mandatory = $true)][string]$Out, [int]$Voices = 3)
$ErrorActionPreference = 'Stop'

$data = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot '..\tests\data\dictation-quality.json') | ConvertFrom-Json
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$english = @($synth.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like 'en-*' } | Select-Object -First $Voices)
if ($english.Count -eq 0) { throw 'no English SAPI voice on this machine' }
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$n = 0
for ($v = 0; $v -lt $english.Count; $v++) {
    $synth.SelectVoice($english[$v].VoiceInfo.Name)
    # A little slower than default for one voice, as people dictate.
    $synth.Rate = if ($v -eq 1) { -1 } else { 0 }
    foreach ($case in $data.cases) {
        $synth.SetOutputToWaveFile((Join-Path $Out "$($case.id)__v$($v + 1).wav"), $format)
        $synth.Speak($case.said)
        $n++
    }
}
$synth.SetOutputToNull()
$synth.Dispose()
"quality clips: $n made, $($data.cases.Count) cases x $($english.Count) voice(s): $(($english | ForEach-Object { $_.VoiceInfo.Name }) -join ', ')"
