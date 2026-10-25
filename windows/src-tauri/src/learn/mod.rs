// Glim learns (docs/glim-learns-plan.md §1, §2, §4; decided 2026-10-24,
// §9): learned shortcuts and routines, for the Ask lane only. The dictation
// key never learns, never replays and never runs a routine.
//
// A shortcut: after an Ask request worked (Claude's open_app or resume_work,
// and Glim saw it work), the validated call is kept under the request's
// gist. The same ask again is matched here, before any API call, and the
// call runs again through today's checks. A routine: "When I say work mode,
// open Outlook and Codex", saved on the user's Save, run when its name is
// said.
//
// Learning never makes a new power: only open_app and resume_work are ever
// kept, and a replay goes through the same validation as Claude's answer.
// Local files only (%LOCALAPPDATA%\Glim\learn\shortcuts.json and
// routines.json), each write a temp file and a rename; a file that doesn't
// parse is set aside as *.bad. The log gets tool names, codes and counts,
// never a gist, a routine's name, an app or a repo.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A learned shortcut unused this long is deleted at the next save.
pub const EXPIRY_SECS: u64 = 30 * 24 * 3600;
/// At most this many shortcuts; past that the least recently used goes.
pub const MAX_SHORTCUTS: usize = 200;
/// At most this many routines, of at most this many steps.
pub const MAX_ROUTINES: usize = 20;
pub const MAX_STEPS: usize = 5;

/// Words that never change what is asked.
const FILLER: [&str; 30] = [
    "um", "uh", "er", "hmm", "hey", "hi", "ok", "okay", "glim", "please", "can", "could", "would", "will", "you", "quickly",
    "just", "now", "so", "kindly", "go", "ahead", "let's", "let", "i", "i'd", "want", "wanna", "need", "like",
];
/// Small words a gist leaves out.
const STOP: [&str; 13] = ["a", "an", "the", "to", "my", "me", "for", "of", "on", "in", "at", "up", "and"];
/// "This", "that"…: the ask means something else every time, so it is never
/// learned or matched.
pub const DEICTIC: [&str; 8] = ["this", "that", "these", "those", "it", "here", "there", "them"];
/// The app slot in a gist: "open {app}".
pub const SLOT: &str = "{app}";

/// A few verbs mean the same thing.
fn fold(word: &str) -> &str {
    match word {
        "launch" | "start" | "run" | "load" => "open",
        "continue" | "resume" => "resume",
        w => w,
    }
}

/// The words of `text`: lower case, punctuation gone (a hyphen or an
/// apostrophe inside a word stays: "my-repo", "let's").
pub fn words(text: &str) -> Vec<String> {
    let cleaned: String = text.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '\'' || c == '\u{2019}' { c } else { ' ' }).collect();
    cleaned
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '-' || c == '\'' || c == '\u{2019}').replace('\u{2019}', "'").to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// The words a gist keeps, in the order said: filler and stop words out,
/// verbs folded.
fn kept(text: &str) -> Vec<String> {
    words(text).into_iter().filter(|w| !FILLER.contains(&w.as_str()) && !STOP.contains(&w.as_str())).map(|w| fold(&w).to_string()).collect()
}

fn key_of(mut words: Vec<String>) -> String {
    words.sort();
    words.dedup();
    words.join(" ")
}

/// The plain key of `text`: its kept words, sorted, each once. "um, can
/// you open outlook for me" and "outlook, open" are both "open outlook".
pub fn key(text: &str) -> String {
    key_of(kept(text))
}

/// Says "this", "that", "it"…
pub fn deictic(text: &str) -> bool {
    words(text).iter().any(|w| DEICTIC.contains(&w.as_str()))
}

/// An ask's gist (plan §1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gist {
    /// The kept words, sorted, joined by spaces.
    pub plain: String,
    /// When the ask names exactly one app Glim knows: the key with that app
    /// as `{app}`, and the app (an open_app value or an own app's name).
    pub slotted: Option<(String, String)>,
    /// How many different words are kept.
    pub len: usize,
}

/// The apps a gist can name: the words each is said with, and its open_app
/// value. The built-in ones, then the user's own (`own`, by name).
pub fn app_words(own: &[String]) -> Vec<(Vec<String>, String)> {
    let mut out: Vec<(Vec<String>, String)> = [("outlook", "outlook"), ("codex", "codex"), ("claude", "claude"), ("claude code", "claude_code")]
        .iter()
        .map(|(said, value)| (kept(said), value.to_string()))
        .collect();
    out.extend(own.iter().map(|name| (kept(name), name.clone())));
    out.retain(|(said, _)| !said.is_empty());
    out
}

pub fn gist(text: &str, apps: &[(Vec<String>, String)]) -> Gist {
    let kept = kept(text);
    let plain = key_of(kept.clone());
    let len = plain.split(' ').filter(|w| !w.is_empty()).count();
    // The apps named, longest name first where two start at the same word
    // ("claude code" over "claude").
    let mut found: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < kept.len() {
        match apps.iter().filter(|(said, _)| !said.is_empty() && kept[i..].starts_with(said)).max_by_key(|(said, _)| said.len()) {
            Some((said, value)) => {
                found.push((i, said.len(), value.clone()));
                i += said.len();
            }
            None => i += 1,
        }
    }
    let distinct: BTreeSet<&String> = found.iter().map(|f| &f.2).collect();
    let slotted = (distinct.len() == 1).then(|| {
        let mut out = Vec::new();
        let mut at = 0;
        for (start, n, _) in &found {
            out.extend(kept[at..*start].iter().cloned());
            out.push(SLOT.to_string());
            at = start + n;
        }
        out.extend(kept[at..].iter().cloned());
        (key_of(out), found[0].2.clone())
    });
    Gist { plain, slotted, len }
}

/// Why an ask isn't learned (plan §1, "Never learned"). The log says the
/// code only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotLearned {
    /// Settings → Ask Glim → "Learn shortcuts from what I ask" is off.
    Off,
    Deictic,
    /// Fewer than 2 words kept.
    Short,
    /// Just a command word ("open", "where was I").
    CommandWord,
    /// Glim didn't see it work, or Claude needed a sound-alike, a question,
    /// or the sensitive scan stopped it.
    Unconfirmed,
}

impl NotLearned {
    pub fn code(self) -> &'static str {
        match self {
            NotLearned::Off => "off",
            NotLearned::Deictic => "deictic",
            NotLearned::Short => "short",
            NotLearned::CommandWord => "command-word",
            NotLearned::Unconfirmed => "unconfirmed",
        }
    }
}

/// Whether the words of an ask could be learned (the tool and the check
/// after acting are the caller's).
pub fn learnable(text: &str, g: &Gist) -> Result<(), NotLearned> {
    if deictic(text) {
        return Err(NotLearned::Deictic);
    }
    if g.len < 2 {
        return Err(NotLearned::Short);
    }
    if crate::voice::COMMAND_WORDS.iter().any(|c| key(c) == g.plain) {
        return Err(NotLearned::CommandWord);
    }
    Ok(())
}

/// The check after opening an app (plan §1): the program in front now is
/// `exe` (any case), or, when Glim can't know the app's file (a shortcut, a
/// package), another app than Glim and than the one in front before.
pub fn forward_ok(front: &str, exe: Option<&str>, before: Option<&str>) -> bool {
    match exe {
        Some(exe) => front.eq_ignore_ascii_case(exe),
        None => !front.eq_ignore_ascii_case("glim.exe") && before.is_none_or(|b| !front.eq_ignore_ascii_case(b)),
    }
}

/// An own app's program file, when its path names one (an .exe); None for a
/// shortcut (.lnk).
pub fn own_exe(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_string_lossy().into_owned();
    name.to_ascii_lowercase().ends_with(".exe").then_some(name)
}

// ── Shortcuts ─────────────────────────────────────────────────────────────────

/// A call Glim may keep and run again: only these two tools (plan §1's
/// allowlist), with today's values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Action {
    /// An open_app value ("outlook") or one of the user's own apps by name.
    OpenApp { app: String },
    /// The resume card, with this repo first (a folder name Glim reads).
    ResumeWork {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
    },
}

impl Action {
    pub fn tool(&self) -> &'static str {
        match self {
            Action::OpenApp { .. } => "open_app",
            Action::ResumeWork { .. } => "resume_work",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    /// The gist (plain, or with `{app}` when `slot`).
    pub key: String,
    pub action: Action,
    /// open_app with the app as a slot: the same words about another known
    /// app open that app.
    #[serde(default)]
    pub slot: bool,
    /// Seconds since 1970, of the last use (or of learning it).
    pub last_used: u64,
    /// Replays in a row that didn't work: the second deletes it.
    #[serde(default)]
    pub fails: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub items: Vec<Shortcut>,
}

impl Shortcuts {
    /// The shortcut for this ask: the plain key first, then the app slot
    /// (open_app only, run with the app this ask names). Its index, and the
    /// call to run.
    pub fn find(&self, g: &Gist) -> Option<(usize, Action)> {
        if let Some(i) = self.items.iter().position(|s| !s.slot && s.key == g.plain) {
            return Some((i, self.items[i].action.clone()));
        }
        let (key, app) = g.slotted.as_ref()?;
        let i = self.items.iter().position(|s| s.slot && &s.key == key && matches!(s.action, Action::OpenApp { .. }))?;
        Some((i, Action::OpenApp { app: app.clone() }))
    }

    /// A replay worked: its clock starts again.
    pub fn used(&mut self, i: usize, now: u64) {
        if let Some(s) = self.items.get_mut(i) {
            s.last_used = now;
            s.fails = 0;
        }
    }

    /// A replay didn't work. True when it was the second in a row: the
    /// shortcut is gone.
    pub fn failed(&mut self, i: usize) -> bool {
        let Some(s) = self.items.get_mut(i) else { return false };
        s.fails = s.fails.saturating_add(1);
        if s.fails >= 2 {
            self.items.remove(i);
            return true;
        }
        false
    }

    /// Keeps `action` under this ask's gist, replacing what was there. An
    /// open_app of the one app the ask names is kept with the app as a slot.
    pub fn learn(&mut self, g: &Gist, action: Action, now: u64) {
        let slotted = match (&action, &g.slotted) {
            (Action::OpenApp { app }, Some((key, named))) if app == named => Some(key.clone()),
            _ => None,
        };
        let slot = slotted.is_some();
        let key = slotted.unwrap_or_else(|| g.plain.clone());
        self.items.retain(|s| !(s.key == key && s.slot == slot));
        self.items.push(Shortcut { key, action, slot, last_used: now, fails: 0 });
        while self.items.len() > MAX_SHORTCUTS {
            let oldest = self.items.iter().enumerate().min_by_key(|(_, s)| s.last_used).map(|(i, _)| i).unwrap_or(0);
            self.items.remove(oldest);
        }
    }

    /// Settings → Forget, or Try again on its card.
    pub fn remove(&mut self, key: &str, slot: bool) -> bool {
        let before = self.items.len();
        self.items.retain(|s| !(s.key == key && s.slot == slot));
        self.items.len() != before
    }

    /// Deletes the shortcuts unused for 30 days; how many went.
    pub fn prune(&mut self, now: u64) -> usize {
        let before = self.items.len();
        self.items.retain(|s| now.saturating_sub(s.last_used) <= EXPIRY_SECS);
        before - self.items.len()
    }
}

// ── Routines ──────────────────────────────────────────────────────────────────

/// "When I say work mode, …" (plan §2): the user made it, so it never
/// expires.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub name: String,
    pub steps: Vec<Action>,
    #[serde(default)]
    pub created: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routines {
    pub items: Vec<Routine>,
}

/// Why a routine's name can't be used (the card and Settings say it in
/// words). `own`: the user's own apps' names; `others`: the other routines'.
pub fn name_problem(name: &str, own: &[String], others: &[&str]) -> Option<&'static str> {
    let name = name.trim();
    let n = name.chars().count();
    if !(2..=40).contains(&n) {
        return Some("name-length");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '\'') {
        return Some("name-chars");
    }
    if deictic(name) {
        return Some("name-deictic");
    }
    let k = key(name);
    if k.is_empty() {
        return Some("name-length");
    }
    let norm = crate::voice::own_apps::norm;
    let apps = ["outlook", "codex", "claude", "claudecode", "classicoutlook", "claudeapp"];
    if apps.contains(&norm(name).as_str()) || own.iter().any(|o| norm(o) == norm(name)) {
        return Some("name-app");
    }
    if crate::voice::COMMAND_WORDS.iter().any(|c| norm(c) == norm(name) || key(c) == k) {
        return Some("name-command");
    }
    if others.iter().any(|o| key(o) == k) {
        return Some("name-taken");
    }
    None
}

/// Why these steps can't be a routine: none, or more than 5.
pub fn steps_problem(steps: &[Action]) -> Option<&'static str> {
    if steps.is_empty() {
        return Some("no-steps");
    }
    (steps.len() > MAX_STEPS).then_some("too-many-steps")
}

impl Routines {
    fn names_except(&self, skip: Option<&str>) -> Vec<&str> {
        self.items.iter().map(|r| r.name.as_str()).filter(|n| Some(*n) != skip).collect()
    }

    /// Whether `name` with `steps` could be saved now.
    pub fn problem(&self, name: &str, steps: &[Action], own: &[String]) -> Option<&'static str> {
        if self.items.len() >= MAX_ROUTINES {
            return Some("too-many-routines");
        }
        name_problem(name, own, &self.names_except(None)).or_else(|| steps_problem(steps))
    }

    pub fn add(&mut self, name: &str, steps: Vec<Action>, own: &[String], now: u64) -> Result<(), &'static str> {
        if let Some(p) = self.problem(name, &steps, own) {
            return Err(p);
        }
        self.items.push(Routine { name: name.trim().to_string(), steps, created: now });
        Ok(())
    }

    /// The routine this ask names: its words are the routine's name, or the
    /// name with "open"/"run"/"start" ("run work mode").
    pub fn find(&self, g: &Gist) -> Option<&Routine> {
        let without_open = g.plain.split(' ').filter(|w| *w != "open").collect::<Vec<_>>().join(" ");
        self.items.iter().find(|r| {
            let k = key(&r.name);
            k == g.plain || k == without_open
        })
    }

    pub fn rename(&mut self, from: &str, to: &str, own: &[String]) -> Result<(), &'static str> {
        let Some(i) = self.items.iter().position(|r| r.name == from) else { return Err("gone") };
        if let Some(p) = name_problem(to, own, &self.names_except(Some(from))) {
            return Err(p);
        }
        self.items[i].name = to.trim().to_string();
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|r| r.name != name);
        self.items.len() != before
    }
}

// ── Files ─────────────────────────────────────────────────────────────────────

/// %LOCALAPPDATA%\Glim\learn
pub fn dir() -> PathBuf {
    crate::platform::local_dir().join("learn")
}

pub const SHORTCUTS_FILE: &str = "shortcuts.json";
pub const ROUTINES_FILE: &str = "routines.json";

/// A file's contents, or empty when it is missing. One that doesn't parse
/// is set aside as `<name>.bad` (its text never logged) and counts as empty.
pub fn load<T: Default + DeserializeOwned>(path: &Path) -> T {
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return T::default();
    }
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            let bad = path.with_extension("json.bad");
            let _ = std::fs::remove_file(&bad);
            let _ = std::fs::rename(path, &bad);
            crate::log::line("learn: a file didn't parse, set aside (counts as empty)");
            T::default()
        }
    }
}

/// Writes a temp file beside `path`, then renames it over: a crash never
/// leaves half a file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    if let Err(err) = std::fs::write(&temp, bytes).and_then(|_| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What is learned, loaded from `dir` once.
pub struct Learned {
    pub shortcuts: Shortcuts,
    pub routines: Routines,
    dir: PathBuf,
}

impl Learned {
    pub fn load(dir: &Path) -> Learned {
        Learned { shortcuts: load(&dir.join(SHORTCUTS_FILE)), routines: load(&dir.join(ROUTINES_FILE)), dir: dir.to_path_buf() }
    }

    /// Saves the shortcuts, the expired ones gone first.
    pub fn save_shortcuts(&mut self) {
        let gone = self.shortcuts.prune(now());
        if gone > 0 {
            crate::log::line(format!("learn: {gone} shortcuts expired"));
        }
        if let Err(err) = save(&self.dir.join(SHORTCUTS_FILE), &self.shortcuts) {
            crate::log::line(format!("learn: could not save the shortcuts ({})", err.kind()));
        }
    }

    pub fn save_routines(&self) {
        if let Err(err) = save(&self.dir.join(ROUTINES_FILE), &self.routines) {
            crate::log::line(format!("learn: could not save the routines ({})", err.kind()));
        }
    }

    /// The startup line: counts only.
    pub fn counts(&self) -> String {
        format!("learn: {} shortcuts, {} routines", self.shortcuts.items.len(), self.routines.items.len())
    }
}

static STATE: Mutex<Option<Learned>> = Mutex::new(None);

/// Runs `f` on what is learned (loaded from disk the first time).
pub fn with<R>(f: impl FnOnce(&mut Learned) -> R) -> R {
    let mut state = STATE.lock().unwrap();
    let learned = state.get_or_insert_with(|| {
        let l = Learned::load(&dir());
        crate::log::line(l.counts());
        l
    });
    f(learned)
}

// ── What Settings and the cards show ──────────────────────────────────────────

/// One step or shortcut, for the island and Settings: the tool, and the app's
/// or repo's name as shown ("" = every repo).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    pub tool: &'static str,
    pub name: String,
}

pub fn step_view(a: &Action) -> StepView {
    match a {
        Action::OpenApp { app } => {
            let label = crate::voice::app_label(app);
            StepView { tool: "open_app", name: if crate::voice::OPENABLE.contains(&app.as_str()) { label.to_string() } else { app.clone() } }
        }
        Action::ResumeWork { repo } => StepView { tool: "resume_work", name: repo.clone().unwrap_or_default() },
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutView {
    /// The words it answers to ("open {app}"): shown in Settings only.
    pub key: String,
    pub slot: bool,
    pub step: StepView,
    /// Days until it expires unless used.
    pub days_left: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    pub name: String,
    pub steps: Vec<StepView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnView {
    pub shortcuts: Vec<ShortcutView>,
    pub routines: Vec<RoutineView>,
}

pub fn view(l: &Learned, now: u64) -> LearnView {
    let mut shortcuts: Vec<ShortcutView> = l
        .shortcuts
        .items
        .iter()
        .map(|s| ShortcutView {
            key: s.key.clone(),
            slot: s.slot,
            step: step_view(&s.action),
            days_left: EXPIRY_SECS.saturating_sub(now.saturating_sub(s.last_used)) / 86_400,
        })
        .collect();
    shortcuts.sort_by(|a, b| a.key.cmp(&b.key));
    LearnView {
        shortcuts,
        routines: l.routines.items.iter().map(|r| RoutineView { name: r.name.clone(), steps: r.steps.iter().map(step_view).collect() }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<(Vec<String>, String)> {
        app_words(&["Spotify".to_string(), "Maple Notes".to_string()])
    }

    fn open(app: &str) -> Action {
        Action::OpenApp { app: app.into() }
    }

    #[test]
    fn a_gist_drops_filler_stop_words_order_and_punctuation() {
        for (said, want) in [
            ("Um, can you open Outlook for me?", "open outlook"),
            ("outlook, open", "open outlook"),
            ("Hey Glim, please launch Outlook.", "open outlook"),
            ("start outlook quickly", "open outlook"),
            ("Where was I on my-repo?", "my-repo was where"),
            ("pick up my-repo", "my-repo pick"),
            ("Let's continue my-repo", "my-repo resume"),
            ("open, open OUTLOOK!", "open outlook"),
        ] {
            assert_eq!(gist(said, &apps()).plain, want, "{said:?}");
        }
        // Two asks that differ only in order are the same key.
        assert_eq!(key("open codex now"), key("codex open"));
        assert_eq!(gist("open outlook", &apps()).len, 2);
    }

    #[test]
    fn the_one_app_an_ask_names_becomes_a_slot() {
        let g = gist("please open Claude Code", &apps());
        assert_eq!(g.slotted, Some(("open {app}".into(), "claude_code".into())), "the longer name wins");
        assert_eq!(gist("open claude", &apps()).slotted, Some(("open {app}".into(), "claude".into())));
        assert_eq!(gist("open maple notes", &apps()).slotted, Some(("open {app}".into(), "Maple Notes".into())));
        // Two different apps: no slot. The same one twice: one slot.
        assert_eq!(gist("open outlook and codex", &apps()).slotted, None);
        assert_eq!(gist("outlook open outlook", &apps()).slotted, Some(("open {app}".into(), "outlook".into())));
        assert_eq!(gist("where was I on my-repo", &apps()).slotted, None);
    }

    #[test]
    fn a_shortcut_for_one_app_replays_for_another_known_app_only() {
        let mut s = Shortcuts::default();
        s.learn(&gist("open outlook", &apps()), open("outlook"), 100);
        assert_eq!(s.items.len(), 1);
        assert!(s.items[0].slot);
        assert_eq!(s.find(&gist("could you open codex", &apps())).map(|f| f.1), Some(open("codex")));
        assert_eq!(s.find(&gist("open spotify", &apps())).map(|f| f.1), Some(open("Spotify")));
        // An app Glim doesn't know is never invented.
        assert_eq!(s.find(&gist("open notepad", &apps())), None);
        // Other words: no match.
        assert_eq!(s.find(&gist("close outlook", &apps())), None);
    }

    #[test]
    fn resume_work_is_kept_by_its_plain_words_and_never_slotted() {
        let mut s = Shortcuts::default();
        let g = gist("where was I on my-repo", &apps());
        s.learn(&g, Action::ResumeWork { repo: Some("my-repo".into()) }, 100);
        assert!(!s.items[0].slot);
        assert_eq!(s.find(&gist("on my-repo where was I", &apps())).map(|f| f.1), Some(Action::ResumeWork { repo: Some("my-repo".into()) }));
        assert_eq!(s.find(&gist("where was I on other-repo", &apps())), None);
        // open_app of an app the ask didn't name (Claude picked it): plain key.
        let g = gist("open my mail", &apps());
        s.learn(&g, open("outlook"), 100);
        assert!(s.items.iter().any(|x| x.key == "mail open" && !x.slot));
    }

    #[test]
    fn what_is_never_learned() {
        let a = apps();
        assert_eq!(learnable("open this", &gist("open this", &a)), Err(NotLearned::Deictic));
        for d in ["close that", "open it", "put these here", "send them there", "open those"] {
            assert_eq!(learnable(d, &gist(d, &a)), Err(NotLearned::Deictic), "{d}");
        }
        assert_eq!(learnable("open", &gist("open", &a)), Err(NotLearned::Short));
        assert_eq!(learnable("um outlook please", &gist("um outlook please", &a)), Err(NotLearned::Short));
        assert_eq!(learnable("where was I", &gist("where was I", &a)), Err(NotLearned::CommandWord));
        assert_eq!(learnable("open outlook", &gist("open outlook", &a)), Ok(()));
    }

    #[test]
    fn glim_learns_only_what_it_saw_come_to_the_front() {
        assert!(forward_ok("OUTLOOK.EXE", Some("outlook.exe"), Some("WINWORD.EXE")));
        assert!(forward_ok("OUTLOOK.EXE", Some("OUTLOOK.EXE"), Some("OUTLOOK.EXE")), "already in front counts");
        assert!(!forward_ok("WINWORD.EXE", Some("OUTLOOK.EXE"), Some("WINWORD.EXE")));
        // A file Glim can't know: another app than before, never Glim.
        assert!(forward_ok("Spotify.exe", None, Some("WINWORD.EXE")));
        assert!(!forward_ok("WINWORD.EXE", None, Some("winword.exe")));
        assert!(!forward_ok("glim.exe", None, None));
        assert!(forward_ok("Spotify.exe", None, None));
        assert_eq!(own_exe("C:\\Apps\\Spotify\\Spotify.EXE").as_deref(), Some("Spotify.EXE"));
        assert_eq!(own_exe("C:\\Users\\x\\Desktop\\Notes.lnk"), None);
        assert_eq!(own_exe(""), None);
    }

    #[test]
    fn two_failed_replays_in_a_row_delete_it_and_a_success_resets() {
        let mut s = Shortcuts::default();
        let g = gist("open outlook", &apps());
        s.learn(&g, open("outlook"), 100);
        assert!(!s.failed(0));
        s.used(0, 200);
        assert_eq!(s.items[0].fails, 0);
        assert!(!s.failed(0));
        assert!(s.failed(0));
        assert!(s.items.is_empty());
    }

    #[test]
    fn learning_again_replaces_and_try_again_removes() {
        let mut s = Shortcuts::default();
        let g = gist("where was I on my-repo", &apps());
        s.learn(&g, Action::ResumeWork { repo: None }, 100);
        s.learn(&g, Action::ResumeWork { repo: Some("my-repo".into()) }, 150);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].action, Action::ResumeWork { repo: Some("my-repo".into()) });
        assert!(s.remove(&g.plain, false));
        assert!(!s.remove(&g.plain, false));
    }

    #[test]
    fn a_month_unused_expires_and_the_200_limit_drops_the_least_recently_used() {
        let mut s = Shortcuts::default();
        let day = 86_400;
        s.learn(&gist("open outlook", &apps()), open("outlook"), 0);
        s.learn(&gist("where was I on my-repo", &apps()), Action::ResumeWork { repo: None }, 20 * day);
        assert_eq!(s.prune(31 * day), 1);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].action, Action::ResumeWork { repo: None });

        let mut s = Shortcuts::default();
        for i in 0..=MAX_SHORTCUTS as u64 {
            s.learn(&gist(&format!("pick up repo{i} now"), &apps()), Action::ResumeWork { repo: None }, 1_000 + i);
        }
        assert_eq!(s.items.len(), MAX_SHORTCUTS);
        assert!(!s.items.iter().any(|x| x.key == "pick repo0"), "the oldest went");
        assert!(s.items.iter().any(|x| x.key == "pick repo1"));
    }

    #[test]
    fn routine_names_follow_the_rules() {
        let own = vec!["Spotify".to_string()];
        assert_eq!(name_problem("work mode", &own, &[]), None);
        assert_eq!(name_problem("x", &own, &[]), Some("name-length"));
        assert_eq!(name_problem(&"a".repeat(41), &own, &[]), Some("name-length"));
        assert_eq!(name_problem("work: mode", &own, &[]), Some("name-chars"));
        assert_eq!(name_problem("do this", &own, &[]), Some("name-deictic"));
        assert_eq!(name_problem("Claude Code", &own, &[]), Some("name-app"));
        assert_eq!(name_problem("spotify", &own, &[]), Some("name-app"));
        assert_eq!(name_problem("open", &own, &[]), Some("name-command"));
        assert_eq!(name_problem("where was I", &own, &[]), Some("name-command"));
        assert_eq!(name_problem("Mode work", &own, &["work mode"]), Some("name-taken"));
        assert_eq!(name_problem("the a", &own, &[]), Some("name-length"));
    }

    #[test]
    fn a_routine_needs_one_to_five_steps_and_at_most_twenty_exist() {
        let mut r = Routines::default();
        assert_eq!(r.add("work mode", vec![], &[], 1), Err("no-steps"));
        assert_eq!(r.add("work mode", vec![open("outlook"); 6], &[], 1), Err("too-many-steps"));
        assert_eq!(r.add("work mode", vec![open("outlook"); 5], &[], 1), Ok(()));
        for i in 1..MAX_ROUTINES {
            r.add(&format!("mode{i}"), vec![open("codex")], &[], 1).unwrap();
        }
        assert_eq!(r.add("one more", vec![open("codex")], &[], 1), Err("too-many-routines"));
    }

    #[test]
    fn a_routine_is_matched_by_its_name_with_or_without_run() {
        let mut r = Routines::default();
        r.add("work mode", vec![open("outlook"), open("codex")], &[], 1).unwrap();
        let a = apps();
        for said in ["work mode", "Work mode, please", "run work mode", "start work mode", "mode work"] {
            assert_eq!(r.find(&gist(said, &a)).map(|x| x.name.as_str()), Some("work mode"), "{said}");
        }
        assert!(r.find(&gist("work", &a)).is_none());
        assert!(r.find(&gist("work mode now and outlook", &a)).is_none());
        assert_eq!(r.rename("work mode", "focus time", &[]), Ok(()));
        assert!(r.find(&gist("focus time", &a)).is_some());
        assert_eq!(r.rename("focus time", "open", &[]), Err("name-command"));
        assert!(r.remove("focus time"));
        assert!(r.items.is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glim-learn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_round_trip_and_a_corrupt_one_is_set_aside_as_empty() {
        let dir = scratch("files");
        let mut l = Learned::load(&dir);
        assert!(l.shortcuts.items.is_empty() && l.routines.items.is_empty());
        l.shortcuts.learn(&gist("open outlook", &apps()), open("outlook"), now());
        l.routines.add("work mode", vec![open("outlook"), Action::ResumeWork { repo: Some("my-repo".into()) }], &[], 1).unwrap();
        l.save_shortcuts();
        l.save_routines();
        let again = Learned::load(&dir);
        assert_eq!(again.shortcuts, l.shortcuts);
        assert_eq!(again.routines, l.routines);
        // Only the two files: no temp file left behind.
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, [ROUTINES_FILE, SHORTCUTS_FILE]);
        // Truncated: set aside, empty.
        let bytes = std::fs::read(dir.join(SHORTCUTS_FILE)).unwrap();
        std::fs::write(dir.join(SHORTCUTS_FILE), &bytes[..bytes.len() / 2]).unwrap();
        let broken = Learned::load(&dir);
        assert!(broken.shortcuts.items.is_empty());
        assert_eq!(std::fs::read(dir.join("shortcuts.json.bad")).unwrap(), &bytes[..bytes.len() / 2]);
        assert!(!dir.join(SHORTCUTS_FILE).exists());
        assert_eq!(broken.routines, l.routines, "the other file is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_holds_calls_with_enum_values_and_names_never_paths() {
        let mut s = Shortcuts::default();
        s.learn(&gist("open spotify", &apps()), open("Spotify"), 5);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"items":[{"key":"open {app}","action":{"tool":"open_app","app":"Spotify"},"slot":true,"lastUsed":5,"fails":0}]}"#);
        let r: Routine = serde_json::from_str(r#"{"name":"work mode","steps":[{"tool":"resume_work"},{"tool":"open_app","app":"codex"}]}"#).unwrap();
        assert_eq!(r.steps, vec![Action::ResumeWork { repo: None }, open("codex")]);
        // No other tool can be in a file.
        assert!(serde_json::from_str::<Action>(r#"{"tool":"draft_email","to":["x"]}"#).is_err());
    }

    #[test]
    fn the_log_lines_carry_tool_names_codes_and_counts_only() {
        // Every "learn:" line in this file and in voice/run.rs, as written:
        // only these placeholders may be filled in.
        let allowed = ["{gone}", "{}", "{tool}", "{code}", "{n}", "{k}", "{total}"];
        for (file, src) in [("learn/mod.rs", include_str!("mod.rs")), ("voice/run.rs", include_str!("../voice/run.rs"))] {
            let src = src.split("#[cfg(test)]").next().unwrap();
            for (at, _) in src.match_indices("\"learn: ") {
                let line = &src[at..at + src[at + 1..].find('"').unwrap() + 2];
                let mut rest = line;
                while let Some(open) = rest.find('{') {
                    let close = rest[open..].find('}').map(|c| open + c + 1).unwrap_or(rest.len());
                    assert!(allowed.contains(&&rest[open..close]), "{file}: {line}");
                    rest = &rest[close..];
                }
            }
        }
        let l = Learned { shortcuts: Shortcuts::default(), routines: Routines::default(), dir: PathBuf::new() };
        assert_eq!(l.counts(), "learn: 0 shortcuts, 0 routines");
    }
}
