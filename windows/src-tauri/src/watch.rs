// "Tell me when …" (owner, 2026-10-28): Ask Glim's typed action
// `watch { target: claude_code | codex | window, app?, phrase?, mode?: shows | gone }`.
// ZERO AI while waiting: nothing goes to Claude after the request that set
// the watch up.
//
// - claude_code: the next Claude Code turn that ends (the relay's Stop hook
//   event, pipe.rs), after the watch started.
// - codex: Codex's own session logs (CODEX_HOME or ~/.codex/sessions,
//   rollout-*.jsonl), read only, the bytes written since the watch started,
//   looked at for a completion marker only (`task_complete`). Nothing in them
//   is kept or logged.
// - window: the named app's window (open_app's apps, the same refusals as the
//   window action: never a password manager's, an admin app's or Glim's own)
//   read with Windows' own OCR every 5 s; the phrase showing (or gone) on
//   2 checks in a row fires it. The OCR text lives in memory for one check,
//   never logged, never sent.
//
// At most 3 watches at once, each for at most 3 hours; "stop watching" ends
// them all. When one fires: the island's card, and a phone ping through
// phone.rs (Settings → Phone on, and its away / quiet-hours rules). Nothing
// else happens: no follow-up actions in v1. The log has the target kind and
// what happened, never the phrase, the app or anything read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MAX_WATCHES: usize = 3;
pub const MAX_AGE: Duration = Duration::from_secs(3 * 60 * 60);
pub const EVERY: Duration = Duration::from_secs(5);
/// A window's phrase must show (or be gone) this many checks in a row.
pub const HITS_NEEDED: u8 = 2;
/// The longest phrase watched for.
pub const MAX_PHRASE: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    ClaudeCode,
    Codex,
    /// `exe`: the program file whose window is read; `label`: its name for
    /// the card. `gone`: fire when the phrase is no longer there.
    Window { exe: String, label: String, phrase: String, gone: bool },
}

impl Target {
    /// For the log and the phone: the kind only, never the app or phrase.
    pub fn kind(&self) -> &'static str {
        match self {
            Target::ClaudeCode => "claude_code",
            Target::Codex => "codex",
            Target::Window { .. } => "window",
        }
    }
}

/// What the island shows when a watch ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Fired,
    Expired,
}

#[derive(Clone, Debug)]
pub struct Watch {
    pub id: u64,
    pub target: Target,
    pub started: Instant,
    hits: u8,
    /// codex: each log file's length when the watch started (only what is
    /// written after counts).
    seen: Vec<(PathBuf, u64)>,
}

/// What a check reads. The app's is real (Codex's logs, OCR); the tests'
/// is a fake.
pub trait Sources {
    /// The OCR'd text of `exe`'s window (None: no window, refused, or OCR
    /// failed). In memory only.
    fn window_text(&mut self, exe: &str) -> Option<String>;
    /// Codex's session log files and their lengths now.
    fn codex_logs(&mut self) -> Vec<(PathBuf, u64)>;
    /// The bytes of `path` from `from` (at most a few MB).
    fn read_from(&mut self, path: &Path, from: u64) -> Vec<u8>;
}

/// Whether `text` holds `phrase`: case and spacing ignored (OCR breaks lines).
pub fn shows(text: &str, phrase: &str) -> bool {
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let p = flat(phrase);
    !p.is_empty() && flat(text).contains(&p)
}

/// Codex wrote a turn's end: its completion marker in the new bytes.
pub fn codex_completed(bytes: &[u8]) -> bool {
    bytes.windows(b"task_complete".len()).any(|w| w == b"task_complete")
}

#[derive(Default)]
pub struct Watches {
    items: Vec<Watch>,
    next: u64,
}

impl Watches {
    /// A new watch, or why not ("watch-too-many", "watch-no-phrase").
    pub fn start<S: Sources>(&mut self, target: Target, now: Instant, src: &mut S) -> Result<u64, &'static str> {
        if self.items.len() >= MAX_WATCHES {
            return Err("watch-too-many");
        }
        if let Target::Window { phrase, .. } = &target {
            if phrase.trim().is_empty() || phrase.chars().count() > MAX_PHRASE {
                return Err("watch-no-phrase");
            }
        }
        let seen = if target == Target::Codex { src.codex_logs() } else { Vec::new() };
        self.next += 1;
        self.items.push(Watch { id: self.next, target, started: now, hits: 0, seen });
        Ok(self.next)
    }

    /// "Stop watching": all of them; how many there were.
    pub fn stop_all(&mut self) -> usize {
        let n = self.items.len();
        self.items.clear();
        n
    }

    pub fn active(&self) -> usize {
        self.items.len()
    }

    /// A Claude Code turn ended (the relay's Stop): every claude_code watch
    /// fires.
    pub fn claude_code_stopped(&mut self) -> Vec<(Watch, Ended)> {
        let (fired, keep): (Vec<Watch>, Vec<Watch>) = self.items.drain(..).partition(|w| w.target == Target::ClaudeCode);
        self.items = keep;
        fired.into_iter().map(|w| (w, Ended::Fired)).collect()
    }

    /// One check (every EVERY): Codex's logs, windows' OCR, the 3-hour limit.
    pub fn tick<S: Sources>(&mut self, now: Instant, src: &mut S) -> Vec<(Watch, Ended)> {
        let mut out = Vec::new();
        let mut keep = Vec::new();
        let mut logs: Option<Vec<(PathBuf, u64)>> = None;
        for mut w in self.items.drain(..) {
            if now.duration_since(w.started) >= MAX_AGE {
                out.push((w, Ended::Expired));
                continue;
            }
            let fired = match &w.target {
                Target::ClaudeCode => false,
                Target::Codex => {
                    let now_logs = logs.get_or_insert_with(|| src.codex_logs()).clone();
                    let mut done = false;
                    for (path, len) in &now_logs {
                        let from = w.seen.iter().find(|(p, _)| p == path).map_or(0, |(_, l)| *l);
                        if *len > from && codex_completed(&src.read_from(path, from)) {
                            done = true;
                        }
                    }
                    w.seen = now_logs;
                    done
                }
                Target::Window { exe, phrase, gone, .. } => {
                    let hit = match src.window_text(exe) {
                        Some(text) => shows(&text, phrase) != *gone,
                        None => false,
                    };
                    w.hits = if hit { w.hits + 1 } else { 0 };
                    w.hits >= HITS_NEEDED
                }
            };
            if fired {
                out.push((w, Ended::Fired));
            } else {
                keep.push(w);
            }
        }
        self.items = keep;
        out
    }
}

// ── Codex's session logs (read only) ──────────────────────────────────────────

/// Where Codex keeps its sessions: CODEX_HOME\sessions, else ~\.codex\sessions.
pub fn codex_sessions_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home).join("sessions"));
    }
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(|h| PathBuf::from(h).join(".codex").join("sessions"))
}

/// The rollout logs changed in the last 2 days (Codex writes
/// sessions\YYYY\MM\DD\rollout-*.jsonl), with their lengths. Names and
/// lengths only; nothing is opened here.
pub fn codex_logs_in(dir: &Path) -> Vec<(PathBuf, u64)> {
    fn walk(dir: &Path, depth: u8, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() && depth < 3 {
                walk(&p, depth + 1, out);
            } else if meta.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                && meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(2 * 24 * 3600))
            {
                out.push((p, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

/// At most this much of a log is read per check.
const MAX_READ: u64 = 4 * 1024 * 1024;

pub fn read_tail(path: &Path, from: u64) -> Vec<u8> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = from.max(len.saturating_sub(MAX_READ));
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(MAX_READ).read_to_end(&mut buf);
    buf
}

// ── The app's side ───────────────────────────────────────────────────────────

/// The real sources: Codex's logs on this PC, Windows' OCR of a window.
#[cfg(windows)]
struct OnThisPc;

#[cfg(windows)]
impl Sources for OnThisPc {
    fn window_text(&mut self, exe: &str) -> Option<String> {
        crate::platform::ocr_window(exe)
    }
    fn codex_logs(&mut self) -> Vec<(PathBuf, u64)> {
        codex_sessions_dir().map(|d| codex_logs_in(&d)).unwrap_or_default()
    }
    fn read_from(&mut self, path: &Path, from: u64) -> Vec<u8> {
        read_tail(path, from)
    }
}

#[cfg(windows)]
static STATE: std::sync::Mutex<Option<Watches>> = std::sync::Mutex::new(None);
#[cfg(windows)]
static LOOP_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A new watch from Ask Glim; the check loop runs while any watch does.
#[cfg(windows)]
pub fn start(app: &tauri::AppHandle, target: Target) -> Result<(), &'static str> {
    let kind = target.kind();
    let started = STATE.lock().unwrap().get_or_insert_with(Watches::default).start(target, Instant::now(), &mut OnThisPc).map(drop);
    crate::log::line(format!("watch: {} ({kind})", if started.is_ok() { "started" } else { "not started" }));
    if started.is_ok() && !LOOP_ON.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(EVERY);
            let (ended, left) = {
                let mut st = STATE.lock().unwrap();
                let w = st.get_or_insert_with(Watches::default);
                (w.tick(Instant::now(), &mut OnThisPc), w.active())
            };
            finished(&app, ended);
            if left == 0 {
                LOOP_ON.store(false, std::sync::atomic::Ordering::SeqCst);
                break;
            }
        });
    }
    started
}

/// "Stop watching": how many ended.
#[cfg(windows)]
pub fn stop_all() -> usize {
    let n = STATE.lock().unwrap().as_mut().map_or(0, Watches::stop_all);
    crate::log::line(format!("watch: stopped {n}"));
    n
}

/// A hook event from the relay (pipe.rs): a Claude Code turn ended.
#[cfg(windows)]
pub fn on_hook(app: &tauri::AppHandle, event: &str, payload: &serde_json::Value) {
    let agent = payload.get("coucou_agent").and_then(serde_json::Value::as_str);
    if event != "Stop" || !matches!(agent, None | Some("claude-desktop")) {
        return;
    }
    let ended = STATE.lock().unwrap().as_mut().map(Watches::claude_code_stopped).unwrap_or_default();
    finished(app, ended);
}

/// The island's card, and a phone ping for one that fired.
#[cfg(windows)]
fn finished(app: &tauri::AppHandle, ended: Vec<(Watch, Ended)>) {
    use tauri::{Emitter, Manager};
    for (w, how) in ended {
        let kind = w.target.kind();
        crate::log::line(format!("watch: {} ({kind})", if how == Ended::Fired { "fired" } else { "ended after 3 hours" }));
        let (app_label, phrase, gone) = match &w.target {
            Target::Window { label, phrase, gone, .. } => (label.clone(), phrase.clone(), *gone),
            _ => (String::new(), String::new(), false),
        };
        let e = crate::voice::VoiceEvent {
            phase: if how == Ended::Fired { "watched" } else { "watch-expired" },
            code: if gone { format!("{kind}-gone") } else { kind.to_string() },
            app: app_label,
            text: phrase,
            mode: "ask",
            ..Default::default()
        };
        let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::voice::EVENT, &e);
        if how == Ended::Fired {
            let settings = app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().clone());
            if let Some(settings) = settings {
                let ok = crate::phone::away_and_awake(&settings.phone, crate::phone::idle_ms(), crate::platform::local_time());
                if ok {
                    let pill = format!("watch_{kind}");
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::phone::notify(&settings, crate::phone::Kind::Watch, &pill, "").await;
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        windows: HashMap<String, Vec<Option<String>>>,
        logs: Vec<(PathBuf, u64)>,
        content: HashMap<PathBuf, Vec<u8>>,
    }

    impl Sources for Fake {
        fn window_text(&mut self, exe: &str) -> Option<String> {
            let q = self.windows.get_mut(exe)?;
            if q.is_empty() { None } else { q.remove(0) }
        }
        fn codex_logs(&mut self) -> Vec<(PathBuf, u64)> {
            self.logs.clone()
        }
        fn read_from(&mut self, path: &Path, from: u64) -> Vec<u8> {
            self.content.get(path).map(|b| b[from as usize..].to_vec()).unwrap_or_default()
        }
    }

    fn window(phrase: &str, gone: bool) -> Target {
        Target::Window { exe: "OUTLOOK.EXE".into(), label: "Outlook".into(), phrase: phrase.into(), gone }
    }

    #[test]
    fn claude_code_fires_on_its_next_stop_only() {
        let (mut w, mut f, t0) = (Watches::default(), Fake::default(), Instant::now());
        w.start(Target::ClaudeCode, t0, &mut f).unwrap();
        w.start(Target::Codex, t0, &mut f).unwrap();
        assert!(w.tick(t0 + EVERY, &mut f).is_empty(), "a check doesn't fire it");
        let fired = w.claude_code_stopped();
        assert_eq!(fired.len(), 1);
        assert_eq!((fired[0].0.target.kind(), &fired[0].1), ("claude_code", &Ended::Fired));
        assert_eq!(w.active(), 1, "the Codex watch stays");
        assert!(w.claude_code_stopped().is_empty(), "fired once");
    }

    #[test]
    fn codex_fires_on_a_completion_written_after_the_watch_started() {
        let log = PathBuf::from(r"C:\x\sessions\2026\10\28\rollout-a.jsonl");
        let old = b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n".to_vec();
        let mut f = Fake { logs: vec![(log.clone(), old.len() as u64)], ..Default::default() };
        f.content.insert(log.clone(), old.clone());
        let (mut w, t0) = (Watches::default(), Instant::now());
        w.start(Target::Codex, t0, &mut f).unwrap();
        // An old completion (before the watch) doesn't count.
        assert!(w.tick(t0 + EVERY, &mut f).is_empty());
        // Codex is working: new lines, no marker.
        let mut more = old.clone();
        more.extend_from_slice(b"{\"type\":\"response_item\",\"payload\":{\"text\":\"secret\"}}\n");
        f.logs = vec![(log.clone(), more.len() as u64)];
        f.content.insert(log.clone(), more.clone());
        assert!(w.tick(t0 + 2 * EVERY, &mut f).is_empty());
        // Its turn ends.
        more.extend_from_slice(b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n");
        f.logs = vec![(log.clone(), more.len() as u64)];
        f.content.insert(log.clone(), more);
        let fired = w.tick(t0 + 3 * EVERY, &mut f);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].1, Ended::Fired);
        // A new session file started after the watch counts from its start.
        let mut w = Watches::default();
        let mut f = Fake::default();
        w.start(Target::Codex, t0, &mut f).unwrap();
        let new = PathBuf::from(r"C:\x\sessions\2026\10\28\rollout-b.jsonl");
        let bytes = b"{\"payload\":{\"type\":\"task_complete\"}}\n".to_vec();
        f.logs = vec![(new.clone(), bytes.len() as u64)];
        f.content.insert(new, bytes);
        assert_eq!(w.tick(t0 + EVERY, &mut f).len(), 1);
    }

    #[test]
    fn a_window_fires_after_two_checks_in_a_row() {
        let t0 = Instant::now();
        let mut f = Fake::default();
        f.windows.insert("OUTLOOK.EXE".into(), vec![Some("Inbox".into()), Some("Build PASSED today".into()), Some("Inbox".into()), Some("build   passed".into()), Some("BUILD PASSED".into())]);
        let mut w = Watches::default();
        w.start(window("Build passed", false), t0, &mut f).unwrap();
        let mut t = t0;
        let mut fired_at = None;
        for i in 1..=5 {
            t += EVERY;
            if !w.tick(t, &mut f).is_empty() {
                fired_at = Some(i);
                break;
            }
        }
        assert_eq!(fired_at, Some(5), "hit, miss resets, hit, hit");
        // "gone": fires when the phrase is no longer there, twice; a missing
        // window (None) is neither.
        let mut f = Fake::default();
        f.windows.insert("OUTLOOK.EXE".into(), vec![Some("Uploading 40%".into()), None, Some("Done".into()), Some("Done".into())]);
        let mut w = Watches::default();
        w.start(window("uploading", true), t0, &mut f).unwrap();
        let n: Vec<usize> = (1..=4).map(|i| w.tick(t0 + EVERY * i, &mut f).len()).collect();
        assert_eq!(n, [0, 0, 0, 1]);
    }

    #[test]
    fn three_at_most_three_hours_at_most_and_stop_ends_them() {
        let (mut w, mut f, t0) = (Watches::default(), Fake::default(), Instant::now());
        for _ in 0..3 {
            w.start(Target::ClaudeCode, t0, &mut f).unwrap();
        }
        assert_eq!(w.start(Target::Codex, t0, &mut f), Err("watch-too-many"));
        assert_eq!(w.stop_all(), 3);
        assert_eq!(w.active(), 0);
        w.start(Target::Codex, t0, &mut f).unwrap();
        assert!(w.tick(t0 + MAX_AGE - EVERY, &mut f).is_empty());
        let ended = w.tick(t0 + MAX_AGE, &mut f);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].1, Ended::Expired);
        assert_eq!(w.start(window("  ", false), t0, &mut f), Err("watch-no-phrase"));
        assert_eq!(w.start(window(&"x".repeat(MAX_PHRASE + 1), false), t0, &mut f), Err("watch-no-phrase"));
    }

    #[test]
    fn phrases_match_whatever_the_case_and_spacing() {
        assert!(shows("The build\nPASSED  at 3pm", "build passed"));
        assert!(!shows("The build failed", "build passed"));
        assert!(!shows("anything", "   "));
        assert!(codex_completed(br#"{"payload":{"type":"task_complete"}}"#));
        assert!(!codex_completed(br#"{"payload":{"type":"agent_message"}}"#));
    }

    /// The logs Codex writes: found by name under sessions\YYYY\MM\DD, read
    /// from an offset, nothing outside that folder.
    #[test]
    fn codex_logs_are_found_and_read_from_an_offset() {
        let dir = std::env::temp_dir().join(format!("glim-watch-{}", std::process::id()));
        let day = dir.join("2026").join("10").join("28");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-x.jsonl"), b"0123456789").unwrap();
        std::fs::write(day.join("notes.txt"), b"no").unwrap();
        let logs = codex_logs_in(&dir);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].1, 10);
        assert_eq!(read_tail(&logs[0].0, 6), b"6789");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing read is logged: every log line names the kind, or a count.
    #[test]
    fn the_log_never_carries_what_was_read() {
        let src = include_str!("watch.rs").replace("\r\n", "\n");
        let code = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        let lines: Vec<&str> = code.match_indices("log::line(").map(|(i, _)| &code[i..i + code[i..].find(";\n").unwrap()]).collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        for l in lines {
            for banned in ["phrase", "label", "text", "exe", "path", "bytes"] {
                assert!(!l.contains(banned), "{banned} in {l}");
            }
        }
    }
}
