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
pub mod draft;
#[cfg(windows)]
pub mod draft_worker;
#[cfg(all(test, windows))]
mod live;

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

/// The `open_app` apps Glim opens: classic Outlook, and the Codex and Claude
/// desktop apps (owner, 2026-10-16: "the apps", found on the PC the way
/// Outlook is; platform::open_known_app).
pub const OPENABLE: [&str; 3] = ["outlook", "codex", "claude_code"];

/// An `open_app` app's name for the card.
pub fn app_label(app: &str) -> &'static str {
    match app {
        "outlook" => "Outlook",
        "codex" => "Codex",
        "claude_code" => "Claude Code",
        _ => "that app",
    }
}

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

/// Which key started the utterance (Decision A, 2026-10-16), or that it
/// answers a waiting question card (Decision B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Ctrl+Alt+Shift+V: always text to type, never a command. Claude is
    /// offered only the `dictate` tool.
    Dictate,
    /// Ctrl+Alt+Shift+A: always a request to Glim, never typed. Claude is
    /// offered every tool but `dictate`.
    Ask,
    /// A question card was waiting: this utterance (from either key) or the
    /// card's typed answer answers it. Never typed, like Ask.
    Answer,
}

impl Mode {
    /// The island's name for it ("dictate" / "ask" / "answer").
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Dictate => "dictate",
            Mode::Ask => "ask",
            Mode::Answer => "answer",
        }
    }
}

/// The clean-up step's answer (ai::dictate), already validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cleanup {
    /// `dictate`: the cleaned text to type.
    Dictate(String),
    /// `clarify`: a question for the user.
    Clarify(String),
    /// `resume_work` (1.5c, decided 2026-10-15): the "Pick up where you left
    /// off" card, with this repo first when Claude named one.
    Resume(Option<String>),
    /// `open_app` (1.5c): one of the schema's enum, "outlook", "codex" or
    /// "claude_code" (anything else doesn't validate).
    Open(&'static str),
    /// `draft_email` (1.5d, decided 2026-10-16): names as said, subject, body.
    Draft(draft::Spoken),
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
    /// Open the resume card (it only reads; opening Claude Code in a repo
    /// still waits for a click on its row, decision 10).
    Resume { repo: Option<String> },
    /// Open an app Glim knows how to open (only classic Outlook so far): low
    /// risk, no confirmation (plan §5).
    Open { app: &'static str },
    /// The "Draft email" card: it waits for Create draft (plan §5); nothing
    /// is ever sent.
    Draft(draft::Spoken),
}

/// Everything §4.1 decides, after the transcript and the clean-up.
/// `field_at_press`: an editable, non-password field had the focus when the
/// key went down (the capture thread's last reading).
///
/// Dictate types (cleaned, or the raw transcript when there's no clean-up and
/// it doesn't start like a command: the command words now apply only there).
/// Ask and Answer never type: anything Glim can't act on is "heard".
pub fn decide(mode: Mode, transcript: &str, cleanup: Result<Cleanup, CleanupFailed>, field_at_press: bool) -> Decision {
    if crate::capture::sensitive::contains_sensitive_number(transcript) {
        return Decision::Sensitive;
    }
    match mode {
        Mode::Dictate => decide_dictation(transcript, cleanup, field_at_press),
        Mode::Ask | Mode::Answer => decide_request(transcript, cleanup),
    }
}

/// The dictation key: only `dictate` is valid (anything else is treated as no
/// clean-up, so the words are still typed, never acted on).
fn decide_dictation(transcript: &str, cleanup: Result<Cleanup, CleanupFailed>, field_at_press: bool) -> Decision {
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
        // §4.1: no clean-up types the raw transcript; one that starts like a
        // command is still held back (the command words' only use now).
        _ if field_at_press && !starts_like_command(transcript) => Decision::Type { text: transcript.trim().to_string(), cleaned: false },
        _ => Decision::Heard { transcript: transcript.to_string() },
    }
}

/// The Ask Glim key, or an answer to a question card: never typed.
fn decide_request(transcript: &str, cleanup: Result<Cleanup, CleanupFailed>) -> Decision {
    match cleanup {
        Ok(Cleanup::Clarify(question)) => Decision::Question { question },
        Ok(Cleanup::Resume(repo)) => Decision::Resume { repo },
        Ok(Cleanup::Open(app)) if OPENABLE.contains(&app) => Decision::Open { app },
        // Claude's draft is scanned like any text before it's shown or saved.
        Ok(Cleanup::Draft(d)) => {
            let all = [d.subject.as_str(), d.body.as_str()].into_iter().chain(d.to.iter().chain(&d.cc).chain(&d.bcc).map(String::as_str));
            if all.into_iter().any(crate::capture::sensitive::contains_sensitive_number) {
                Decision::Sensitive
            } else {
                Decision::Draft(d)
            }
        }
        // `dictate` isn't offered to Ask; an app Glim can't open; no clean-up.
        Ok(Cleanup::Dictate(_)) | Ok(Cleanup::Open(_)) | Err(_) => Decision::Heard { transcript: transcript.to_string() },
    }
}

/// Why a transcript was heard but not acted on, for the card: the clean-up's
/// failure code when there was no clean-up; else "command" (Ask: an action
/// not built yet), "no-field" (dictation with no field to type in) or
/// "command-words" (dictation without a clean-up that starts like a command).
pub fn heard_code(mode: Mode, cleanup_failure: &str, field_at_press: bool) -> String {
    match mode {
        Mode::Dictate if !field_at_press => "no-field".into(),
        Mode::Dictate => "command-words".into(),
        _ if !cleanup_failure.is_empty() => cleanup_failure.to_string(),
        _ => "command".into(),
    }
}

/// The repo Claude named, as one of the repos Glim reads (`known`, folder
/// names): letters and digits compared, any case ("dale cuts" is "dale-cuts").
/// None when it names none of them: the card then shows them all.
pub fn match_repo(said: &str, known: &[String]) -> Option<String> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let want = norm(said);
    if want.is_empty() {
        return None;
    }
    known.iter().find(|k| norm(k) == want).cloned()
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
    /// Which key (Decision A) or "answer" (a question card was waiting):
    /// "dictate", "ask", "answer"; the pill's label while listening.
    pub mode: &'static str,
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
pub(crate) mod tests {
    use super::*;

    #[test]
    fn resume_work_opens_the_card_with_or_without_a_field() {
        for field in [true, false] {
            for mode in [Mode::Ask, Mode::Answer] {
                assert_eq!(decide(mode, "pick up from yesterday", Ok(Cleanup::Resume(None)), field), Decision::Resume { repo: None });
                assert_eq!(decide(mode, "where was I on dale cuts", Ok(Cleanup::Resume(Some("dale-cuts".into()))), field), Decision::Resume { repo: Some("dale-cuts".into()) });
            }
        }
        // A card number still stops everything.
        assert_eq!(decide(Mode::Ask, "resume 4111 1111 1111 1111", Ok(Cleanup::Resume(None)), true), Decision::Sensitive);
        let known = vec!["dale-cuts".to_string(), "local-companion".to_string(), "dalebooking-crm".to_string()];
        assert_eq!(match_repo("dale cuts", &known).as_deref(), Some("dale-cuts"));
        assert_eq!(match_repo("Local Companion", &known).as_deref(), Some("local-companion"));
        assert_eq!(match_repo("dale", &known), None, "no partial matches");
        assert_eq!(match_repo("../secret", &known), None);
        assert_eq!(match_repo("", &known), None);
    }

    #[test]
    fn open_app_opens_three_known_apps_by_fixed_names_only() {
        assert_eq!(OPENABLE, ["outlook", "codex", "claude_code"]);
        // Anything else returns before Windows is asked to start anything.
        #[cfg(windows)]
        for app in ["notepad", r"C:\Windows\System32\cmd.exe", ""] {
            assert!(!crate::platform::open_known_app(app), "{app}");
        }
        let src = include_str!("../platform/windows.rs");
        let f = &src[src.find("pub fn open_known_app(").unwrap()..];
        let f = &f[..f.find("\n}").unwrap()];
        assert!(f.contains(r#""outlook" => shell_open(w!("OUTLOOK.EXE")),"#));
        assert!(f.contains(r#""codex" => activate_package("OpenAI.Codex", "App"),"#));
        assert!(f.contains(r#""claude_code" => activate_package("Claude", "Claude"),"#));
        assert!(f.contains("_ => false,"));
        assert!(!f.contains("Command::new"), "no shell, no child Glim spawns itself");
    }

    #[test]
    fn a_heard_card_says_why() {
        assert_eq!(heard_code(Mode::Ask, "cap", true), "cap");
        assert_eq!(heard_code(Mode::Answer, "timeout", false), "timeout");
        assert_eq!(heard_code(Mode::Ask, "", true), "command");
        assert_eq!(heard_code(Mode::Ask, "", false), "command");
        assert_eq!(heard_code(Mode::Dictate, "timeout", false), "no-field");
        assert_eq!(heard_code(Mode::Dictate, "timeout", true), "command-words");
        assert_eq!(Mode::Dictate.as_str(), "dictate");
        assert_eq!(Mode::Ask.as_str(), "ask");
        assert_eq!(Mode::Answer.as_str(), "answer");
    }

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

    /// The two sentences the router hijacked in Anthony's 2026-10-16 test,
    /// dictated with V: typed as cleaned, never a card, whatever else.
    pub(crate) const HIJACKED: [&str; 2] = [
        "This is a test of Glim dictation in my own voice",
        "Please send the updated lease to the tenant before Friday afternoon",
    ];

    #[test]
    fn dictation_always_types_and_never_acts() {
        for said in HIJACKED {
            let cleaned = format!("{said}.");
            assert_eq!(decide(Mode::Dictate, said, Ok(Cleanup::Dictate(cleaned.clone())), true), Decision::Type { text: cleaned, cleaned: true }, "{said}");
            // Whatever comes back that isn't `dictate` (it is never offered):
            // still typed, raw, never a question or an action.
            let spoken = draft::Spoken { to: vec!["the tenant".into()], body: said.into(), ..Default::default() };
            for other in [Cleanup::Clarify("What would you like me to do?".into()), Cleanup::Draft(spoken), Cleanup::Open("outlook"), Cleanup::Resume(None)] {
                assert_eq!(decide(Mode::Dictate, said, Ok(other.clone()), true), Decision::Type { text: said.into(), cleaned: false }, "{said} / {other:?}");
            }
            for f in [CleanupFailed::Timeout, CleanupFailed::NoKey, CleanupFailed::Cap, CleanupFailed::Outage("network".into())] {
                assert_eq!(decide(Mode::Dictate, said, Err(f), true), Decision::Type { text: said.into(), cleaned: false });
            }
        }
        // No clean-up: words that start like a command still aren't typed (the
        // command list's only use now) …
        assert_eq!(decide(Mode::Dictate, "open outlook", Err(CleanupFailed::Timeout), true), Decision::Heard { transcript: "open outlook".into() });
        // … but with a clean-up, they are: V means "type this".
        assert_eq!(decide(Mode::Dictate, "open outlook", Ok(Cleanup::Dictate("Open Outlook.".into())), true), Decision::Type { text: "Open Outlook.".into(), cleaned: true });
        // No field at the press: nothing typed.
        assert_eq!(decide(Mode::Dictate, "thanks", Ok(Cleanup::Dictate("Thanks.".into())), false), Decision::Heard { transcript: "thanks".into() });
        assert_eq!(decide(Mode::Dictate, "thanks", Err(CleanupFailed::Timeout), false), Decision::Heard { transcript: "thanks".into() });
    }

    #[test]
    fn ask_glim_never_types() {
        for mode in [Mode::Ask, Mode::Answer] {
            for field in [true, false] {
                assert_eq!(decide(mode, "open outlook", Ok(Cleanup::Open("outlook")), field), Decision::Open { app: "outlook" });
                assert_eq!(decide(mode, "open codex", Ok(Cleanup::Open("codex")), field), Decision::Open { app: "codex" });
                assert_eq!(decide(mode, "open claude code", Ok(Cleanup::Open("claude_code")), field), Decision::Open { app: "claude_code" });
                assert_eq!(decide(mode, "send it", Ok(Cleanup::Clarify("Click Send on the draft to send it.".into())), field), Decision::Question { question: "Click Send on the draft to send it.".into() });
                let d = draft::Spoken { to: vec!["Maria".into()], subject: "Plumber".into(), body: "The plumber comes Tuesday.".into(), ..Default::default() };
                assert_eq!(decide(mode, "draft an email to maria", Ok(Cleanup::Draft(d.clone())), field), Decision::Draft(d.clone()));
                let card = draft::Spoken { body: "Card 4111 1111 1111 1111".into(), ..d };
                assert_eq!(decide(mode, "draft an email to maria", Ok(Cleanup::Draft(card)), field), Decision::Sensitive, "Claude's body is scanned too");
                // Even a `dictate` (never offered to Ask) or no clean-up: heard, not typed.
                assert_eq!(decide(mode, HIJACKED[1], Ok(Cleanup::Dictate("x".into())), field), Decision::Heard { transcript: HIJACKED[1].into() });
                for f in [CleanupFailed::Timeout, CleanupFailed::NoKey, CleanupFailed::Cap, CleanupFailed::Outage("network".into())] {
                    assert_eq!(decide(mode, "thanks for the update", Err(f), field), Decision::Heard { transcript: "thanks for the update".into() });
                }
            }
        }
    }

    #[test]
    fn card_and_id_numbers_are_never_sent_or_typed() {
        for mode in [Mode::Dictate, Mode::Ask, Mode::Answer] {
            assert_eq!(decide(mode, "my card is 4111 1111 1111 1111", Err(CleanupFailed::Timeout), true), Decision::Sensitive);
        }
        assert_eq!(decide(Mode::Dictate, "the number is", Ok(Cleanup::Dictate("The number is 123-45-6789.".into())), true), Decision::Sensitive);
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
