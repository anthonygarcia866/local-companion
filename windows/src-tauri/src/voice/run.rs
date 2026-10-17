// Push-to-talk, end to end (Windows). Two held keys (Decision A, 2026-10-16):
// "pushToTalk" (Ctrl+Alt+Shift+V) dictates, "askGlim" (Ctrl+Alt+Shift+A) asks
// Glim; each arrives through the shortcut table. A global hotkey can't see
// "held", so the key's own state is polled (30 ms) until it is released (plan
// §1). Then: whisper on 127.0.0.1, the Claude clean-up, voice::decide, and the
// capture thread types (it owns the field).
//
// Decision B: while Glim's question card waits (PENDING, at most
// QUESTION_TTL), the next utterance from either key answers it, and so does
// an answer typed into the card (`answer_typed`). Cancel or Close clears it.
//
// The island hears every phase on voice::EVENT: "listening" (s-hear, the
// level ~10 times a second), "transcribing" (s-think), "cleaning" (data
// leaving the machine), then the result. The log gets phases, durations,
// counts and codes, never the transcript.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use super::{Cleanup, CleanupFailed, Decision, Mode, VoiceEvent, EVENT, MIN_HELD_MS};
use crate::ai::dictate::Asked;
use crate::capture::uia::{request, Request};

/// One utterance at a time: the key repeating while held is ignored.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Glim's question card, waiting for an answer (Decision B).
struct Pending {
    asked: Asked,
    since: Instant,
}

static PENDING: std::sync::Mutex<Option<Pending>> = std::sync::Mutex::new(None);

/// A question nobody answered stops waiting: the dictation key goes back to
/// dictating even if the island missed the card's closing.
pub const QUESTION_TTL: Duration = Duration::from_secs(120);

/// The mode for a press of `key`: an answer while a fresh question waits.
fn mode_for(key: Mode, waiting: Option<Duration>) -> Mode {
    match waiting {
        Some(age) if age < QUESTION_TTL => Mode::Answer,
        _ => key,
    }
}

/// What a press answers, if a question is waiting (an old one is dropped).
fn waiting_question() -> Option<Asked> {
    let mut p = PENDING.lock().unwrap();
    match p.as_ref().map(|q| q.since.elapsed()) {
        Some(age) if mode_for(Mode::Dictate, Some(age)) == Mode::Answer => p.as_ref().map(|q| q.asked.clone()),
        Some(_) => {
            *p = None;
            None
        }
        None => None,
    }
}

fn set_question(asked: Option<Asked>) {
    *PENDING.lock().unwrap() = asked.map(|asked| Pending { asked, since: Instant::now() });
}

/// Cancel / Close on the question card (or its timeout): nothing waits now.
pub fn cancel_question() {
    if PENDING.lock().unwrap().take().is_some() {
        crate::log::line("voice: question closed");
    }
}

/// The island's state for "nothing came of it": the question card again
/// while answering one, else nothing.
fn quiet(asked: &Option<Asked>, mode: Mode) -> VoiceEvent {
    match asked {
        Some(a) => VoiceEvent { phase: "question", text: a.question.clone(), mode: mode.as_str(), ..Default::default() },
        None => VoiceEvent { phase: "idle", mode: mode.as_str(), ..Default::default() },
    }
}

fn send(app: &AppHandle, e: VoiceEvent) {
    let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, &e);
}

/// The virtual-key code of a binding's main key ("Ctrl+Alt+Shift+V" → V).
pub fn main_key_vk(binding: &str) -> u16 {
    let key = binding.rsplit('+').next().unwrap_or("").trim();
    let up = key.to_ascii_uppercase();
    if up.len() == 1 {
        let c = up.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return c as u16;
        }
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
        return 0x6F + n;
    }
    match up.as_str() {
        "SPACE" => 0x20,
        _ => b'V' as u16,
    }
}

fn key_down(vk: u16) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

/// Settings → Dictation, read at each press.
struct Config {
    size: &'static str,
    prompt: String,
    binding: String,
}

/// The shortcut that starts each key's utterances.
pub fn action_of(key: Mode) -> &'static str {
    match key {
        Mode::Ask => "askGlim",
        _ => "pushToTalk",
    }
}

fn config(app: &AppHandle, key: Mode) -> Config {
    let s = app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().clone()).unwrap_or_default();
    let binding = crate::shortcuts::find(action_of(key)).map(|d| crate::shortcuts::effective(d, &s.shortcuts).keys).unwrap_or_default();
    Config { size: super::whisper::model_size(&s.voice_model), prompt: super::prompt_from(&s.voice_words), binding }
}

/// A voice key went down: `key` is Mode::Dictate (pushToTalk) or Mode::Ask
/// (askGlim). A waiting question card turns either into its answer.
pub fn on_key_down(app: &AppHandle, key: Mode) {
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let cfg = config(app, key);
    let vk = main_key_vk(&cfg.binding);
    let asked = waiting_question();
    let mode = if asked.is_some() { Mode::Answer } else { key };
    let m = mode.as_str();
    // The capture thread notes whether an editable field has the focus now.
    request(Request::VoiceBegin);
    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        send(&app, VoiceEvent { phase: "listening", mode: m, ..Default::default() });
        let mut last_level = Instant::now();
        let recorded = super::mic::record(
            || !key_down(vk),
            |level| {
                if last_level.elapsed() >= Duration::from_millis(90) {
                    last_level = Instant::now();
                    send(&app, VoiceEvent { phase: "listening", level, mode: m, ..Default::default() });
                }
            },
        );
        let held = started.elapsed();
        let pcm = match recorded {
            Err(e) => {
                crate::log::line(format!("voice: no recording ({})", e.code()));
                send(&app, VoiceEvent { phase: "failed", code: e.code().into(), mode: m, ..Default::default() });
                ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
            Ok(p) => p,
        };
        if held < Duration::from_millis(MIN_HELD_MS) {
            // An accidental press: nothing is sent (a waiting question stays).
            crate::log::line(format!("voice: held {:.1}s, too short: ignored", held.as_secs_f64()));
            send(&app, quiet(&asked, mode));
            ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
        tauri::async_runtime::spawn(async move {
            process(&app, pcm, held, cfg, mode, asked).await;
            ACTIVE.store(false, Ordering::SeqCst);
        });
    });
}

/// An answer typed into the question card (Decision B). False when no
/// question is waiting (it closed or timed out) or the text is empty.
pub fn answer_typed(app: &AppHandle, text: String) -> bool {
    let text = text.trim().chars().take(1000).collect::<String>();
    let Some(asked) = waiting_question() else { return false };
    if text.is_empty() || ACTIVE.swap(true, Ordering::SeqCst) {
        return false;
    }
    crate::log::line(format!("voice: typed answer, {} words", text.split_whitespace().count()));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        act(&app, &text, Mode::Answer, Some(asked)).await;
        ACTIVE.store(false, Ordering::SeqCst);
    });
    true
}

async fn process(app: &AppHandle, pcm: Vec<i16>, held: Duration, cfg: Config, mode: Mode, asked: Option<Asked>) {
    let m = mode.as_str();
    send(app, VoiceEvent { phase: "transcribing", mode: m, ..Default::default() });
    let t0 = Instant::now();
    let transcript = super::whisper::transcribe(&pcm, cfg.size, &cfg.prompt).await;
    drop(pcm); // the audio is gone once transcribed
    let transcript = match transcript {
        Ok(t) => t,
        Err(code) => {
            // Speech-to-text unavailable: the card, nothing typed (§4.1).
            crate::log::line(format!("voice: held {:.1}s, no transcript ({code})", held.as_secs_f64()));
            send(app, VoiceEvent { phase: "failed", code: code.into(), mode: m, ..Default::default() });
            return;
        }
    };
    let words = transcript.split_whitespace().count();
    crate::log::line(format!("voice: held {:.1}s, transcribed ({}) {:.1}s, {words} words, {m}", held.as_secs_f64(), cfg.size, t0.elapsed().as_secs_f64()));
    if words == 0 {
        send(app, VoiceEvent { phase: "failed", code: "nothing-heard".into(), mode: m, ..Default::default() });
        return;
    }
    act(app, &transcript, mode, asked).await;
}

/// After the words are known (spoken or typed into the card): the clean-up,
/// voice::decide, and what follows.
async fn act(app: &AppHandle, transcript: &str, mode: Mode, asked: Option<Asked>) {
    let m = mode.as_str();
    let cleanup = clean_up(app, mode, transcript, asked.as_ref()).await;
    let why = cleanup.as_ref().err().map(CleanupFailed::code).unwrap_or_default();
    let field = super::FIELD_AT_PRESS.load(Ordering::SeqCst);
    let decision = super::decide(mode, transcript, cleanup, field);
    // A new question waits (its request: the earlier one, then this answer);
    // anything else ends the question.
    set_question(match &decision {
        Decision::Question { question } => Some(Asked {
            request: match &asked {
                Some(a) => format!("{}\n{}", a.request, transcript),
                None => transcript.to_string(),
            },
            question: question.clone(),
        }),
        _ => None,
    });
    match decision {
        Decision::Type { text, cleaned } => {
            request(Request::VoiceType { text, cleaned, why });
        }
        Decision::Heard { transcript } => {
            let code = super::heard_code(mode, &why, field);
            crate::log::line(format!("voice: heard, not acted on ({code})"));
            send(app, VoiceEvent { phase: "heard", text: transcript, code, mode: m, ..Default::default() });
        }
        Decision::Question { question } => {
            crate::log::line("voice: clarify (a question waits)");
            send(app, VoiceEvent { phase: "question", text: question, mode: m, ..Default::default() });
        }
        Decision::Resume { repo } => {
            // Only a repo Glim reads goes to the island; the log never names it.
            let repo = repo.and_then(|r| super::match_repo(&r, &known_repos()));
            crate::log::line(format!("voice: resume_work ({})", if repo.is_some() { "a repo first" } else { "all repos" }));
            send(app, VoiceEvent { phase: "idle", mode: m, ..Default::default() });
            let _ = app.emit_to(crate::island::WINDOW_LABEL, "resume-show", serde_json::json!({ "repo": repo }));
        }
        Decision::Open { app: which } => {
            // Low risk, no confirmation (plan §5): the app opening says it worked.
            let opened = crate::platform::open_known_app(which);
            crate::log::line(format!("voice: open_app {which} {}", if opened { "ok" } else { "failed" }));
            let e = if opened {
                VoiceEvent { phase: "idle", mode: m, ..Default::default() }
            } else {
                VoiceEvent { phase: "failed", code: "open-failed".into(), app: super::app_label(which).into(), mode: m, ..Default::default() }
            };
            send(app, e);
        }
        Decision::Sensitive => {
            crate::log::line("voice: not sent (sensitive)");
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), mode: m, ..Default::default() });
        }
    }
}

/// The repos Glim reads (resume::repos), by folder name: the clean-up's
/// "Known repos" and what a named repo is matched against.
fn known_repos() -> Vec<String> {
    crate::resume::repos(&[]).iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
}

/// The Claude clean-up, with 1c's checks first (key, cap; Brain = Local isn't
/// built, so no clean-up), its spend recorded.
async fn clean_up(app: &AppHandle, mode: Mode, transcript: &str, asked: Option<&Asked>) -> Result<Cleanup, CleanupFailed> {
    use crate::ai::{claude, dictate, key, rewrite, spend};
    let config = rewrite::config();
    let saved = key::load(&key::vault());
    let has_key = saved.as_ref().map(Option::is_some).map_err(Clone::clone);
    let spent = spend::current(&spend::path(), &spend::month_now());
    // Pause doesn't stop the clean-up (an action may still run); typing checks it.
    if let Err(code) = rewrite::precheck(false, &config, &has_key, &spent) {
        return Err(match code {
            "no-key" => CleanupFailed::NoKey,
            "cap" => CleanupFailed::Cap,
            other => CleanupFailed::Outage(other.into()),
        });
    }
    let Ok(Some(api_key)) = saved else { return Err(CleanupFailed::NoKey) };
    send(app, VoiceEvent { phase: "cleaning", mode: mode.as_str(), ..Default::default() });
    let started = Instant::now();
    // The app and its kind (plan §4: "the focused app's exe and field kind"):
    // "OUTLOOK.EXE (email)". Never the field's text.
    let app_exe = format!(
        "{} ({})",
        crate::writing::tones::current_app().unwrap_or_else(|| "unknown".into()),
        crate::writing::tones::current_kind()
    );
    let today = crate::writing::ask::today();
    let (result, usage) = match crate::net::Url::parse(claude::API_BASE) {
        Ok(base) => {
            let repos = known_repos();
            let cx = dictate::Context { mode, app: &app_exe, today: &today, repos: &repos, asked };
            dictate::clean_up(&base, &api_key, config.model, transcript, &cx).await
        }
        Err(_) => (Err(CleanupFailed::Outage("network".into())), None),
    };
    let outcome = match &result {
        Ok(Cleanup::Dictate(_)) => "dictate".to_string(),
        Ok(Cleanup::Clarify(_)) => "clarify".to_string(),
        Ok(Cleanup::Command(c)) => (*c).to_string(),
        Ok(Cleanup::Open(_)) => "open_app".to_string(),
        Ok(Cleanup::Resume(_)) => "resume_work".to_string(),
        Err(f) => f.code(),
    };
    match &usage {
        Some(u) => {
            crate::log::line(format!("voice: cleanup {outcome} {} in={} out={} ${:.6} {:.1}s", u.model, u.input_tokens, u.output_tokens, u.cost_nanos as f64 / 1e9, started.elapsed().as_secs_f64()));
            if let Err(err) = spend::record(&spend::path(), &spend::month_now(), u) {
                crate::log::line(format!("voice: could not record the spend: {err}"));
            }
        }
        None => crate::log::line(format!("voice: cleanup {outcome} {:.1}s", started.elapsed().as_secs_f64())),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_has_its_shortcut_and_a_waiting_question_turns_either_into_an_answer() {
        assert_eq!(action_of(Mode::Dictate), "pushToTalk");
        assert_eq!(action_of(Mode::Ask), "askGlim");
        for key in [Mode::Dictate, Mode::Ask] {
            assert_eq!(mode_for(key, None), key);
            assert_eq!(mode_for(key, Some(Duration::from_secs(5))), Mode::Answer);
            // An old question no longer catches the dictation key.
            assert_eq!(mode_for(key, Some(QUESTION_TTL)), key);
        }
    }

    #[test]
    fn a_question_waits_until_answered_cancelled_or_too_old() {
        // One test owns PENDING (tests run in threads).
        set_question(Some(Asked { request: "open the app".into(), question: "Which app?".into() }));
        assert_eq!(waiting_question().map(|a| a.question), Some("Which app?".into()));
        cancel_question();
        assert_eq!(waiting_question(), None);
        set_question(Some(Asked { request: "r".into(), question: "q".into() }));
        PENDING.lock().unwrap().as_mut().unwrap().since = Instant::now() - QUESTION_TTL;
        assert_eq!(waiting_question(), None, "too old: dropped");
        assert!(PENDING.lock().unwrap().is_none());
        // A too-short press while answering puts the question back, not nothing.
        let asked = Some(Asked { request: "r".into(), question: "Which app?".into() });
        let e = quiet(&asked, Mode::Answer);
        assert_eq!((e.phase, e.text.as_str(), e.mode), ("question", "Which app?", "answer"));
        assert_eq!(quiet(&None, Mode::Dictate).phase, "idle");
    }

    #[test]
    fn the_held_key_is_the_bindings_main_key() {
        assert_eq!(main_key_vk("Ctrl+Alt+Shift+V"), b'V' as u16);
        assert_eq!(main_key_vk("Ctrl+Alt+Shift+d"), b'D' as u16);
        assert_eq!(main_key_vk("Ctrl+Alt+7"), b'7' as u16);
        assert_eq!(main_key_vk("Ctrl+Shift+F9"), 0x78);
        assert_eq!(main_key_vk("Ctrl+Alt+Space"), 0x20);
        assert_eq!(main_key_vk(""), b'V' as u16);
    }

    /// Transcripts never reach the log or the dev panel: every log line here
    /// is built from codes, durations, counts and the model's name.
    #[test]
    fn the_run_code_never_logs_the_transcript() {
        let src = include_str!("run.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for line in code.lines().filter(|l| l.contains("log::line")) {
            for var in ["transcript", "text", "question", "pcm"] {
                assert!(!line.contains(&format!("{{{var}}}")) && !line.contains(&format!(", {var})")), "{line}");
            }
        }
    }
}
