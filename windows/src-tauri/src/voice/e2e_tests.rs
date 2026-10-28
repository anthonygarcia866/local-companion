// Voice without a microphone (automated testing stage 4, 2026-10-28): synthetic
// speech clips -> whisper -> the Claude clean-up -> the action router, with a
// FAKE executor that records every action and opens nothing.
//
//   cargo test --lib voice::e2e_tests -- --ignored --nocapture
//
// - GLIM_VOICE_CLIPS: a folder of 16 kHz mono WAV clips, made by
//   scripts/voice-clips.ps1 (Windows' SAPI voices; none are kept in the repo).
// - GLIM_WHISPER_DIR: whisper.cpp b5454's whisper-server with its DLLs and
//   ggml-base.en-q5_1.bin, as scripts/fetch-whisper.ps1 downloads them (each
//   checked against its SHA-256). Copied into the test's own scratch Glim
//   folder; Glim's own code starts the server (loopback only) and transcribes.
// - GLIM_VOICE_API_KEY (optional): an Anthropic key. Set: the clean-up is LIVE,
//   through Glim's own request code, until the run has spent COST_CAP; then,
//   and without a key, each request gets a canned answer in the Messages API's
//   response shape (written by hand from the tool schema, not a recording).
//   Either way the body Glim would send is built and scanned exactly as for a
//   real request: a card number never leaves.
//
// The router is voice::run::act, the code the voice keys run, through the
// Effects seam (voice/run.rs). Learned shortcuts, routines and memory are the
// real modules, in the test's scratch folder. Printed: the transcripts of the
// synthetic clips, the recorded actions, counts and the cost. Never a key.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::run::{self, Effects};
use super::{Cleanup, CleanupFailed, Mode, VoiceEvent};
use crate::ai::dictate::Asked;
use crate::ai::{claude, dictate, key};

/// What a live run may spend (nano-dollars): about one cent.
const COST_CAP_NANOS: u64 = 10_000_000;
/// "Words Glim should know" for this run (Settings → Dictation): made-up names.
const VOICE_WORDS: &str = "Zorvath, Quillane";

#[derive(Default)]
struct Rec {
    /// What the router did: "open outlook", "type …", "resume …", "draft".
    actions: Vec<String>,
    events: Vec<VoiceEvent>,
    notified: Vec<String>,
    /// The app in front, as the fake Windows has it.
    front: Option<String>,
    /// The canned answer for the next request (recorded mode).
    canned: Option<Value>,
    /// Requests answered (live or canned), and those stopped before sending.
    answered: u32,
    blocked: u32,
    live: u32,
    /// Rules sent with the last request.
    last_rules: usize,
    cost_nanos: u64,
    /// Learning running in the background (awaited by `settle`).
    spawned: Vec<tokio::task::JoinHandle<()>>,
}

#[derive(Clone)]
struct Fake {
    rec: Arc<Mutex<Rec>>,
    key: Option<key::ApiKey>,
}

impl Fake {
    fn with<R>(&self, f: impl FnOnce(&mut Rec) -> R) -> R {
        f(&mut self.rec.lock().unwrap())
    }
    fn act(&self, what: String) {
        println!("    action: {what}");
        self.with(|r| r.actions.push(what));
    }
    fn last_event(&self) -> VoiceEvent {
        self.with(|r| r.events.last().cloned().unwrap_or_default())
    }
    fn actions_since(&self, n: usize) -> Vec<String> {
        self.with(|r| r.actions[n..].to_vec())
    }

    /// Glim's clean-up, as clean_up_app builds it, with the network last.
    async fn claude(&self, mode: Mode, transcript: &str, asked: Option<&Asked>) -> Result<Cleanup, CleanupFailed> {
        let remembered = match mode {
            Mode::Dictate => String::new(),
            _ => crate::memory::with(|m| (m.for_request(transcript), false)),
        };
        let (rules, _) = crate::memory::Memory::counts(&remembered);
        let app = if mode == Mode::Dictate { "OUTLOOK.EXE (email)" } else { "unknown (unknown)" };
        let cx = dictate::Context { mode, app, today: "Wednesday, October 28, 2026", repos: &[], own_apps: &[], asked, remembered: &remembered };
        let model = claude::DEFAULT_MODEL;
        let body = dictate::request_body(model, transcript, &cx);
        self.with(|r| r.last_rules = rules);
        // claude::send's own last look: nothing leaves.
        if claude::blocked(&body) {
            self.with(|r| r.blocked += 1);
            println!("    claude: not sent (sensitive)");
            return Err(CleanupFailed::Outage("sensitive".into()));
        }
        let spent = self.with(|r| r.cost_nanos);
        let canned = self.with(|r| r.canned.take());
        let (result, usage) = match &self.key {
            Some(k) if spent < COST_CAP_NANOS => {
                let base = crate::net::Url::parse(claude::API_BASE).unwrap();
                self.with(|r| r.live += 1);
                dictate::clean_up(&base, k, model, transcript, &cx).await
            }
            _ => {
                let answer = canned.expect("a canned answer for this request");
                dictate::parse_answer(&serde_json::to_vec(&answer).unwrap(), model, mode, &[])
            }
        };
        let live = self.key.is_some() && spent < COST_CAP_NANOS;
        self.with(|r| {
            r.answered += 1;
            if live {
                r.cost_nanos += usage.as_ref().map_or(0, |u| u.cost_nanos);
            }
        });
        let tool = match &result {
            Ok(Cleanup::Dictate(_)) => "dictate".to_string(),
            Ok(Cleanup::Clarify(_)) => "clarify".to_string(),
            Ok(Cleanup::Open(a)) => format!("open_app {a}"),
            Ok(Cleanup::OpenOwn(_)) => "open_app (own)".to_string(),
            Ok(Cleanup::SaveRoutine { name, steps }) => format!("save_routine \"{name}\" ({} steps)", steps.len()),
            Ok(Cleanup::Remember { kind, .. }) => format!("remember {}", kind.as_str()),
            Ok(_) => "another tool".to_string(),
            Err(f) => format!("failed {}", f.code()),
        };
        println!("    claude ({}): {tool}, {rules} rules sent", if live { "live" } else { "canned" });
        result
    }

    /// Learning after an app came forward runs in the background: done.
    async fn settle(&self) {
        loop {
            let Some(h) = self.with(|r| r.spawned.pop()) else { break };
            h.await.unwrap();
        }
    }
}

impl Effects for Fake {
    fn emit_voice(&self, e: &VoiceEvent) {
        if !matches!(e.phase, "cleaning" | "transcribing" | "listening") {
            println!("    card: {} {} {}", e.phase, e.code, if e.steps.is_empty() { String::new() } else { format!("({} steps)", e.steps.len()) });
        }
        self.with(|r| r.events.push(e.clone()));
    }
    fn notify(&self, event: &str) {
        self.with(|r| r.notified.push(event.to_string()));
    }
    fn resume_show(&self, payload: Value) {
        self.act(format!("resume {payload}"));
    }
    fn type_text(&self, text: String, cleaned: bool, _why: String) {
        self.act(format!("type{} {text}", if cleaned { "" } else { " (raw)" }));
    }
    fn foreground_exe(&self) -> Option<String> {
        self.with(|r| r.front.clone())
    }
    fn open_known_app(&self, which: &'static str) -> bool {
        self.act(format!("open {which}"));
        // The app's window comes to the front, as Windows would show it.
        self.with(|r| r.front = Some(super::app_exe(which).unwrap_or_else(|| format!("{which}.exe"))));
        true
    }
    fn open_own_app(&self, path: &str) -> bool {
        self.act(format!("open own {path}"));
        true
    }
    fn own_apps(&self) -> Vec<super::own_apps::OwnApp> {
        Vec::new()
    }
    fn known_repos(&self) -> Vec<String> {
        Vec::new()
    }
    fn learning_on(&self) -> bool {
        true
    }
    fn dismiss_draft(&self) {
        self.act("dismiss".into());
    }
    fn draft(&self, _spoken: super::draft::Spoken) {
        self.act("draft".into());
    }
    fn edit_draft(&self, _mode: Mode, _change: &str) -> impl std::future::Future<Output = ()> + Send {
        self.act("edit draft".into());
        async {}
    }
    fn clean_up(&self, mode: Mode, transcript: &str, asked: Option<&Asked>) -> impl std::future::Future<Output = Result<Cleanup, CleanupFailed>> + Send {
        let me = self.clone();
        let transcript = transcript.to_string();
        let asked = asked.cloned();
        async move { me.claude(mode, &transcript, asked.as_ref()).await }
    }
    fn spawn(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        let h = tokio::spawn(fut);
        self.with(|r| r.spawned.push(h));
    }
}

/// A canned Messages API answer: one tool call.
fn tool(name: &str, input: Value) -> Value {
    json!({
        "id": "msg_canned", "type": "message", "role": "assistant", "model": claude::DEFAULT_MODEL,
        "content": [{ "type": "tool_use", "id": "toolu_canned", "name": name, "input": input }],
        "stop_reason": "tool_use", "usage": { "input_tokens": 1600, "output_tokens": 40 }
    })
}

fn wav_pcm(path: &Path) -> Vec<i16> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..4], b"RIFF");
    let mut i = 12;
    while i + 8 <= b.len() {
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"fmt " {
            let channels = u16::from_le_bytes([b[i + 10], b[i + 11]]);
            let rate = u32::from_le_bytes([b[i + 12], b[i + 13], b[i + 14], b[i + 15]]);
            let bits = u16::from_le_bytes([b[i + 22], b[i + 23]]);
            assert_eq!((channels, rate, bits), (1, 16_000, 16), "{}: 16 kHz mono 16-bit", path.display());
        }
        if &b[i..i + 4] == b"data" {
            return b[i + 8..(i + 8 + len).min(b.len())].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        }
        i += 8 + len + (len & 1);
    }
    panic!("no data chunk in {}", path.display());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// Lower-case words, letters and digits only.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

fn heard(t: &str, expected: &[&str]) {
    let w = words(t);
    for e in expected {
        assert!(w.iter().any(|x| x == e), "whisper heard {t:?}, expected the word {e:?}");
    }
}

struct Run {
    fx: Fake,
    clips: PathBuf,
    prompt: String,
}

impl Run {
    /// One utterance: the clip through whisper (Glim's hear()), then the
    /// router, as the voice keys run it; background learning awaited.
    async fn say(&self, clip: &str, mode: Mode, canned: Option<Value>) -> String {
        let pcm = wav_pcm(&self.clips.join(format!("{clip}.wav")));
        let t = run::hear(&pcm, "base", &self.prompt).await.unwrap_or_else(|code| panic!("whisper: {code}"));
        println!("[{clip}] heard: {t:?}");
        self.fx.with(|r| r.canned = canned);
        run::act(&self.fx, &t, mode, None, true).await;
        self.fx.settle().await;
        t
    }
}

/// Learned shortcuts, routines and memory are process-wide: one test at a
/// time uses them, each from empty.
static SERIAL: Mutex<()> = Mutex::new(());

fn fresh() -> std::sync::MutexGuard<'static, ()> {
    let one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    crate::learn::with(|l| {
        l.shortcuts.items.clear();
        l.routines.items.clear();
    });
    crate::memory::with(|m| {
        *m = crate::memory::Memory::default();
        ((), false)
    });
    one
}

fn fake() -> Fake {
    Fake { rec: Arc::new(Mutex::new(Rec { front: Some("Code.exe".into()), ..Default::default() })), key: None }
}

/// Typed words straight into the router (no whisper), as the card's text box
/// sends them; background learning awaited.
async fn type_in(fx: &Fake, text: &str, canned: Option<Value>) {
    println!("[typed] {text:?}");
    fx.with(|r| r.canned = canned);
    run::act(fx, text, Mode::Ask, None, true).await;
    fx.settle().await;
}

/// The fixes from stage 4's findings, without whisper (runs in every
/// `cargo test`): a remembered rule beats a learned shortcut, the new
/// shortcut says "Learned: say … any time", and a valid card number in a
/// "remember" is refused before anything is saved or sent.
#[test]
fn rules_beat_shortcuts_and_a_card_number_is_never_remembered() {
    let _one = fresh();
    let fx = fake();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let answered = || fx.with(|r| r.answered);
        type_in(&fx, "open Outlook", Some(tool("open_app", json!({ "app": "outlook" })))).await;
        assert_eq!(shortcut_keys(), ["open {app}"]);
        let e = fx.last_event();
        assert_eq!((e.phase, e.text.as_str(), e.app.as_str()), ("learned-new", "open Outlook", "Outlook"), "{e:?}");
        type_in(&fx, "open Outlook", None).await;
        assert_eq!((answered(), fx.last_event().phase), (1, "learned"), "replayed, no Claude call");

        type_in(&fx, "remember: open Claude means the desktop app", Some(tool("remember", json!({ "kind": "rule", "text": "open Claude means the desktop app" })))).await;
        assert_eq!(crate::memory::with(|m| (m.rules.len(), false)), 1);
        let n = fx.with(|r| r.actions.len());
        type_in(&fx, "open Claude", Some(tool("open_app", json!({ "app": "claude" })))).await;
        assert_eq!(answered(), 3, "the rule beat the \"open {{app}}\" shortcut: Claude asked");
        assert_eq!(fx.with(|r| r.last_rules), 1, "with the rule");
        assert_eq!(fx.actions_since(n), ["open claude"]);
        assert_ne!(fx.last_event().phase, "learned-new", "nothing learned from an ask a rule sent to Claude");
        // The rule is about Claude, not every "open": Outlook still replays.
        type_in(&fx, "open Outlook", None).await;
        assert_eq!((answered(), fx.last_event().phase), (3, "learned"), "a rule's verb doesn't block other apps");

        // A valid test card number (Luhn-checked) said with "remember".
        let card = "remember my card number 4111 1111 1111 1111";
        assert!(crate::capture::sensitive::contains_sensitive_number(card));
        let (sent, blocked) = fx.with(|r| (r.answered, r.blocked));
        type_in(&fx, card, Some(tool("remember", json!({ "kind": "fact", "text": "my card is 4111 1111 1111 1111" })))).await;
        assert_eq!(fx.with(|r| (r.answered, r.blocked)), (sent, blocked + 1), "stopped before sending");
        assert_eq!(crate::memory::with(|m| ((m.rules.len(), m.facts.len()), false)), (1, 0), "nothing saved");
        let e = fx.last_event();
        assert_eq!((e.phase, e.code.as_str()), ("failed", "sensitive"), "{e:?}");
        assert!(!shortcut_keys().iter().any(|k| k.contains("card")), "nothing learned from it");
    });
}

fn shortcut_keys() -> Vec<String> {
    crate::learn::with(|l| l.shortcuts.items.iter().map(|s| s.key.clone()).collect())
}

#[test]
#[ignore = "needs GLIM_VOICE_CLIPS and GLIM_WHISPER_DIR (CI's voice step)"]
fn voice_without_a_mic() {
    let clips = PathBuf::from(std::env::var_os("GLIM_VOICE_CLIPS").expect("GLIM_VOICE_CLIPS"));
    let whisper_dir = PathBuf::from(std::env::var_os("GLIM_WHISPER_DIR").expect("GLIM_WHISPER_DIR"));
    copy_dir(&whisper_dir, &super::whisper::folder());
    assert!(super::whisper::files("base").is_some(), "whisper-server.exe and the base model");
    let key = std::env::var("GLIM_VOICE_API_KEY").ok().filter(|k| !k.trim().is_empty()).map(|raw| {
        let v = key::MemoryVault::default();
        key::save(&v, &raw).expect("the key's format");
        key::load(&v).unwrap().unwrap()
    });
    println!("voice e2e: Claude {}", if key.is_some() { "LIVE (capped at $0.01)" } else { "canned answers (no GLIM_VOICE_API_KEY)" });
    let _one = fresh();
    let fx = Fake { rec: Arc::new(Mutex::new(Rec { front: Some("Code.exe".into()), ..Default::default() })), key };
    let r = Run { fx: fx.clone(), clips, prompt: super::whisper_prompt(VOICE_WORDS) };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let answered = || fx.with(|r| r.answered);

        // 1. "open Outlook": Claude's open_app, Outlook opens, the shortcut is
        // learned. An app named in the words is learned as a slot: "open {app}"
        // then opens any app said that way (learn::Shortcuts::learn).
        let n = fx.with(|r| r.actions.len());
        let t = r.say("open-outlook", Mode::Ask, Some(tool("open_app", json!({ "app": "outlook" })))).await;
        heard(&t, &["open", "outlook"]);
        assert_eq!(fx.actions_since(n), ["open outlook"]);
        assert_eq!(answered(), 1);
        println!("    learned: {:?}", shortcut_keys());
        assert_eq!(shortcut_keys(), ["open {app}"]);
        let e = fx.last_event();
        assert_eq!((e.phase, e.app.as_str()), ("learned-new", "Outlook"), "the \"Learned: say … any time\" card: {e:?}");
        assert_eq!(e.text.to_lowercase(), "open outlook", "the words to say: {e:?}");

        // 2. Again: the learned shortcut replays, Claude isn't asked.
        let n = fx.with(|r| r.actions.len());
        r.say("open-outlook", Mode::Ask, None).await;
        assert_eq!(fx.actions_since(n), ["open outlook"]);
        assert_eq!(answered(), 1, "no Claude call for a learned shortcut");
        assert_eq!(fx.last_event().phase, "learned");

        // 3. A routine: its card lists both steps; Save keeps it.
        let steps = json!([{ "tool": "open_app", "app": "outlook" }, { "tool": "open_app", "app": "claude" }]);
        let t = r.say("routine-work-mode", Mode::Ask, Some(tool("save_routine", json!({ "name": "work mode", "steps": steps })))).await;
        heard(&t, &["work", "mode", "outlook", "claude"]);
        let card = fx.last_event();
        assert_eq!(card.phase, "routine-ask", "{card:?}");
        assert_eq!(card.text, "work mode");
        assert_eq!(card.steps.iter().map(|s| (s.tool, s.name.as_str())).collect::<Vec<_>>(), [("open_app", "Outlook"), ("open_app", "Claude")]);
        assert!(run::save_proposed(&fx), "Save on the card");
        assert_eq!(fx.last_event().phase, "routine-saved");

        // 4. "work mode": both steps run, no Claude call.
        let before = answered();
        let n = fx.with(|r| r.actions.len());
        let t = r.say("work-mode", Mode::Ask, None).await;
        heard(&t, &["work", "mode"]);
        assert_eq!(fx.actions_since(n), ["open outlook", "open claude"]);
        assert_eq!(answered(), before, "a routine runs on this PC");
        let e = fx.last_event();
        assert_eq!((e.phase, e.code.as_str()), ("routine", "ok"));

        // 5. "Remember: open Claude means the desktop app": a saved rule.
        let t = r.say("remember-claude", Mode::Ask, Some(tool("remember", json!({ "kind": "rule", "text": "open Claude means the desktop app" })))).await;
        heard(&t, &["remember", "claude", "desktop"]);
        let e = fx.last_event();
        assert_eq!((e.phase, e.code.as_str()), ("remembered", "rule"), "{e:?}");
        assert_eq!(crate::memory::with(|m| (m.rules.len(), false)), 1);

        // 6. A card number to remember: refused, and nothing leaves the PC.
        let blocked = fx.with(|r| r.blocked);
        let before = answered();
        let t = r.say("remember-card", Mode::Ask, Some(tool("remember", json!({ "kind": "fact", "text": "my card is 4111 1111 1111 1111" })))).await;
        let e = fx.last_event();
        // Refused: a card that says why (canned: Glim's filter; live, Claude
        // may also ask back or decline), and nothing new remembered.
        assert!(matches!(e.phase, "failed" | "heard" | "question"), "{e:?}");
        if e.phase == "failed" {
            assert!(e.code == "sensitive" || e.code.starts_with("memory-"), "refused: {}", e.code);
        }
        assert_eq!(crate::memory::with(|m| (m.rules.len() + m.facts.len(), false)), 1, "nothing new remembered");
        if crate::capture::sensitive::contains_sensitive_number(&t) {
            // Heard as a card number: stopped before anything was sent.
            assert_eq!(fx.with(|r| r.blocked), blocked + 1);
            assert_eq!(answered(), before, "the card number was not sent");
            println!("    refused before sending ({})", e.code);
        } else {
            println!("    whisper didn't hear a card number; Glim's memory filter refused it ({})", e.code);
        }

        // 7. "open Claude": the desktop app ("claude", not "claude_code"). The
        // rule from step 5 shares its words, so it beats the "open {app}"
        // shortcut from step 1: Claude is asked, with the rule.
        let n = fx.with(|r| r.actions.len());
        let before = answered();
        let t = r.say("open-claude", Mode::Ask, Some(tool("open_app", json!({ "app": "claude" })))).await;
        heard(&t, &["open", "claude"]);
        assert_eq!(fx.actions_since(n), ["open claude"]);
        assert_eq!(answered(), before + 1, "a rule beats the learned shortcut: Claude asked");
        assert_eq!(fx.with(|r| r.last_rules), 1, "with the rule");

        // 8. #62: "open the office" (Claude: Outlook) is learned with words of
        // its own; a rule about those words removes it, and the card says so.
        let n = fx.with(|r| r.actions.len());
        let t = r.say("open-the-office", Mode::Ask, Some(tool("open_app", json!({ "app": "outlook" })))).await;
        heard(&t, &["open", "office"]);
        assert_eq!(fx.actions_since(n), ["open outlook"]);
        println!("    learned: {:?}", shortcut_keys());
        assert!(shortcut_keys().iter().any(|k| k.contains("office")), "learned: {:?}", shortcut_keys());
        let t = r.say("remember-office", Mode::Ask, Some(tool("remember", json!({ "kind": "rule", "text": "when I say the office I mean Outlook" })))).await;
        heard(&t, &["remember", "office", "outlook"]);
        let e = fx.last_event();
        assert_eq!(e.phase, "remembered", "{e:?}");
        assert!(e.app.contains("office"), "the card names the removed shortcut: {:?}", e.app);
        assert!(!shortcut_keys().iter().any(|k| k.contains("office")), "removed: {:?}", shortcut_keys());
        assert!(shortcut_keys().iter().any(|k| k == "open {app}"), "a slot shortcut has no words of its own: kept");
        assert!(fx.with(|r| r.notified.iter().any(|n| n == "memory-changed")));

        // 9. Dictation: made-up names (Settings words) and "unit four B" -> "unit 4B".
        super::FIELD_AT_PRESS.store(true, std::sync::atomic::Ordering::SeqCst);
        let n = fx.with(|r| r.actions.len());
        let t = r.say("dictation", Mode::Dictate, Some(tool("dictate", json!({ "text": "Please tell Zorvath Quillane that unit 4B is ready for the walkthrough." })))).await;
        assert!(t.to_lowercase().contains("unit 4b"), "\"unit four B\" written as a unit: {t:?}");
        // The made-up names come from the run's word list (whisper's prompt).
        let names: Vec<&str> = ["Zorvath", "Quillane"].into_iter().filter(|n| t.contains(n)).collect();
        println!("    names as written: {} of 2 ({names:?})", names.len());
        assert!(!names.is_empty(), "no made-up name as written: {t:?}");
        let typed = fx.actions_since(n);
        assert_eq!(typed.len(), 1, "{typed:?}");
        assert!(typed[0].starts_with("type ") && typed[0].to_lowercase().contains("unit 4b"), "{typed:?}");

        super::whisper::stop();
        let (answered, blocked, live, cost) = fx.with(|r| (r.answered, r.blocked, r.live, r.cost_nanos));
        let line = format!(
            "voice e2e: 10 utterances, {answered} Claude answers ({live} live, {} canned), {blocked} stopped before sending, cost ${:.6}",
            answered - live,
            cost as f64 / 1e9
        );
        println!("{line}");
        if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(summary) {
                let _ = writeln!(f, "### {line}");
            }
        }
    });
}
