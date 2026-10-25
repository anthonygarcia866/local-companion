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

fn send(app: &AppHandle, mut e: VoiceEvent) {
    // The correction tip (Glim learns §3) rides on the next result card.
    if TIP_RESULTS.contains(&e.phase) && TIP.swap(false, Ordering::SeqCst) {
        e.tip = true;
    }
    let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, &e);
}

/// When the last Ask request was: a correction is the next one within a
/// minute (memory::is_correction).
static LAST_ASK: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
const CORRECTION_WITHIN: Duration = Duration::from_secs(60);
/// A correction's tip is waiting for its result card: once per correction.
static TIP: AtomicBool = AtomicBool::new(false);
/// The result cards that can carry the tip line.
const TIP_RESULTS: [&str; 4] = ["heard", "question", "failed", "remembered"];

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
    Config { size: super::whisper::model_size(&s.voice_model), prompt: super::whisper_prompt(&s.voice_words), binding }
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
    // The draft card asking who an email goes to takes the answer too (4c).
    let draft = asked.is_none() && super::draft_worker::waiting();
    let mode = if asked.is_some() || draft { Mode::Answer } else { key };
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
            process(&app, pcm, held, cfg, mode, asked, draft).await;
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

async fn process(app: &AppHandle, pcm: Vec<i16>, held: Duration, cfg: Config, mode: Mode, asked: Option<Asked>, draft: bool) {
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
    // "unit seven D" → "unit 7D" (fix d), before anything reads it.
    let transcript = super::unit_numbers(&transcript);
    let words = transcript.split_whitespace().count();
    crate::log::line(format!("voice: held {:.1}s, transcribed ({}) {:.1}s, {words} words, {m}", held.as_secs_f64(), cfg.size, t0.elapsed().as_secs_f64()));
    if words == 0 {
        send(app, VoiceEvent { phase: "failed", code: "nothing-heard".into(), mode: m, ..Default::default() });
        return;
    }
    if draft {
        // Straight to the card: matched against its chips or the address book,
        // on this PC; nothing goes to Claude.
        if crate::capture::sensitive::contains_sensitive_number(&transcript) {
            crate::log::line("voice: not used (sensitive)");
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), mode: m, ..Default::default() });
            return;
        }
        if super::dismisses(&transcript) {
            dismiss(app, mode);
            return;
        }
        crate::log::line("voice: answer for the draft card");
        super::draft_worker::answer(app, transcript);
        send(app, VoiceEvent { phase: "idle", mode: m, ..Default::default() });
        return;
    }
    act(app, &transcript, mode, asked).await;
}

/// After the words are known (spoken or typed into the card): the clean-up,
/// voice::decide, and what follows.
async fn act(app: &AppHandle, transcript: &str, mode: Mode, asked: Option<Asked>) {
    let m = mode.as_str();
    // "Never mind" (fix c): the card closes, nothing goes to Claude, nothing
    // is asked back. Dictation still types those words.
    if mode != Mode::Dictate && super::dismisses(transcript) {
        dismiss(app, mode);
        return;
    }
    // Glim learns §3: "no, …", "I meant …" right after an Ask: the result
    // says "Tip: say 'Remember: …'". Never on the dictation key.
    if mode == Mode::Ask {
        let mut last = LAST_ASK.lock().unwrap();
        let recent = last.is_some_and(|t| t.elapsed() < CORRECTION_WITHIN);
        TIP.store(recent && crate::memory::is_correction(transcript), Ordering::SeqCst);
        *last = Some(Instant::now());
    } else {
        TIP.store(false, Ordering::SeqCst);
    }
    let cleanup = clean_up(app, mode, transcript, asked.as_ref()).await;
    // "Open claw": Claude asked which app; a word that sounds like exactly
    // one app opens it (fix d). Only after Claude answered (§4.1).
    let cleanup = match cleanup {
        Ok(Cleanup::Clarify(q)) if mode != Mode::Dictate => match super::open_sound_alike(transcript) {
            Some(app_name) => {
                crate::log::line("voice: open_app by a sound-alike");
                Ok(Cleanup::Open(app_name))
            }
            None => Ok(Cleanup::Clarify(q)),
        },
        other => other,
    };
    let why = cleanup.as_ref().err().map(CleanupFailed::code).unwrap_or_default();
    let field = super::FIELD_AT_PRESS.load(Ordering::SeqCst);
    let decision = super::decide(mode, transcript, cleanup, field);
    // An app opening shows no card: the tip gets one of its own.
    let tip_card = matches!(decision, Decision::Open { .. } | Decision::OpenOwn { .. });
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
        Decision::OpenOwn { name } => {
            // The path from Settings as they are now (the app may have been
            // removed since), checked again in open_own_app. The log never
            // names the app or its path.
            let opened = super::own_apps::find(&own_apps(app), &name).is_some_and(|a| crate::platform::open_own_app(&a.path));
            crate::log::line(format!("voice: open_app own app {}", if opened { "ok" } else { "failed" }));
            let e = if opened {
                VoiceEvent { phase: "idle", mode: m, ..Default::default() }
            } else {
                VoiceEvent { phase: "failed", code: "open-failed".into(), app: name, mode: m, ..Default::default() }
            };
            send(app, e);
        }
        Decision::Draft(spoken) => {
            crate::log::line(format!("voice: draft_email ({} names)", spoken.to.len() + spoken.cc.len() + spoken.bcc.len()));
            send(app, VoiceEvent { phase: "idle", ..Default::default() });
            super::draft_worker::begin(app, spoken);
        }
        Decision::EditDraft { change } => {
            crate::log::line("voice: edit_draft");
            edit_last_draft(app, mode, &change).await;
        }
        Decision::Remember { kind: which, text } => {
            // Glim's own filter decides (memory.rs); the card shows what was
            // kept, or why not. The log names the kind and the code only.
            let added = crate::memory::with(|mem| {
                let r = mem.add(which, &text);
                let changed = r.is_ok();
                (r, changed)
            });
            let kind = which.as_str();
            match added {
                Ok(saved) => {
                    crate::log::line(format!("memory: saved a {kind}"));
                    send(app, VoiceEvent { phase: "remembered", text: saved, code: kind.into(), mode: m, ..Default::default() });
                    let _ = app.emit("memory-changed", ());
                }
                Err(code) => {
                    crate::log::line(format!("memory: refused ({code})"));
                    send(app, VoiceEvent { phase: "failed", code: format!("memory-{code}"), mode: m, ..Default::default() });
                }
            }
        }
        Decision::Sensitive => {
            crate::log::line("voice: not sent (sensitive)");
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), mode: m, ..Default::default() });
        }
    }
    // A correction whose result had no card (an app opened): the tip gets a
    // card of its own. Either way it is said once.
    if TIP.swap(false, Ordering::SeqCst) && tip_card {
        send(app, VoiceEvent { phase: "tip", mode: m, ..Default::default() });
    }
}

/// A spoken "never mind": the waiting question ends, the voice card and the
/// draft card close (Cancel), and the voice keys go back to dictating and
/// asking.
fn dismiss(app: &AppHandle, mode: Mode) {
    crate::log::line("voice: dismissed (never mind)");
    set_question(None);
    super::draft_worker::dismiss(app);
    send(app, VoiceEvent { phase: "idle", mode: mode.as_str(), ..Default::default() });
}

/// The repos Glim reads (resume::repos), by folder name: the clean-up's
/// "Known repos" and what a named repo is matched against.
fn known_repos() -> Vec<String> {
    crate::resume::repos(&[]).iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
}

/// The user's own apps from Settings → Writing → Apps Glim can open.
fn own_apps(app: &AppHandle) -> Vec<super::own_apps::OwnApp> {
    app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().voice_apps.clone()).unwrap_or_default()
}

/// "Change the email you just drafted" (fix b, 2026-10-17): the draft saved
/// last is read back from Outlook, Claude applies the change, and the card
/// shows it; Update draft on the card applies it. The log has codes only.
async fn edit_last_draft(app: &AppHandle, mode: Mode, change: &str) {
    use crate::ai::{claude, dictate, spend};
    let current = match super::draft_worker::read_last(app).await {
        Ok(c) => c,
        Err(e) => {
            send(app, VoiceEvent { phase: "idle", mode: mode.as_str(), ..Default::default() });
            super::draft_worker::note(app, e.code());
            return;
        }
    };
    let (config, api_key) = match api_ready() {
        Ok(k) => k,
        Err(f) => {
            crate::log::line(format!("voice: edit_draft not sent ({})", f.code()));
            send(app, VoiceEvent { phase: "failed", code: f.code(), mode: mode.as_str(), ..Default::default() });
            return;
        }
    };
    send(app, VoiceEvent { phase: "cleaning", mode: mode.as_str(), ..Default::default() });
    let started = Instant::now();
    let (result, usage) = match crate::net::Url::parse(claude::API_BASE) {
        Ok(base) => dictate::revise(&base, &api_key, config.model, &current, change).await,
        Err(_) => (Err(CleanupFailed::Outage("network".into())), None),
    };
    drop(current);
    if let Some(u) = &usage {
        if let Err(err) = spend::record(&spend::path(), &spend::month_now(), u) {
            crate::log::line(format!("voice: could not record the spend: {err}"));
        }
    }
    let outcome = result.as_ref().err().map(CleanupFailed::code).unwrap_or_else(|| "ok".into());
    crate::log::line(format!("voice: edit_draft {outcome} {:.1}s", started.elapsed().as_secs_f64()));
    send(app, VoiceEvent { phase: "idle", mode: mode.as_str(), ..Default::default() });
    match result {
        // Claude's text is scanned like any before it's shown or saved.
        Ok(new) if [new.subject.as_str(), new.body.as_str()].into_iter().any(crate::capture::sensitive::contains_sensitive_number) => {
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), mode: mode.as_str(), ..Default::default() });
        }
        Ok(new) => super::draft_worker::show_edit(app, new),
        Err(CleanupFailed::Outage(code)) if code == "sensitive" => {
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), mode: mode.as_str(), ..Default::default() });
        }
        Err(_) => super::draft_worker::note(app, "edit-failed"),
    }
}

/// 1c's checks before anything goes to Claude (key, cap; Brain = Local isn't
/// built, so no clean-up): the model to use and the key.
fn api_ready() -> Result<(crate::ai::rewrite::Config, crate::ai::key::ApiKey), CleanupFailed> {
    use crate::ai::{key, rewrite, spend};
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
    Ok((config, api_key))
}

/// The Claude clean-up, with 1c's checks first (api_ready), its spend recorded.
async fn clean_up(app: &AppHandle, mode: Mode, transcript: &str, asked: Option<&Asked>) -> Result<Cleanup, CleanupFailed> {
    use crate::ai::{claude, dictate, spend};
    let (config, api_key) = api_ready()?;
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
    // Glim learns §3: every fact and the rules relevant to this request,
    // Ask lane only (never dictation). The log has the counts.
    let remembered = match mode {
        Mode::Dictate => String::new(),
        _ => {
            let about = asked.map(|a| format!("{} {transcript}", a.request)).unwrap_or_else(|| transcript.to_string());
            crate::memory::with(|mem| (mem.for_request(&about), false))
        }
    };
    let (rules, facts) = crate::memory::Memory::counts(&remembered);
    if rules + facts > 0 {
        crate::log::line(format!("memory: sent {rules} rules, {facts} facts"));
    }
    let (result, usage) = match crate::net::Url::parse(claude::API_BASE) {
        Ok(base) => {
            let repos = known_repos();
            let own = super::own_apps::names(&own_apps(app));
            let cx = dictate::Context { mode, app: &app_exe, today: &today, repos: &repos, own_apps: &own, asked, remembered: &remembered };
            dictate::clean_up(&base, &api_key, config.model, transcript, &cx).await
        }
        Err(_) => (Err(CleanupFailed::Outage("network".into())), None),
    };
    let outcome = match &result {
        Ok(Cleanup::Dictate(_)) => "dictate".to_string(),
        Ok(Cleanup::Clarify(_)) => "clarify".to_string(),
        Ok(Cleanup::Open(_)) | Ok(Cleanup::OpenOwn(_)) => "open_app".to_string(),
        Ok(Cleanup::Draft(_)) => "draft_email".to_string(),
        Ok(Cleanup::Resume(_)) => "resume_work".to_string(),
        Ok(Cleanup::EditDraft(_)) => "edit_draft".to_string(),
        Ok(Cleanup::Remember { .. }) => "remember".to_string(),
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
