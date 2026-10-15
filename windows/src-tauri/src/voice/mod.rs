// Voice 1.5b: push-to-talk dictation (docs/voice-actions-plan.md §1, §4).
//
// Hold Ctrl+Alt+Shift+V (Anthony, 2026-10-14: registered globally, Word's
// InsertAutoText on that key is given up), speak, let go: the microphone
// records into memory only (mic.rs), whisper.cpp's server on 127.0.0.1
// transcribes it (whisper.rs; the audio is POSTed from memory, never written
// to disk), the Claude API cleans it up (ai/dictate.rs), and the text is typed
// into the field the user is in (writing::apply::insert, 1b's checks).
//
// This file holds the rules, pure and tested: what happens with a transcript
// and the clean-up's answer (`decide`), the cautious no-Claude rule for a
// spoken command (`starts_like_command`, §4.1, the list confirmed by Anthony
// 2026-10-14), and the in-memory WAV and multipart encoding.
//
// Transcripts are captured text: never logged (only phases, durations,
// counts and codes), scanned for card/SSN/IBAN numbers before anything
// leaves the machine and again before anything is typed.

#[cfg(windows)]
pub mod mic;
#[cfg(windows)]
pub mod run;
pub mod whisper;

use serde::Serialize;

/// An editable, non-password field had the focus when the key went down
/// (set by the capture thread on Request::VoiceBegin).
pub static FIELD_AT_PRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Shorter than this held is an accidental press: nothing recorded or sent.
pub const MIN_HELD_MS: u64 = 400;
/// The longest recording; then it stops as if released.
pub const MAX_HELD_MS: u64 = 60_000;
/// The microphone's format: 16 kHz, mono, 16-bit (what whisper wants).
pub const SAMPLE_RATE: u32 = 16_000;

/// Leading filler dropped before the command check (§4.1).
const FILLER: [&str; 7] = ["hey glim", "hi glim", "ok glim", "okay", "um", "uh", "so"];
/// The command words (§4.1, confirmed as-is by Anthony 2026-10-14).
pub const COMMAND_WORDS: [&str; 10] = ["open", "pick up", "resume", "where was i", "draft", "email", "send", "reply", "delete", "go to"];

fn words(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// §4.1: without Claude, Glim can't classify an utterance; a transcript that
/// starts like a command (after leading filler, any case) is never typed.
pub fn starts_like_command(transcript: &str) -> bool {
    let mut t = words(transcript);
    loop {
        let before = t.len();
        for f in FILLER {
            if t == f {
                t.clear();
            } else if let Some(rest) = t.strip_prefix(&format!("{f} ")) {
                t = rest.to_string();
            }
        }
        if t.len() == before {
            break;
        }
    }
    COMMAND_WORDS.iter().any(|c| t == *c || t.starts_with(&format!("{c} ")))
}

/// The clean-up step's answer (ai::dictate), already validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cleanup {
    /// `dictate`: the cleaned text to type.
    Dictate(String),
    /// `clarify`: a question for the user.
    Clarify(String),
    /// Any action tool (`open_app`, `resume_work`, `draft_email`): not built
    /// in this PR (1.5c), so "couldn't act on it".
    Command(&'static str),
}

/// Why there is no clean-up (§4.1): each of these falls back to the raw
/// transcript for dictation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupFailed {
    /// The API is unreachable or answered with an error, or an answer Glim
    /// can't use (no valid tool call).
    Outage(String),
    /// No answer within ~4 s: cancelled, a late answer discarded.
    Timeout,
    NoKey,
    /// The monthly spend cap is reached: nothing was sent.
    Cap,
}

impl CleanupFailed {
    pub fn code(&self) -> String {
        match self {
            CleanupFailed::Outage(c) => c.clone(),
            CleanupFailed::Timeout => "timeout".into(),
            CleanupFailed::NoKey => "no-key".into(),
            CleanupFailed::Cap => "cap".into(),
        }
    }
}

/// What the island is shown, and whether anything is typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Type `text` into the field. `cleaned` false = the raw transcript
    /// (the "not cleaned up" marker on Glim's card and badge only).
    Type { text: String, cleaned: bool },
    /// "Here's what I heard: … (couldn't act on it)": nothing happens.
    Heard { transcript: String },
    /// Claude asks something back; nothing typed.
    Question { question: String },
    /// Not sent / not typed: a card, SSN or IBAN number.
    Sensitive,
}

/// Everything §4.1 decides, after the transcript and the clean-up.
/// `field_at_press`: an editable, non-password field had the focus when the
/// key went down (the capture thread's last reading).
pub fn decide(transcript: &str, cleanup: Result<Cleanup, CleanupFailed>, field_at_press: bool) -> Decision {
    if crate::capture::sensitive::contains_sensitive_number(transcript) {
        return Decision::Sensitive;
    }
    match cleanup {
        Ok(Cleanup::Dictate(text)) => {
            // Claude's text is scanned again before typing.
            if crate::capture::sensitive::contains_sensitive_number(&text) {
                Decision::Sensitive
            } else if field_at_press {
                Decision::Type { text, cleaned: true }
            } else {
                Decision::Heard { transcript: transcript.to_string() }
            }
        }
        Ok(Cleanup::Clarify(question)) => Decision::Question { question },
        Ok(Cleanup::Command(_)) => Decision::Heard { transcript: transcript.to_string() },
        // §4.1: dictation types the raw transcript; a command never is.
        Err(_) if field_at_press && !starts_like_command(transcript) => Decision::Type { text: transcript.trim().to_string(), cleaned: false },
        Err(_) => Decision::Heard { transcript: transcript.to_string() },
    }
}

/// What the island hears on `EVENT`: phases and codes; the transcript only
/// where the card shows it (never logged).
pub const EVENT: &str = "voice";

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEvent {
    /// "listening" (with `level`), "transcribing", "cleaning", "typed",
    /// "heard", "question", "failed", "idle".
    pub phase: &'static str,
    /// 0..1, the microphone level, while listening.
    pub level: f32,
    /// "typed": false = the raw transcript, not cleaned up.
    pub cleaned: bool,
    /// "heard": the transcript; "question": Claude's question.
    pub text: String,
    /// "failed" / "typed" outcome codes ("no-whisper", "sensitive", "paused",
    /// an apply Outcome code…); "heard"/"typed": why it wasn't cleaned up.
    pub code: String,
    /// The app typed into ("Word").
    pub app: String,
}

/// 16-bit mono PCM as a WAV file, in memory.
pub fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A multipart/form-data body: text fields, then one file. Returns the
/// content type (with its boundary) and the body.
pub fn multipart(fields: &[(&str, &str)], file_field: &str, file_name: &str, file: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----glim-voice-7f3a9c2e5b1d";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// The microphone level of a chunk, 0..1 (RMS, scaled so speech reaches ~1).
pub fn level(chunk: &[i16]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    let sum: f64 = chunk.iter().map(|&s| (s as f64) * (s as f64)).sum();
    ((sum / chunk.len() as f64).sqrt() / 3000.0).min(1.0) as f32
}

/// The personal word list (Settings) as whisper's prompt: one line, commas,
/// at most 400 characters (whisper keeps half its context for the prompt).
pub fn prompt_from(words: &str) -> String {
    let list: Vec<&str> = words.split([',', '\n', ';']).map(str::trim).filter(|w| !w.is_empty()).collect();
    let mut out = String::new();
    for w in list {
        let next = if out.is_empty() { w.to_string() } else { format!("{out}, {w}") };
        if next.chars().count() > 400 {
            break;
        }
        out = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_recognised_after_filler_in_any_case() {
        for t in [
            "open Outlook", "Pick up where I left off", "resume the local companion project", "Where was I on dale cuts?",
            "draft an email to Maria", "Email the HOA board", "send it", "Reply to Sarah", "delete the last sentence",
            "Go to the next paragraph", "Hey Glim, open Outlook", "um, send the invoice", "Okay. Delete that.", "OPEN",
        ] {
            assert!(starts_like_command(t), "{t}");
        }
        for t in [
            "Thanks for the quick turnaround on unit 4B", "The plumber comes Tuesday", "Opened the gate at nine",
            "Sending thanks to everyone", "Emailed Maria yesterday", "Gone to lunch", "Undelete is not a word", "",
        ] {
            assert!(!starts_like_command(t), "{t}");
        }
    }

    #[test]
    fn decide_follows_the_approved_rules() {
        let field = true;
        // Cleaned up: typed as Claude wrote it.
        assert_eq!(
            decide("thanks for the quick turnaround", Ok(Cleanup::Dictate("Thanks for the quick turnaround.".into())), field),
            Decision::Type { text: "Thanks for the quick turnaround.".into(), cleaned: true }
        );
        // A command Glim can't carry out: heard, nothing typed.
        assert_eq!(decide("open outlook", Ok(Cleanup::Command("open_app")), field), Decision::Heard { transcript: "open outlook".into() });
        assert_eq!(decide("send it", Ok(Cleanup::Clarify("Click Send on the draft to send it.".into())), field), Decision::Question { question: "Click Send on the draft to send it.".into() });
        // Clean-up failed: dictation types the raw transcript (marker on Glim only) …
        for f in [CleanupFailed::Timeout, CleanupFailed::NoKey, CleanupFailed::Cap, CleanupFailed::Outage("network".into())] {
            assert_eq!(
                decide(" thanks for the quick turnaround ", Err(f.clone()), field),
                Decision::Type { text: "thanks for the quick turnaround".into(), cleaned: false },
                "{f:?}"
            );
            // … a command never: the card, nothing typed …
            assert_eq!(decide("open outlook", Err(f.clone()), field), Decision::Heard { transcript: "open outlook".into() });
            // … and nothing without an editable field at the press.
            assert_eq!(decide("thanks for the update", Err(f), false), Decision::Heard { transcript: "thanks for the update".into() });
        }
        // No field at the press: even a cleaned dictation isn't typed.
        assert_eq!(decide("thanks", Ok(Cleanup::Dictate("Thanks.".into())), false), Decision::Heard { transcript: "thanks".into() });
    }

    #[test]
    fn card_and_id_numbers_are_never_sent_or_typed() {
        assert_eq!(decide("my card is 4111 1111 1111 1111", Err(CleanupFailed::Timeout), true), Decision::Sensitive);
        assert_eq!(decide("the number is", Ok(Cleanup::Dictate("The number is 123-45-6789.".into())), true), Decision::Sensitive);
    }

    #[test]
    fn the_wav_is_16_khz_mono_pcm_in_memory() {
        let pcm: Vec<i16> = (0..1600).map(|i| (i % 100) as i16).collect();
        let wav = wav_bytes(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(wav.len(), 44 + 3200);
        let (ctype, body) = multipart(&[("response_format", "text"), ("prompt", "Glim, HOA")], "file", "audio.wav", &wav);
        assert!(ctype.starts_with("multipart/form-data; boundary="));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"prompt\"\r\n\r\nGlim, HOA\r\n"));
        assert!(text.contains("filename=\"audio.wav\""));
        assert!(text.trim_end().ends_with("--"));
    }

    #[test]
    fn level_and_prompt() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0; 160]), 0.0);
        assert_eq!(level(&[32000; 160]), 1.0);
        assert_eq!(prompt_from(" Glim, HOA ,\nOkafor;; dale-cuts "), "Glim, HOA, Okafor, dale-cuts");
        assert!(prompt_from(&"word, ".repeat(200)).chars().count() <= 400);
    }

    /// Transcripts never reach the log: nothing in voice/ logs a variable
    /// that could hold one.
    #[test]
    fn the_voice_code_never_logs_text() {
        for (name, src) in [("mod.rs", include_str!("mod.rs")), ("whisper.rs", include_str!("whisper.rs"))] {
            let code = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
            for banned in ["println!", "eprintln!", "dbg!", "std::fs::write", "File::create"] {
                assert!(!code.contains(banned), "{name}: {banned}");
            }
        }
    }
}
