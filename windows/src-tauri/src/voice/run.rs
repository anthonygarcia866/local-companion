// Push-to-talk, end to end (Windows). The key's press arrives through the
// shortcut table ("pushToTalk", Ctrl+Alt+Shift+V by default); a global
// hotkey can't see "held", so the key's own state is polled (30 ms) until it
// is released (plan §1). Then: whisper on 127.0.0.1, the Claude clean-up,
// voice::decide, and the capture thread types (it owns the field).
//
// The island hears every phase on voice::EVENT: "listening" (s-hear, the
// level ~10 times a second), "transcribing" (s-think), "cleaning" (data
// leaving the machine), then the result. The log gets phases, durations,
// counts and codes, never the transcript.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use super::{Cleanup, CleanupFailed, Decision, VoiceEvent, EVENT, MIN_HELD_MS};
use crate::capture::uia::{request, Request};

/// One utterance at a time: the key repeating while held is ignored.
static ACTIVE: AtomicBool = AtomicBool::new(false);

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

fn config(app: &AppHandle) -> Config {
    let s = app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().clone()).unwrap_or_default();
    let binding = crate::shortcuts::find("pushToTalk").map(|d| crate::shortcuts::effective(d, &s.shortcuts).keys).unwrap_or_default();
    Config { size: super::whisper::model_size(&s.voice_model), prompt: super::prompt_from(&s.voice_words), binding }
}

/// The push-to-talk key went down.
pub fn on_key_down(app: &AppHandle) {
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let cfg = config(app);
    let vk = main_key_vk(&cfg.binding);
    // The capture thread notes whether an editable field has the focus now.
    request(Request::VoiceBegin);
    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        send(&app, VoiceEvent { phase: "listening", ..Default::default() });
        let mut last_level = Instant::now();
        let recorded = super::mic::record(
            || !key_down(vk),
            |level| {
                if last_level.elapsed() >= Duration::from_millis(90) {
                    last_level = Instant::now();
                    send(&app, VoiceEvent { phase: "listening", level, ..Default::default() });
                }
            },
        );
        let held = started.elapsed();
        let pcm = match recorded {
            Err(e) => {
                crate::log::line(format!("voice: no recording ({})", e.code()));
                send(&app, VoiceEvent { phase: "failed", code: e.code().into(), ..Default::default() });
                ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
            Ok(p) => p,
        };
        if held < Duration::from_millis(MIN_HELD_MS) {
            // An accidental press: nothing is sent.
            crate::log::line(format!("voice: held {:.1}s, too short: ignored", held.as_secs_f64()));
            send(&app, VoiceEvent { phase: "idle", ..Default::default() });
            ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
        tauri::async_runtime::spawn(async move {
            process(&app, pcm, held, cfg).await;
            ACTIVE.store(false, Ordering::SeqCst);
        });
    });
}

async fn process(app: &AppHandle, pcm: Vec<i16>, held: Duration, cfg: Config) {
    send(app, VoiceEvent { phase: "transcribing", ..Default::default() });
    let t0 = Instant::now();
    let transcript = super::whisper::transcribe(&pcm, cfg.size, &cfg.prompt).await;
    drop(pcm); // the audio is gone once transcribed
    let transcript = match transcript {
        Ok(t) => t,
        Err(code) => {
            // Speech-to-text unavailable: the card, nothing typed (§4.1).
            crate::log::line(format!("voice: held {:.1}s, no transcript ({code})", held.as_secs_f64()));
            send(app, VoiceEvent { phase: "failed", code: code.into(), ..Default::default() });
            return;
        }
    };
    let words = transcript.split_whitespace().count();
    crate::log::line(format!("voice: held {:.1}s, transcribed ({}) {:.1}s, {words} words", held.as_secs_f64(), cfg.size, t0.elapsed().as_secs_f64()));
    if words == 0 {
        send(app, VoiceEvent { phase: "failed", code: "nothing-heard".into(), ..Default::default() });
        return;
    }
    let cleanup = clean_up(app, &transcript).await;
    let why = cleanup.as_ref().err().map(CleanupFailed::code).unwrap_or_default();
    let field = super::FIELD_AT_PRESS.load(Ordering::SeqCst);
    match super::decide(&transcript, cleanup, field) {
        Decision::Type { text, cleaned } => {
            request(Request::VoiceType { text, cleaned, why });
        }
        Decision::Heard { transcript } => {
            let code = super::heard_code(&why, field);
            crate::log::line(format!("voice: heard, not acted on ({code})"));
            send(app, VoiceEvent { phase: "heard", text: transcript, code, ..Default::default() });
        }
        Decision::Question { question } => {
            crate::log::line("voice: clarify");
            send(app, VoiceEvent { phase: "question", text: question, ..Default::default() });
        }
        Decision::Sensitive => {
            crate::log::line("voice: not sent (sensitive)");
            send(app, VoiceEvent { phase: "failed", code: "sensitive".into(), ..Default::default() });
        }
    }
}

/// The Claude clean-up, with 1c's checks first (key, cap; Brain = Local isn't
/// built, so no clean-up), its spend recorded.
async fn clean_up(app: &AppHandle, transcript: &str) -> Result<Cleanup, CleanupFailed> {
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
    send(app, VoiceEvent { phase: "cleaning", ..Default::default() });
    let started = Instant::now();
    let app_exe = crate::writing::tones::current_app().unwrap_or_else(|| "unknown".into());
    let today = crate::writing::ask::today();
    let (result, usage) = match crate::net::Url::parse(claude::API_BASE) {
        Ok(base) => dictate::clean_up(&base, &api_key, config.model, transcript, &app_exe, &today).await,
        Err(_) => (Err(CleanupFailed::Outage("network".into())), None),
    };
    let outcome = match &result {
        Ok(Cleanup::Dictate(_)) => "dictate".to_string(),
        Ok(Cleanup::Clarify(_)) => "clarify".to_string(),
        Ok(Cleanup::Command(c)) => (*c).to_string(),
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
