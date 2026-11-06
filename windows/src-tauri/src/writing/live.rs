// Live revisions in the capture thread (owner, 2026-11-03): when typing
// pauses (Settings → Writing → Live revisions → pause delay, 1-3 s, 1.5 s by
// default) in a paragraph of at least MIN_WORDS words that was edited since
// its last check, that version goes to Claude (ai/live.rs). One request at a
// time; an answer for a version that is no longer the one in the field is
// stale and dropped. "Unchanged" shows nothing; an edit that changes more
// than MAX_CHANGED of the words is dropped. What is shown (PR 3 draws it) is
// a word-level diff, kept with the paragraph exactly as it was sent so
// accepting it can check the field still holds that text.
//
// It runs only where the writing checker reads (the paragraph is the
// checker's `current`), so never in a password field, a payment or ID field,
// or an app or site on the pause lists; and only in apps its own switch
// leaves on (all by default). Nothing here logs or keeps text beyond the
// paragraph on its way or shown.

use std::collections::{BTreeMap, VecDeque};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{FixMode, Session};

/// Not for a few words: nothing worth a request yet.
pub const MIN_WORDS: usize = 5;
/// An edit changing more than this share of the words is dropped.
pub const MAX_CHANGED: f64 = 0.60;
/// The island hears live revisions on this.
pub const EVENT: &str = "writing-live";
pub const DEFAULT_PAUSE_MS: u64 = 1500;
pub const MIN_PAUSE_MS: u64 = 1000;
pub const MAX_PAUSE_MS: u64 = 3000;
/// A request older than this is given up (its answer, if it comes, is stale)
/// so the next pause can go out.
pub const GIVE_UP: Duration = Duration::from_secs(12);
/// How many paragraph versions are remembered as already checked.
const REMEMBERED: usize = 64;

/// Settings → Writing → Live revisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub on: bool,
    /// Per-app switches by lowercase exe; an app not here is on.
    pub apps: BTreeMap<String, bool>,
    pub pause: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config { on: true, apps: BTreeMap::new(), pause: Duration::from_millis(DEFAULT_PAUSE_MS) }
    }
}

impl Config {
    pub fn from_settings(s: &crate::settings::Settings) -> Config {
        Config {
            on: s.live_revisions,
            apps: super::prefs::app_switches(&s.live_revision_apps),
            pause: Duration::from_millis(u64::from(s.live_pause_ms).clamp(MIN_PAUSE_MS, MAX_PAUSE_MS)),
        }
    }

    /// Whether live revisions run in this app.
    pub fn on_in(&self, exe: &str) -> bool {
        self.on && self.apps.get(&exe.to_ascii_lowercase()).copied().unwrap_or(true)
    }
}

static CONFIG: RwLock<Option<Config>> = RwLock::new(None);

pub fn set_config(c: Config) {
    *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = Some(c);
}

pub fn config() -> Config {
    CONFIG.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// The live revision's part of the writing Session.
#[derive(Default)]
pub(super) struct LiveState {
    pending: Option<Pending>,
    /// Paragraph versions already sent (newest last).
    sent: VecDeque<u64>,
    /// The paragraph last seen: where it starts and which version it was.
    seen: Option<(usize, u64)>,
    /// The version the user's typing made (edited in place), if the current
    /// paragraph is one: only an edited paragraph is revised.
    edited: Option<u64>,
    shown: Option<Shown>,
    /// A shown revision went away: the island is told once.
    hidden: Option<(u64, &'static str)>,
}

struct Pending {
    id: u64,
    key: u64,
    sent_at: Instant,
}

/// A revision about to go out. Its Debug shows no text.
pub struct LiveSource {
    pub id: u64,
    pub paragraph: String,
    pub before: String,
    pub chat: bool,
    /// How long the paragraph had stayed the same when it went out.
    pub paused_for: Duration,
}

impl std::fmt::Debug for LiveSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSource").field("id", &self.id).field("chars", &self.paragraph.chars().count()).field("chat", &self.chat).finish()
    }
}

/// A revision on show: the paragraph exactly as it was sent, where it
/// starts in the field, and the edit.
#[derive(Clone, PartialEq, Eq)]
pub struct Shown {
    pub id: u64,
    pub key: u64,
    pub paragraph: String,
    pub start: usize,
    pub revised: String,
    pub app: String,
    pub mode: FixMode,
    pub at: Instant,
}

/// One piece of the word-level diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffOp {
    /// "same", "del" (struck through) or "add" (highlighted).
    pub kind: &'static str,
    pub text: String,
}

/// What the island is told: a revision to show, or that it went away.
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEvent {
    /// "show" or "hide"; "sending" and "sent" around the request (the
    /// lantern's data-leaving look).
    pub phase: &'static str,
    pub id: u64,
    pub ops: Vec<DiffOp>,
    pub app: String,
    /// "hide": why ("typing", "field", "dismissed", "timeout", "applied" …).
    pub why: &'static str,
}

impl std::fmt::Debug for LiveEvent {
    /// Never the text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveEvent").field("phase", &self.phase).field("id", &self.id).field("ops", &self.ops.len()).field("why", &self.why).finish()
    }
}

/// What became of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Shown(LiveEvent),
    /// The edit is the paragraph as it was: nothing shown.
    Unchanged,
    /// More than MAX_CHANGED of the words changed: dropped.
    TooDifferent,
    /// Not the request on its way, or the paragraph changed since: dropped.
    Stale,
}

impl Outcome {
    /// The log's word.
    pub fn word(&self) -> &'static str {
        match self {
            Outcome::Shown(_) => "shown",
            Outcome::Unchanged => "unchanged",
            Outcome::TooDifferent => "too-different",
            Outcome::Stale => "stale",
        }
    }
}

impl Session {
    /// After each reading that set `current` (writing::Session::observe_in):
    /// whether the paragraph was edited in place, and a shown revision goes
    /// away once its paragraph is no longer the one in the field.
    pub(super) fn live_note(&mut self) {
        let Some(c) = self.current.as_ref() else {
            self.live.seen = None;
            self.live.edited = None;
            self.live_hide("field");
            return;
        };
        let (start, key) = (c.start, c.key);
        match self.live.seen {
            Some((s, k)) if s == start && k != key => self.live.edited = Some(key),
            Some((s, _)) if s != start => self.live.edited = None,
            None => self.live.edited = None,
            _ => {}
        }
        self.live.seen = Some((start, key));
        if self.live.shown.as_ref().is_some_and(|s| s.key != key) {
            self.live_hide("typing");
        }
    }

    /// The field changed, or nothing may be read: everything live goes.
    pub(super) fn live_reset(&mut self, why: &'static str) {
        self.live.seen = None;
        self.live.edited = None;
        self.live_hide(why);
    }

    /// The shown revision goes (the island is told once, `live_hidden`).
    pub fn live_hide(&mut self, why: &'static str) {
        if let Some(s) = self.live.shown.take() {
            self.live.hidden = Some((s.id, why));
        }
    }

    /// A revision that went away since the last call: the island's "hide".
    pub fn live_hidden(&mut self) -> Option<LiveEvent> {
        self.live.hidden.take().map(|(id, why)| LiveEvent { phase: "hide", id, why, ..Default::default() })
    }

    /// The revision on show, if any.
    pub fn live_shown(&self) -> Option<&Shown> {
        self.live.shown.as_ref()
    }

    /// When a revision is due: the paragraph was edited, has MIN_WORDS words,
    /// hasn't gone out in this version, and has stayed the same for the
    /// pause; nothing else is on its way. Marks it as gone out.
    pub fn live_take(&mut self, now: Instant, cfg: &Config, exe: &str, chat: bool) -> Option<LiveSource> {
        if !cfg.on_in(exe) || self.withheld {
            return None;
        }
        if let Some(p) = &self.live.pending {
            if now.duration_since(p.sent_at) < GIVE_UP {
                return None;
            }
            self.live.pending = None;
        }
        let c = self.current.as_ref()?;
        let paused_for = self.debounce.stable_for(now)?;
        let due = self.live.edited == Some(c.key)
            && !self.live.sent.contains(&c.key)
            && self.live.shown.as_ref().is_none_or(|s| s.key != c.key)
            && c.text.split_whitespace().count() >= MIN_WORDS
            && self.debounce.key() == Some(c.key)
            && paused_for >= cfg.pause;
        if !due {
            return None;
        }
        let (key, paragraph, before) = (c.key, c.text.clone(), c.before.clone());
        if self.live.sent.len() == REMEMBERED {
            self.live.sent.pop_front();
        }
        self.live.sent.push_back(key);
        self.next_id += 1;
        let id = self.next_id;
        self.live.pending = Some(Pending { id, key, sent_at: now });
        Some(LiveSource { id, paragraph, before, chat, paused_for })
    }

    /// The request `id` was held back before it left (ask first, consent.rs:
    /// the card is up): nothing on its way, and that version may still go
    /// out, so "Turn on" sends the paragraph the user paused on without more
    /// typing (away-mode live test 2026-11-05: it was used up by the card).
    pub fn live_held(&mut self, id: u64) {
        if let Some(p) = self.live.pending.take_if(|p| p.id == id) {
            self.live.sent.retain(|k| *k != p.key);
        }
    }

    /// The request `id` failed: nothing on its way (that version is not
    /// sent again until it changes).
    pub fn live_failed(&mut self, id: u64) {
        if self.live.pending.as_ref().is_some_and(|p| p.id == id) {
            self.live.pending = None;
        }
    }

    /// The edit for request `id`.
    pub fn live_answer(&mut self, id: u64, revised: &str, now: Instant) -> Outcome {
        let Some(p) = self.live.pending.take_if(|p| p.id == id) else {
            return Outcome::Stale;
        };
        let Some(c) = self.current.as_ref().filter(|c| c.key == p.key) else {
            return Outcome::Stale;
        };
        let revised = revised.trim();
        if revised == c.text.trim() {
            return Outcome::Unchanged;
        }
        if changed_share(&c.text, revised) > MAX_CHANGED {
            return Outcome::TooDifferent;
        }
        let ops = word_diff(&c.text, revised);
        let shown = Shown { id, key: c.key, paragraph: c.text.clone(), start: c.start, revised: revised.to_string(), app: c.app.clone(), mode: c.mode, at: now };
        let event = LiveEvent { phase: "show", id, ops, app: c.app.clone(), why: "" };
        self.live.hidden = None;
        self.live.shown = Some(shown);
        Outcome::Shown(event)
    }
}

// ── The preview and the accept key (PR 3) ─────────────────────────────────

/// A shown revision goes after this long unseen-to.
pub const SHOW_FOR: Duration = Duration::from_secs(20);

/// The preview's size in logical pixels: three lines of the diff and the
/// key hint; taller once expanded ("…" clicked).
pub const PREVIEW_W: f64 = 440.0;
pub const PREVIEW_H: f64 = 92.0;
pub const PREVIEW_H_OPEN: f64 = 260.0;
/// Between the preview and the paragraph it sits above.
pub const PREVIEW_GAP: f64 = 6.0;

use super::bubble::{Monitor, Rect};

/// Where the preview goes: just above the paragraph (its rectangle, else
/// the caret's line), left-aligned with it, inside its monitor's work area.
/// None when there is no geometry or no room above: the caller falls back
/// to the island's top.
pub fn preview_rect(paragraph: Option<Rect>, line: Option<Rect>, field: Option<Rect>, monitors: &[Monitor], expanded: bool) -> Option<Rect> {
    let anchor = paragraph.filter(|r| r.height() > 0).or(line.filter(|r| r.height() > 0))?;
    let left_edge = field.filter(|f| f.width() > 0).map_or(anchor.left, |f| f.left.max(anchor.left.min(f.right)));
    let m = monitors.iter().find(|m| {
        let w = m.work;
        anchor.left >= w.left && anchor.left < w.right && anchor.top >= w.top && anchor.top < w.bottom
    })?;
    let px = |v: f64| (v * m.scale).round() as i32;
    let (w, h, gap) = (px(PREVIEW_W), px(if expanded { PREVIEW_H_OPEN } else { PREVIEW_H }), px(PREVIEW_GAP));
    let top = anchor.top - gap - h;
    if top < m.work.top {
        return None;
    }
    let left = left_edge.min(m.work.right - w).max(m.work.left);
    Some(Rect::new(left, top, left + w, top + h))
}

/// The fallback: centred under the island's top edge (its window's
/// rectangle), inside the work area of the monitor it is on.
pub fn island_rect(island: Rect, monitors: &[Monitor], expanded: bool) -> Option<Rect> {
    let m = monitors.iter().find(|m| {
        let (x, y) = (island.left + island.width() / 2, island.top);
        x >= m.work.left && x < m.work.right && y >= m.work.top - 1 && y < m.work.bottom
    })?;
    let px = |v: f64| (v * m.scale).round() as i32;
    let (w, h) = (px(PREVIEW_W), px(if expanded { PREVIEW_H_OPEN } else { PREVIEW_H }));
    let left = (island.left + island.width() / 2 - w / 2).min(m.work.right - w).max(m.work.left);
    let top = (island.top + px(44.0)).max(m.work.top);
    Some(Rect::new(left, top, left + w, top + h))
}

impl Session {
    /// A shown revision past SHOW_FOR goes ("timeout").
    pub fn live_expire(&mut self, now: Instant) {
        if self.live.shown.as_ref().is_some_and(|s| now.duration_since(s.at) >= SHOW_FOR) {
            self.live_hide("timeout");
        }
    }

    /// The accept key: the shown revision as a job for writing::apply (which
    /// finds the paragraph word for word in the field again before typing).
    /// Changed when the field no longer holds the paragraph exactly as it
    /// was sent (or nothing is shown); Blocked where Glim doesn't type
    /// (suggestions-only apps: the caller copies instead).
    pub fn live_job(&self) -> Result<super::Job, super::Outcome> {
        use super::Outcome;
        let s = self.live.shown.as_ref().ok_or(Outcome::Changed)?;
        // The field as last read must still hold that version.
        self.current.as_ref().filter(|c| c.key == s.key && c.text == s.paragraph).ok_or(Outcome::Changed)?;
        match s.mode {
            FixMode::Apply => {}
            FixMode::Away => return Err(Outcome::Away),
            FixMode::View | FixMode::Copy => return Err(Outcome::Blocked),
        }
        if !super::offerable(&s.revised) {
            return Err(Outcome::Blocked);
        }
        Ok(super::Job { paragraph: s.paragraph.clone(), field_start: s.start, start: 0, end: s.paragraph.chars().count(), replacement: s.revised.clone(), app: s.app.clone() })
    }

    /// Suggestions-only apps: the revision to copy, if the field still holds
    /// the paragraph as sent and the revision is safe to put on the clipboard.
    pub fn live_copy(&self) -> Result<String, super::Outcome> {
        use super::Outcome;
        let s = self.live.shown.as_ref().ok_or(Outcome::Changed)?;
        self.current.as_ref().filter(|c| c.key == s.key && c.text == s.paragraph).ok_or(Outcome::Changed)?;
        if crate::capture::sensitive::contains_sensitive_number(&s.revised) {
            return Err(Outcome::Blocked);
        }
        Ok(s.revised.clone())
    }
}

/// How often the connection is warmed while the user types.
pub const WARM_EVERY: Duration = Duration::from_secs(60);

impl Session {
    /// Typing in an edited paragraph that may go out at the next pause:
    /// worth opening the connection now (the caller limits it to WARM_EVERY).
    pub fn live_warm_due(&self, cfg: &Config, exe: &str) -> bool {
        let Some(c) = self.current.as_ref() else { return false };
        cfg.on_in(exe) && !self.withheld && self.live.pending.is_none() && self.live.edited == Some(c.key) && c.text.split_whitespace().count() >= MIN_WORDS.saturating_sub(2)
    }
}

/// #75's pre-warm for live revisions: the connection to the Claude API is
/// opened while the user types (a bare `HEAD /`: no body, no key, no text;
/// net::prewarm), so the request at the pause doesn't wait for it. Only
/// with a key saved and the Claude brain; GLIM_NO_PREWARM=1 turns it off.
pub fn prewarm() {
    if std::env::var_os("GLIM_NO_PREWARM").is_some_and(|v| v == "1") {
        return;
    }
    if crate::ai::rewrite::config().brain != crate::ai::rewrite::Brain::Claude {
        return;
    }
    tauri::async_runtime::spawn(async {
        if !matches!(crate::ai::key::load(&crate::ai::key::vault()), Ok(Some(_))) {
            return;
        }
        if let Ok(base) = crate::net::Url::parse(crate::ai::claude::API_BASE) {
            let _ = crate::net::prewarm(&base).await;
        }
    });
}

/// Words and the spaces after them: "Hi  there," → ["Hi  ", "there,"].
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        if !space && in_space {
            out.push(&text[start..i]);
            start = i;
        }
        in_space = space;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The longest common subsequence of two word lists, as index pairs.
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if a[i] == b[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn words(text: &str) -> Vec<&str> {
    tokens(text).into_iter().map(str::trim_end).collect()
}

/// The share of words changed between `old` and `new` (0 = the same words,
/// 1 = none in common): 1 - common / the longer one's word count.
pub fn changed_share(old: &str, new: &str) -> f64 {
    let (a, b) = (words(old), words(new));
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    1.0 - lcs(&a, &b).len() as f64 / longest as f64
}

/// The word-level diff from `old` to `new`: runs of kept, removed and added
/// words (each word with the spaces after it, as in its own text).
pub fn word_diff(old: &str, new: &str) -> Vec<DiffOp> {
    let (ta, tb) = (tokens(old), tokens(new));
    let (a, b): (Vec<&str>, Vec<&str>) = (ta.iter().map(|t| t.trim_end()).collect(), tb.iter().map(|t| t.trim_end()).collect());
    let pairs = lcs(&a, &b);
    let mut ops: Vec<DiffOp> = Vec::new();
    let mut push = |kind: &'static str, text: &str| match ops.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(text),
        _ => ops.push(DiffOp { kind, text: text.to_string() }),
    };
    let (mut i, mut j) = (0, 0);
    for (pi, pj) in pairs.into_iter().chain(std::iter::once((a.len(), b.len()))) {
        while i < pi {
            push("del", ta[i]);
            i += 1;
        }
        while j < pj {
            push("add", tb[j]);
            j += 1;
        }
        if pi < a.len() && pj < b.len() {
            push("same", tb[pj]);
            i += 1;
            j += 1;
        }
    }
    ops
}

#[cfg(test)]
mod tests;
