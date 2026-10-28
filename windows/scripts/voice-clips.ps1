# Speech clips for the voice end-to-end test (src-tauri/src/voice/e2e_tests.rs),
# made with Windows' own SAPI voices: 16 kHz mono 16-bit WAV, what Glim's
# microphone code hands to whisper. Nothing is recorded and no clip is kept
# in the repo: CI makes them on each run.
#
#   powershell -File scripts/voice-clips.ps1 -Out <folder>

param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Stop'

$clips = [ordered]@{
    'open-outlook'      = 'Open Outlook.'
    'routine-work-mode' = 'When I say work mode, open Outlook and put it on the left, then open Chrome and put it on the right.'
    'work-mode'         = 'Work mode.'
    'remember-claude'   = 'Remember: open Claude means the desktop app.'
    # A test card number (Visa's documented test number), digit by digit.
    'remember-card'     = 'Remember my card number, 4 1 1 1, 1 1 1 1, 1 1 1 1, 1 1 1 1.'
    'open-claude'       = 'Open Claude.'
    # The window action.
    'outlook-left'      = 'Put Outlook on the left.'
    'maximize-chrome'   = 'Maximize Google Chrome.'
    'open-the-office'   = 'Open the office.'
    'remember-office'   = 'Remember: when I say the office, I mean Outlook.'
    # Made-up names (the run's "Words Glim should know") and a unit.
    'dictation'         = 'Please tell Zorvath Quillane that unit four B is ready for the walkthrough.'
}

Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voices = @($synth.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like 'en-*' })
if ($voices.Count -eq 0) { throw 'no English SAPI voice on this machine' }
$voice = $voices[0].VoiceInfo.Name
$synth.SelectVoice($voice)
$synth.Rate = -1
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
New-Item -ItemType Directory -Force -Path $Out | Out-Null
foreach ($id in $clips.Keys) {
    $synth.SetOutputToWaveFile((Join-Path $Out "$id.wav"), $format)
    $synth.Speak($clips[$id])
}
$synth.SetOutputToNull()
$synth.Dispose()
"voice clips: $($clips.Count) made with $voice ($($voices.Count) English voices installed)"
