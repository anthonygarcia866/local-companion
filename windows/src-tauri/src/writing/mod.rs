// Phase 1a: the instant writing checker (docs/phase1-plan.md).
//
// The capture thread hands each reading of the focused field to a `Session`:
// about 1 s after the text stops changing, the paragraph around the caret
// (signature and quoted text left out, see text.rs) goes through Harper
// (checker.rs), and the findings go to the island as `Suggestions`.
//
// Privacy, as for capture: the field's text never leaves the capture thread.
// Only suggestions do (the problem words, a little context, Harper's message
// and replacements), to the island, live, never stored. Suggestions and
// paragraphs have Debug impls without their words, and nothing here logs
// anything but counts and timings (tests check the source).

pub mod apply;
pub mod apps;
pub mod ask;
pub mod checker;
pub mod docs_tip;
pub mod prefs;
pub mod text;
pub mod tones;

use serde::Serialize;
use std::time::{Duration, Instant};

pub use checker::Rules;
use checker::{Checker, Finding};
use text::{paragraph_to_check, Paragraph};

/// How long the text must stay unchanged before it is checked (the owner:
/// 1 s felt slow, 2026-10-10). Readings come every 300 ms, so a check runs
/// 600-900 ms after typing stops.
pub const DEBOUNCE: Duration = Duration::from_millis(600);
/// Characters of context shown around a problem in the island.
pub const CONTEXT_CHARS: usize = 24;
/// The event the island listens to.
pub const EVENT: &str = "writing-suggestions";

/// One problem, for the island.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// Offsets in characters from the start of the field (for 1b).
    pub start: usize,
    pub end: usize,
    pub problem: String,
    pub before: String,
    pub after: String,
    pub kind: String,
    pub message: String,
    pub replacements: Vec<String>,
    /// The first replacement is clearly the best (checker::rank): Apply all
    /// skips the others.
    pub clear: bool,
}

impl std::fmt::Debug for Suggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggestion").field("start", &self.start).field("end", &self.end).field("kind", &self.kind).finish()
    }
}

/// Glim's Pause (the tray), as Rust knows it: the island tells it
/// (`writing_set_paused`). While paused nothing is read, checked, kept or typed.
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_paused(on: bool) {
    PAUSED.store(on, std::sync::atomic::Ordering::SeqCst);
}

pub fn paused() -> bool {
    PAUSED.load(std::sync::atomic::Ordering::SeqCst)
}

/// What clicking a replacement does (1b, docs/phase1-plan.md §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FixMode {
    /// Nothing: an app not yet tested for fixes (chat apps and their web
    /// versions, Edge, new Outlook).
    #[default]
    View,
    /// Typed into the field (§7.3).
    Apply,
    /// A field Glim can't select or type in (ValuePattern only): each
    /// replacement is a Copy button the user clicks (§7.7).
    Copy,
    /// Fixes would be typed, but the user is no longer in that text (another
    /// field or app, another paragraph): "Click in the field first".
    Away,
}

/// The fix mode for a field: by app and site, then by what UI Automation
/// offers. `chat_site`: the field is on a web chat app (apps::CHAT_SITES).
pub fn fix_mode(exe: &str, pattern: &str, chat_site: bool) -> FixMode {
    if !apps::fixes(exe) || chat_site {
        FixMode::View
    } else if pattern.starts_with("TextPattern") {
        FixMode::Apply
    } else if pattern == "ValuePattern" {
        FixMode::Copy
    } else {
        FixMode::View
    }
}

/// What the island shows: the current field's suggestions (empty = none).
#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestions {
    /// Changes whenever the list does: a click on an older list is refused.
    pub check_id: u64,
    pub mode: FixMode,
    /// The app's short name ("Word"), for the island's notes.
    pub app: String,
    pub items: Vec<Suggestion>,
}

impl std::fmt::Debug for Suggestions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggestions").field("count", &self.items.len()).field("mode", &self.mode).finish()
    }
}

/// The longest a checked paragraph is kept for a fix (owner, 2026-10-11).
pub const KEEP_FOR: Duration = Duration::from_secs(10 * 60);

/// The paragraph the shown suggestions came from, kept (in memory only) so a
/// fix can find it again (§7.2). Only in fields where fixes are typed.
struct Kept {
    text: String,
    /// Where it starts in the field, in characters.
    start: usize,
    key: u64,
    at: Instant,
}

/// Everything one fix needs, handed to the apply step (apply.rs).
pub struct Job {
    /// The paragraph as it was checked.
    pub paragraph: String,
    /// Where it started in the field, in characters (a tie-breaker only).
    pub field_start: usize,
    /// The problem's span in the paragraph, in characters.
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    /// The app's short name, for the island's note.
    pub app: String,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").field("start", &self.start).field("end", &self.end).field("app", &self.app).finish()
    }
}

/// Why a click did nothing, or that it worked. Sent to the island as a code
/// and logged as one (never with text).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Applied,
    Copied,
    /// The text or the list changed since the check: nothing typed, rechecked.
    Changed,
    /// The user isn't in that field.
    Away,
    Paused,
    /// The field is protected now (password, payment labels, paused app or
    /// site, read-only) or the app isn't one fixes are typed in.
    Blocked,
    /// An app running as administrator: Windows blocks Glim's keys.
    Elevated,
    /// A key or mouse button was held down.
    KeysHeld,
    /// The app didn't let Glim select the text.
    Unsupported,
    /// Typed, but the read-back doesn't match: the user is told to undo.
    NotClean,
    /// Apply all: no suggestion's first option is clearly the best.
    Unclear,
    /// The clipboard was busy.
    ClipboardBusy,
}

impl Outcome {
    pub fn code(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Copied => "copied",
            Outcome::Changed => "changed",
            Outcome::Away => "away",
            Outcome::Paused => "paused",
            Outcome::Blocked => "blocked",
            Outcome::Elevated => "elevated",
            Outcome::KeysHeld => "keys-held",
            Outcome::Unsupported => "unsupported",
            Outcome::NotClean => "not-clean",
            Outcome::Unclear => "unclear",
            Outcome::ClipboardBusy => "clipboard-busy",
        }
    }
}

/// The island's note after a click.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixResult {
    pub outcome: Outcome,
    pub app: String,
    /// Apply all: how many fixes went in, of how many (both 0 for one fix).
    pub done: usize,
    pub total: usize,
    /// Apply all: suggestions left to the user, their first option not
    /// clearly the best.
    pub skipped: usize,
}

/// The event the island hears a click's outcome on.
pub const RESULT_EVENT: &str = "writing-fix";

/// A replacement Glim may offer: printable text only (Enter would send a chat
/// message; a tab moves focus), and never something that looks like a card,
/// an SSN or an IBAN.
pub fn offerable(replacement: &str) -> bool {
    !replacement.chars().any(char::is_control) && !crate::capture::sensitive::contains_sensitive_number(replacement)
}

/// What a reading calls for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Wait,
    Check,
}

/// Runs a check once the key (the paragraph and where it starts) has stayed
/// the same for DEBOUNCE, and not again until it changes.
#[derive(Default)]
pub struct Debouncer {
    key: Option<u64>,
    since: Option<Instant>,
    done: bool,
    /// Check on the next reading, without the wait (after a fix).
    now: bool,
}

impl Debouncer {
    pub fn feed(&mut self, key: u64, now: Instant) -> Step {
        if self.now {
            *self = Debouncer { key: Some(key), since: Some(now), done: true, now: false };
            return Step::Check;
        }
        if self.key != Some(key) {
            self.key = Some(key);
            self.since = Some(now);
            self.done = false;
            return Step::Wait;
        }
        if !self.done && self.since.is_some_and(|s| now.duration_since(s) >= DEBOUNCE) {
            self.done = true;
            return Step::Check;
        }
        Step::Wait
    }

    pub fn reset(&mut self) {
        *self = Debouncer::default();
    }

    /// The next reading is checked at once, whatever its text.
    pub fn check_next(&mut self) {
        self.now = true;
    }

    /// The paragraph currently being watched.
    pub fn key(&self) -> Option<u64> {
        self.key
    }

    /// How long the watched paragraph has stayed the same.
    pub fn stable_for(&self, now: Instant) -> Option<Duration> {
        self.since.map(|s| now.duration_since(s))
    }
}

fn key_of(p: &Paragraph) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.text.hash(&mut h);
    p.start.hash(&mut h);
    h.finish()
}

/// The checker's state in the capture thread.
#[derive(Default)]
pub struct Session {
    checker: Option<Checker>,
    debounce: Debouncer,
    shown: Suggestions,
    /// The mode the shown list was checked in (Apply turns into Away for the
    /// island once the paragraph is no longer kept).
    checked_mode: FixMode,
    kept: Option<Kept>,
    /// The paragraph whose suggestions were dismissed: not shown again until
    /// its text changes.
    dismissed: Option<u64>,
    next_id: u64,
    /// Check times since the last report, in microseconds.
    times: Vec<u128>,
    /// Paragraph lengths checked since the last report, in characters.
    lengths: Vec<usize>,
    /// The paragraph the caret is in now, in a field the checker is on: what
    /// a rewrite (1c) would send. Memory only, replaced on every reading.
    current: Option<Current>,
    /// The rewrite asked for or shown (1c): one at a time.
    rewrite: Option<RewriteKept>,
    /// "Check with AI" on its way (docs/phase1-plan.md §15.1): one at a time.
    ai: Option<AiPending>,
    /// The AI's suggestions for one paragraph version (its key): shown with
    /// Harper's while that paragraph stays as it is.
    ai_found: Option<(u64, Vec<Suggestion>)>,
    /// The paragraph version the automatic check last went out for: once
    /// per version.
    ai_auto_done: Option<u64>,
    /// The focused field's last reading was withheld: a payment or ID field,
    /// or text holding a card, SSN or IBAN. A rewrite asked for there says
    /// so (owner, 2026-10-13) instead of "click in the paragraph".
    withheld: bool,
}

/// The focused field's paragraph, for a rewrite.
struct Current {
    text: String,
    start: usize,
    /// key_of the paragraph: which version of it this is.
    key: u64,
    mode: FixMode,
    app: String,
}

/// A rewrite on its way or shown: the paragraph as it was sent (so applying
/// it re-verifies the field word for word, as a fix does) and the answer.
struct RewriteKept {
    id: u64,
    paragraph: String,
    start: usize,
    mode: FixMode,
    app: String,
    text: Option<String>,
    at: Instant,
}

/// The kind of a suggestion from "Check with AI": the island marks it.
pub const AI_KIND: &str = "AI";
/// "Check automatically when I pause typing": how long a pause.
pub const AI_AUTO_AFTER: Duration = Duration::from_secs(2);
/// Not for a word or two: nothing worth a request yet.
pub const AI_AUTO_MIN_WORDS: usize = 4;

/// A "Check with AI" request on its way: its id, the paragraph version.
struct AiPending {
    id: u64,
    key: u64,
    auto: bool,
}

/// Harper's suggestions plus the AI's that don't overlap any of them, in
/// reading order.
fn merged(mut harper: Vec<Suggestion>, ai: &[Suggestion]) -> Vec<Suggestion> {
    harper.retain(|s| s.kind != AI_KIND);
    let extra: Vec<Suggestion> = ai.iter().filter(|a| !harper.iter().any(|h| a.start < h.end && h.start < a.end)).cloned().collect();
    harper.extend(extra);
    harper.sort_by_key(|s| (s.start, s.end));
    harper
}

/// A rewrite about to be sent: its id and the paragraph.
pub struct RewriteSource {
    pub id: u64,
    pub paragraph: String,
}

impl std::fmt::Debug for RewriteSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RewriteSource").field("id", &self.id).field("chars", &self.paragraph.chars().count()).finish()
    }
}

/// A rewrite's answer as the island shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct RewriteShown {
    pub mode: FixMode,
    pub app: String,
    /// Glim would type it (writing::offerable): one paragraph of printable
    /// text that doesn't look like a card, an SSN or an IBAN.
    pub offerable: bool,
}

/// One reading, as the writing checker sees it.
pub struct Observed<'a> {
    /// False when focus moved to another field.
    pub same: bool,
    /// None when nothing may be checked (an app that is off, a skipped or
    /// dropped reading).
    pub text: Option<&'a str>,
    pub caret: Option<usize>,
    pub rules: Rules,
    pub mode: FixMode,
    /// The app's short name.
    pub app: &'a str,
    /// Glim is paused (the tray's Pause).
    pub paused: bool,
    /// The reading was withheld as sensitive (capture: a payment or ID field,
    /// or a card, SSN or IBAN in the text). Its text is None.
    pub sensitive: bool,
}

impl Session {
    /// One reading of the focused field, 1a's way: view-only, never paused.
    #[cfg(test)]
    pub fn observe(
        &mut self,
        same: bool,
        text: Option<&str>,
        caret: Option<usize>,
        rules: Rules,
        now: Instant,
        emit: impl FnMut(&Suggestions),
    ) {
        let r = Observed { same, text, caret, rules, mode: FixMode::View, app: "", paused: false, sensitive: false };
        self.observe_in(&r, now, emit);
    }

    /// One reading of the focused field. `emit` gets the island's new
    /// suggestions whenever they change.
    pub fn observe_in(&mut self, r: &Observed, now: Instant, mut emit: impl FnMut(&Suggestions)) {
        self.withheld = r.sensitive && !r.paused;
        // Paused: nothing is checked, kept or shown.
        if r.paused {
            self.kept = None;
            self.current = None;
            self.rewrite = None;
            self.ai = None;
            self.ai_found = None;
            self.debounce.reset();
            self.show(Suggestions::default(), FixMode::View, &mut emit);
            return;
        }
        // Kept 10 minutes at most; checked again if the user is still there.
        if self.kept.as_ref().is_some_and(|k| now.duration_since(k.at) >= KEEP_FOR) {
            self.kept = None;
            self.debounce.reset();
        }
        // The suggestions belong to the last field that was checked (the
        // owner, 2026-10-10). They stay while the user is anywhere nothing is
        // checked (another app, Glim itself, a password field) and come back
        // with that field without a gap. Another field replaces them with its
        // own check, ~0.6 s later. The kept paragraph doesn't stay: moving to
        // other text drops it (owner, 2026-10-11), and the island's fix
        // buttons say "Click in the field first" until the next check.
        if !r.same {
            self.debounce.reset();
            self.kept = None;
        }
        let Some(text) = r.text else {
            self.debounce.reset();
            self.kept = None;
            self.current = None;
            self.refresh(&mut emit);
            return;
        };
        // A field that is checked but has nothing to check here (a blank
        // paragraph, the caret in the signature or the quote): nothing shown.
        let Some(paragraph) = paragraph_to_check(text, r.caret) else {
            self.debounce.reset();
            self.kept = None;
            self.current = None;
            self.show(Suggestions::default(), FixMode::View, &mut emit);
            return;
        };
        self.current = Some(Current { text: paragraph.text.clone(), start: paragraph.start, key: key_of(&paragraph), mode: r.mode, app: r.app.to_string() });
        if self.rewrite.as_ref().is_some_and(|w| now.duration_since(w.at) >= KEEP_FOR) {
            self.rewrite = None;
        }
        let key = key_of(&paragraph);
        // Another paragraph, or this one edited: different text.
        if self.kept.as_ref().is_some_and(|k| k.key != key) {
            self.kept = None;
        }
        if self.dismissed.is_some_and(|d| d != key) {
            self.dismissed = None;
        }
        if self.ai_found.as_ref().is_some_and(|(k, _)| *k != key) {
            self.ai_found = None;
        }
        if self.debounce.feed(key, now) == Step::Check && self.dismissed.is_none() {
            let mut found = self.check(&paragraph, r.rules);
            if let Some((_, ai)) = &self.ai_found {
                found.items = merged(found.items, ai);
            }
            found.app = r.app.to_string();
            let mut mode = r.mode;
            if mode == FixMode::Apply && !found.items.is_empty() {
                // The sensitive-data scan runs again before anything is kept
                // (owner, 2026-10-11): a hit keeps nothing and shows nothing.
                if crate::capture::sensitive::contains_sensitive_number(&paragraph.text) {
                    found.items.clear();
                    mode = FixMode::View;
                } else {
                    self.kept = Some(Kept { text: paragraph.text.clone(), start: paragraph.start, key, at: now });
                }
            }
            self.show(found, mode, &mut emit);
        } else {
            self.refresh(&mut emit);
        }
    }

    /// The writing settings changed (an app switched off, 1d): everything
    /// shown, kept or remembered goes, and the field is looked at afresh.
    pub fn settings_changed(&mut self, mut emit: impl FnMut(&Suggestions)) {
        self.kept = None;
        self.current = None;
        self.rewrite = None;
        self.ai = None;
        self.ai_found = None;
        self.dismissed = None;
        self.debounce.reset();
        self.show(Suggestions::default(), FixMode::View, &mut emit);
    }

    /// Dismissed from the island: nothing shown or kept until the text changes.
    pub fn dismiss(&mut self, mut emit: impl FnMut(&Suggestions)) {
        self.dismissed = self.debounce.key();
        self.kept = None;
        self.ai_found = None;
        self.show(Suggestions::default(), FixMode::View, &mut emit);
    }

    /// The fix the island asked for (`check_id`, row, replacement): only
    /// numbers come from the island; the words are the checker's own.
    pub fn job(&self, check_id: u64, index: usize, choice: usize) -> Result<Job, Outcome> {
        if check_id != self.shown.check_id {
            return Err(Outcome::Changed);
        }
        match self.shown.mode {
            FixMode::Apply => {}
            FixMode::Away => return Err(Outcome::Away),
            FixMode::View | FixMode::Copy => return Err(Outcome::Blocked),
        }
        let kept = self.kept.as_ref().ok_or(Outcome::Away)?;
        let item = self.shown.items.get(index).ok_or(Outcome::Changed)?;
        let replacement = item.replacements.get(choice).ok_or(Outcome::Changed)?;
        let (start, end) = (item.start.checked_sub(kept.start).ok_or(Outcome::Changed)?, item.end - kept.start);
        let chars: Vec<char> = kept.text.chars().collect();
        if end > chars.len() || chars[start..end].iter().collect::<String>() != item.problem {
            return Err(Outcome::Changed);
        }
        Ok(Job {
            paragraph: kept.text.clone(),
            field_start: kept.start,
            start,
            end,
            replacement: replacement.clone(),
            app: self.shown.app.clone(),
        })
    }

    /// "Apply all": one job per suggestion, with its first replacement, from
    /// the end of the paragraph backward, so the fixes still to come keep
    /// their offsets. Each job carries the paragraph as the fixes before it in
    /// this order will have left it, so `apply::run` re-verifies each one word
    /// for word. A suggestion that overlaps one already taken, or whose
    /// replacement Glim wouldn't offer, is left out; one whose first option
    /// isn't clearly the best is left to the user and counted. Only the list
    /// id comes from the island.
    pub fn jobs_all(&self, check_id: u64) -> Result<(Vec<Job>, usize), Outcome> {
        if self.shown.items.is_empty() {
            return Err(Outcome::Changed);
        }
        let mut order: Vec<usize> = (0..self.shown.items.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.shown.items[i].start));
        let mut jobs: Vec<Job> = Vec::new();
        let mut paragraph: Option<String> = None;
        let mut floor = usize::MAX;
        let mut skipped = 0;
        for i in order {
            let item = &self.shown.items[i];
            if item.end > floor || !item.replacements.first().is_some_and(|r| offerable(r)) {
                continue;
            }
            if !item.clear {
                skipped += 1;
                continue;
            }
            let mut job = self.job(check_id, i, 0)?;
            if let Some(p) = &paragraph {
                job.paragraph = p.clone();
            }
            let chars: Vec<char> = job.paragraph.chars().collect();
            paragraph = Some(chars[..job.start].iter().collect::<String>() + &job.replacement + &chars[job.end..].iter().collect::<String>());
            floor = item.start;
            jobs.push(job);
        }
        if jobs.is_empty() {
            return Err(if skipped > 0 { Outcome::Unclear } else { Outcome::Changed });
        }
        Ok((jobs, skipped))
    }

    // ── 1c: the rewrite (docs/phase1-plan.md §13) ────────────────────────────

    /// A tone was clicked: the paragraph the caret is in goes out, under a
    /// new id. Away when there is none (no field, a blank line, an app the
    /// checker is off for). A rewrite already on its way or shown is replaced.
    /// Blocked when the field was withheld as sensitive: nothing of it was read.
    pub fn rewrite_begin(&mut self, now: Instant) -> Result<RewriteSource, Outcome> {
        let none = if self.withheld { Outcome::Blocked } else { Outcome::Away };
        let c = self.current.as_ref().ok_or(none)?;
        if c.text.trim().is_empty() {
            return Err(Outcome::Away);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.rewrite = Some(RewriteKept { id, paragraph: c.text.clone(), start: c.start, mode: c.mode, app: c.app.clone(), text: None, at: now });
        Ok(RewriteSource { id, paragraph: c.text.clone() })
    }

    /// The answer for rewrite `id` came back. None when that rewrite is no
    /// longer the one shown (dismissed, or another asked for since).
    pub fn rewrite_answer(&mut self, id: u64, text: String) -> Option<RewriteShown> {
        let w = self.rewrite.as_mut().filter(|w| w.id == id && w.text.is_none())?;
        let shown = RewriteShown { mode: w.mode, app: w.app.clone(), offerable: offerable(&text) };
        w.text = Some(text);
        Some(shown)
    }

    /// Whether `id` is the rewrite on its way or shown.
    pub fn rewrite_is(&self, id: u64) -> bool {
        self.rewrite.as_ref().is_some_and(|w| w.id == id)
    }

    /// "Use this": the whole paragraph, as it was sent, replaced by the
    /// rewrite, through 1b's apply path (apply::run re-verifies the field).
    pub fn rewrite_job(&self, id: u64) -> Result<Job, Outcome> {
        let w = self.rewrite.as_ref().filter(|w| w.id == id).ok_or(Outcome::Changed)?;
        let text = w.text.as_ref().ok_or(Outcome::Changed)?;
        match w.mode {
            FixMode::Apply => {}
            FixMode::Away => return Err(Outcome::Away),
            FixMode::View | FixMode::Copy => return Err(Outcome::Blocked),
        }
        if !offerable(text) {
            return Err(Outcome::Blocked);
        }
        Ok(Job { paragraph: w.paragraph.clone(), field_start: w.start, start: 0, end: w.paragraph.chars().count(), replacement: text.clone(), app: w.app.clone() })
    }

    /// The rewrite's text for its Copy button (fields Glim can't type in, or
    /// an answer Glim wouldn't type).
    pub fn rewrite_copy(&self, id: u64) -> Result<String, Outcome> {
        let w = self.rewrite.as_ref().filter(|w| w.id == id).ok_or(Outcome::Changed)?;
        let text = w.text.clone().ok_or(Outcome::Changed)?;
        if crate::capture::sensitive::contains_sensitive_number(&text) {
            return Err(Outcome::Blocked);
        }
        Ok(text)
    }

    /// The app's name for the rewrite's notes.
    pub fn rewrite_app(&self) -> String {
        self.rewrite.as_ref().map(|w| w.app.clone()).unwrap_or_default()
    }

    /// Dismissed, applied or failed: the rewrite and its paragraph go.
    pub fn rewrite_clear(&mut self) {
        self.rewrite = None;
    }

    // ── "Check with AI" (docs/phase1-plan.md §15.1) ─────────────────────────

    /// "Check with AI" was clicked (or is due automatically): the paragraph
    /// the caret is in goes out, under a new id. Away when there is none;
    /// Blocked when the field was withheld as sensitive (nothing was read).
    pub fn ai_begin(&mut self, auto: bool) -> Result<RewriteSource, Outcome> {
        let none = if self.withheld { Outcome::Blocked } else { Outcome::Away };
        let c = self.current.as_ref().ok_or(none)?;
        if c.text.trim().is_empty() {
            return Err(Outcome::Away);
        }
        if auto {
            self.ai_auto_done = Some(c.key);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.ai = Some(AiPending { id, key: c.key, auto });
        Ok(RewriteSource { id, paragraph: c.text.clone() })
    }

    /// Whether `id` is the check on its way, and whether it was automatic.
    pub fn ai_pending(&self, id: u64) -> Option<bool> {
        self.ai.as_ref().filter(|a| a.id == id).map(|a| a.auto)
    }

    /// The AI's answer for check `id`. Shown with Harper's suggestions when
    /// the paragraph is still the version that was sent; None when it isn't
    /// (edited, another field, or another check since). Some(n): n new
    /// suggestions (those overlapping Harper's are left out).
    pub fn ai_answer(&mut self, id: u64, issues: &[crate::ai::check::Issue], mut emit: impl FnMut(&Suggestions)) -> Option<usize> {
        if self.ai.as_ref().is_none_or(|a| a.id != id) {
            return None;
        }
        let pending = self.ai.take()?;
        let c = self.current.as_ref().filter(|c| c.key == pending.key)?;
        let chars: Vec<char> = c.text.chars().collect();
        let found: Vec<Suggestion> = issues
            .iter()
            .filter(|i| i.end <= chars.len() && i.start < i.end)
            .map(|i| {
                let f = Finding { start: i.start, end: i.end, kind: AI_KIND.into(), message: i.message.clone(), replacements: vec![i.replacement.clone()], clear: false };
                suggestion(&chars, c.start, &f)
            })
            .collect();
        // Harper's list for this version, if it is the one shown.
        let harper: Vec<Suggestion> = if self.debounce.key() == Some(pending.key) {
            self.shown.items.iter().filter(|s| s.kind != AI_KIND).cloned().collect()
        } else {
            Vec::new()
        };
        let items = merged(harper.clone(), &found);
        let added = items.len() - harper.len();
        let (text, start, key, mode, app) = (c.text.clone(), c.start, c.key, c.mode, c.app.clone());
        self.ai_found = Some((key, found));
        self.dismissed = None;
        let mut mode = mode;
        if mode == FixMode::Apply && !items.is_empty() {
            if crate::capture::sensitive::contains_sensitive_number(&text) {
                return Some(0);
            }
            self.kept = Some(Kept { text, start, key, at: Instant::now() });
        } else if items.is_empty() {
            mode = FixMode::View;
        }
        self.show(Suggestions { items, app, ..Default::default() }, mode, &mut emit);
        Some(added)
    }

    /// The check `id` failed, or its answer is gone: nothing pending.
    pub fn ai_clear(&mut self, id: u64) {
        if self.ai.as_ref().is_some_and(|a| a.id == id) {
            self.ai = None;
        }
    }

    /// With "check automatically" on: the paragraph the caret is in has
    /// stayed the same for AI_AUTO_AFTER, and this version hasn't gone out.
    pub fn ai_auto_due(&self, now: Instant) -> bool {
        let Some(c) = self.current.as_ref() else { return false };
        !self.withheld
            && self.ai.is_none()
            && self.dismissed.is_none()
            && self.ai_auto_done != Some(c.key)
            && self.ai_found.as_ref().is_none_or(|(k, _)| *k != c.key)
            && c.text.split_whitespace().count() >= AI_AUTO_MIN_WORDS
            && self.debounce.key() == Some(c.key)
            && self.debounce.stable_for(now).is_some_and(|d| d >= AI_AUTO_AFTER)
    }

    /// `ai_auto_due`, and if so the version is marked as gone out: the
    /// capture thread asks for the check once.
    pub fn ai_auto_take(&mut self, now: Instant) -> bool {
        if !self.ai_auto_due(now) {
            return false;
        }
        self.ai_auto_done = self.current.as_ref().map(|c| c.key);
        true
    }

    /// The replacement to copy, for a Copy button (§7.7).
    pub fn copy_text(&self, check_id: u64, index: usize, choice: usize) -> Result<String, Outcome> {
        if check_id != self.shown.check_id {
            return Err(Outcome::Changed);
        }
        if self.shown.mode != FixMode::Copy {
            return Err(Outcome::Blocked);
        }
        let item = self.shown.items.get(index).ok_or(Outcome::Changed)?;
        item.replacements.get(choice).cloned().ok_or(Outcome::Changed)
    }

    /// After a fix (or a refused one): the paragraph changed, or may have.
    /// Drop it and check the field again on the next reading, at once.
    pub fn after_fix(&mut self) {
        self.kept = None;
        self.debounce.check_next();
    }

    /// The app's short name for the shown list.
    pub fn app(&self) -> String {
        self.shown.app.clone()
    }

    /// The check id of the list whose paragraph is kept, if one is.
    pub fn kept_check(&self) -> Option<u64> {
        self.kept.as_ref().map(|_| self.shown.check_id)
    }

    /// The paragraph's suggestions in reading order, from its start (the
    /// owner, 2026-10-10: nearest-the-caret order put an early typo on the
    /// second page, where it looked missed).
    fn check(&mut self, p: &Paragraph, rules: Rules) -> Suggestions {
        let checker = self.checker.get_or_insert_with(Checker::new);
        let t = Instant::now();
        let findings = checker.check_at(&p.text, rules, prefs::get().strictness);
        self.times.push(t.elapsed().as_micros());
        self.lengths.push(p.text.chars().count());
        let chars: Vec<char> = p.text.chars().collect();
        let mut items: Vec<Suggestion> = findings.iter().map(|f| suggestion(&chars, p.start, f)).collect();
        items.sort_by_key(|s| (s.start, s.end));
        Suggestions { items, ..Default::default() }
    }

    /// The list as the island should see it: Apply becomes Away when the
    /// paragraph is no longer kept.
    fn refresh(&mut self, emit: &mut impl FnMut(&Suggestions)) {
        let mode = self.checked_mode;
        let s = Suggestions { check_id: 0, mode, app: self.shown.app.clone(), items: self.shown.items.clone() };
        self.show(s, mode, emit);
    }

    /// Sends the list when it (or its mode) changed. The check id stays the
    /// same for the same list, so a click on it still works after a recheck.
    fn show(&mut self, mut s: Suggestions, mode: FixMode, emit: &mut impl FnMut(&Suggestions)) {
        self.checked_mode = mode;
        s.mode = if mode == FixMode::Apply && self.kept.is_none() { FixMode::Away } else { mode };
        if s.items.is_empty() {
            s.mode = FixMode::View;
            s.app.clear();
        }
        let same_list = s.items == self.shown.items && s.app == self.shown.app;
        s.check_id = if s.items.is_empty() {
            0
        } else if same_list && self.shown.check_id != 0 {
            self.shown.check_id
        } else {
            self.next_id += 1;
            self.next_id
        };
        if s != self.shown {
            emit(&s);
            self.shown = s;
        }
    }

    /// Numbers only, for the periodic capture log line: how many checks, how
    /// long they took, how long the paragraphs were. None if nothing ran.
    pub fn take_report(&mut self) -> Option<String> {
        if self.times.is_empty() {
            return None;
        }
        let mut t = std::mem::take(&mut self.times);
        let mut n = std::mem::take(&mut self.lengths);
        t.sort_unstable();
        n.sort_unstable();
        let ms = |us: u128| us as f64 / 1000.0;
        Some(format!(
            "writing: {} checks, median {:.2} ms, max {:.2} ms; paragraphs median {} chars, max {} chars",
            t.len(),
            ms(t[t.len() / 2]),
            ms(t[t.len() - 1]),
            n[n.len() / 2],
            n[n.len() - 1]
        ))
    }
}

/// Up to CONTEXT_CHARS characters before and after `start..end`, cut only
/// between words: a word the cut would split is left out ("…tomorrow, p"
/// live in Outlook, 2026-10-12).
fn context(chars: &[char], start: usize, end: usize) -> (String, String) {
    let mut from = start.saturating_sub(CONTEXT_CHARS);
    if from > 0 && !chars[from - 1].is_whitespace() {
        // Mid-word: start after the next space, if there is one before the problem.
        match chars[from..start].iter().position(|c| c.is_whitespace()) {
            Some(i) => from += i + 1,
            None => from = start,
        }
    }
    let mut to = (end + CONTEXT_CHARS).min(chars.len());
    if to < chars.len() && !chars[to].is_whitespace() {
        // Mid-word: end at the last space, if there is one after the problem.
        match chars[end..to].iter().rposition(|c| c.is_whitespace()) {
            Some(i) => to = end + i,
            None => to = end,
        }
    }
    (chars[from..start].iter().collect(), chars[end..to].iter().collect())
}

fn suggestion(chars: &[char], offset: usize, f: &Finding) -> Suggestion {
    let s: String = chars[f.start..f.end].iter().collect();
    let replacements = f.replacements.iter().filter(|r| offerable(r)).cloned().collect();
    let (before, after) = context(chars, f.start, f.end);
    Suggestion {
        start: offset + f.start,
        end: offset + f.end,
        problem: s,
        before,
        after,
        kind: f.kind.clone(),
        message: f.message.clone(),
        replacements,
        clear: f.clear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: &str = "Hi Sam,\nWe recieved the payment.\nBest regards,\nAnthony";

    fn run(session: &mut Session, steps: &[(bool, Option<&str>, Option<usize>, u64)]) -> Vec<usize> {
        let t0 = Instant::now();
        let mut emitted = Vec::new();
        for &(same, text, caret, ms) in steps {
            session.observe(same, text, caret, Rules::Full, t0 + Duration::from_millis(ms), |s| emitted.push(s.items.len()));
        }
        emitted
    }

    #[test]
    fn nothing_is_checked_while_the_text_keeps_changing() {
        let mut d = Debouncer::default();
        let t0 = Instant::now();
        for i in 0..20u64 {
            assert_eq!(d.feed(i, t0 + Duration::from_millis(i * 300)), Step::Wait);
        }
    }

    #[test]
    fn one_check_600_ms_after_the_text_stops_changing_and_none_after() {
        assert_eq!(DEBOUNCE, Duration::from_millis(600));
        let mut d = Debouncer::default();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(d.feed(7, at(0)), Step::Wait);
        assert_eq!(d.feed(7, at(300)), Step::Wait);
        assert_eq!(d.feed(7, at(599)), Step::Wait);
        assert_eq!(d.feed(7, at(600)), Step::Check);
        // Unchanged polls after the check: nothing more.
        for ms in [900, 1200, 5000] {
            assert_eq!(d.feed(7, at(ms)), Step::Wait);
        }
        // A change starts the wait again.
        assert_eq!(d.feed(8, at(5300)), Step::Wait);
        assert_eq!(d.feed(8, at(5900)), Step::Check);
    }

    /// No flicker: while the user types, the shown suggestions stay as they
    /// are (nothing is sent), and one update comes after the pause.
    #[test]
    fn typing_continuously_sends_nothing_until_the_pause() {
        let mut s = Session::default();
        let mut field = String::from("We recieved the payment.");
        let mut steps: Vec<(String, u64)> = vec![(field.clone(), 0), (field.clone(), 300), (field.clone(), 600)];
        // Then a word every 300 ms (one reading each) for about 5 s.
        let mut ms = 600;
        for word in " and will deposit it tomorow and send the reciept to the owner by Friday".split_inclusive(' ') {
            field.push_str(word);
            ms += 300;
            steps.push((field.clone(), ms));
        }
        let typing_ends = ms;
        steps.push((field.clone(), ms + 300));
        steps.push((field.clone(), ms + 600));
        let t0 = Instant::now();
        let mut sent: Vec<(u64, usize)> = Vec::new();
        for (text, at) in &steps {
            let caret = Some(text.chars().count());
            s.observe(true, Some(text), caret, Rules::Full, t0 + Duration::from_millis(*at), |x| sent.push((*at, x.items.len())));
        }
        // One update before typing (recieved), none during, one after.
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(sent[0], (600, 1));
        assert!(sent[1].0 > typing_ends && sent[1].1 >= 3, "{sent:?}");
    }

    #[test]
    fn suggestions_come_in_reading_order_wherever_the_caret_is() {
        let field = "Teh lease is recieved. We will deposit it tomorow and send the reciept.";
        let caret = field.find("send").unwrap();
        let mut s = Session::default();
        let mut got = Suggestions::default();
        let t0 = Instant::now();
        for ms in [0, 600] {
            s.observe(true, Some(field), Some(caret), Rules::Full, t0 + Duration::from_millis(ms), |x| got = x.clone());
        }
        let order: Vec<&str> = got.items.iter().map(|i| i.problem.as_str()).collect();
        assert_eq!(order, ["Teh", "recieved", "tomorow", "reciept"]);
    }

    #[test]
    fn a_session_shows_suggestions_after_the_pause_and_another_field_replaces_them() {
        let mut s = Session::default();
        let caret = Some(12);
        let emitted = run(&mut s, &[
            (true, Some(FIELD), caret, 0),
            (true, Some(FIELD), caret, 300),
            (true, Some(FIELD), caret, 1100), // checked: "recieved"
            (true, Some(FIELD), caret, 1400), // unchanged: nothing sent
            (false, Some("Clean text."), Some(3), 1700), // another field: kept until its check
            (true, Some("Clean text."), Some(3), 2000),
            (true, Some("Clean text."), Some(3), 2300), // its check: nothing to fix
        ]);
        assert_eq!(emitted, vec![1, 0]);
        assert!(s.take_report().unwrap().starts_with("writing: 2 checks"));
        assert!(s.take_report().is_none());
    }

    /// The owner's retest: suggestions for the last field stay while they are
    /// in another app (nothing is read there) and are still there, with no gap,
    /// when they come back to it.
    #[test]
    fn suggestions_stay_while_in_another_app_and_come_back_with_the_field() {
        let mut s = Session::default();
        let caret = Some(12);
        let emitted = run(&mut s, &[
            (true, Some(FIELD), caret, 0),
            (true, Some(FIELD), caret, 600), // checked: "recieved"
            (false, None, None, 900),        // a terminal (checker off): kept
            (true, None, None, 5000),
            (false, Some(FIELD), caret, 9000), // back in the same field
            (true, Some(FIELD), caret, 9300),
            (true, Some(FIELD), caret, 9600), // rechecked: the same, nothing sent
        ]);
        assert_eq!(emitted, vec![1]);
    }

    #[test]
    fn a_checked_field_with_nothing_to_check_clears_the_suggestions() {
        let mut s = Session::default();
        let emitted = run(&mut s, &[(true, Some(FIELD), Some(12), 0), (true, Some(FIELD), Some(12), 1000), (true, Some("   "), Some(1), 1300)]);
        assert_eq!(emitted, vec![1, 0]);
        // A caret in the signature: nothing to check, nothing shown.
        let mut s = Session::default();
        let caret = Some(FIELD.find("Anthony").unwrap());
        assert!(run(&mut s, &[(true, Some(FIELD), caret, 0), (true, Some(FIELD), caret, 1500)]).is_empty());
    }

    #[test]
    fn suggestions_carry_field_offsets_and_a_little_context() {
        let mut s = Session::default();
        let mut got = Suggestions::default();
        let t0 = Instant::now();
        for ms in [0, 1000] {
            s.observe(true, Some(FIELD), Some(12), Rules::Full, t0 + Duration::from_millis(ms), |x| got = x.clone());
        }
        let item = &got.items[0];
        let chars: Vec<char> = FIELD.chars().collect();
        assert_eq!(chars[item.start..item.end].iter().collect::<String>(), "recieved");
        assert_eq!((item.problem.as_str(), item.before.as_str(), item.after.as_str()), ("recieved", "We ", " the payment."));
        assert_eq!(item.replacements[0], "received");
    }

    /// Live in Outlook (2026-10-12): the context after "rreceive" ended
    /// "…tomorrow, p". It is cut between words only.
    #[test]
    fn context_is_cut_between_words_only() {
        let chars: Vec<char> = "We rreceive the payment tomorrow, possibleeee".chars().collect();
        let at = |w: &str| {
            let s: String = chars.iter().collect();
            let start = s[..s.find(w).unwrap()].chars().count();
            (start, start + w.chars().count())
        };
        let (s, e) = at("rreceive");
        assert_eq!(context(&chars, s, e), ("We ".into(), " the payment tomorrow,".into()));
        let (s, e) = at("possibleeee");
        assert_eq!(context(&chars, s, e), ("the payment tomorrow, ".into(), "".into()));
        // Short enough: nothing is cut.
        let short: Vec<char> = "We recieved it.".chars().collect();
        assert_eq!(context(&short, 3, 11), ("We ".into(), " it.".into()));
        // One word longer than the window on either side: left out, never split.
        let long: Vec<char> = "Pneumonoultramicroscopicsilicovolcanoconiosis teh Pneumonoultramicroscopicsilicovolcanoconiosis".chars().collect();
        let s = long.iter().position(|&c| c == ' ').unwrap() + 1;
        assert_eq!(context(&long, s, s + 3), ("".into(), "".into()));
    }

    // ── 1b: what is kept for a fix, and when it is dropped (§7.2) ────────────

    /// A field where fixes are typed: one reading per step.
    fn feed(s: &mut Session, same: bool, text: Option<&str>, caret: Option<usize>, mode: FixMode, paused: bool, at: Instant) -> Option<Suggestions> {
        let mut got = None;
        let r = Observed { same, text, caret, rules: Rules::Full, mode, app: "Notepad", paused, sensitive: false };
        s.observe_in(&r, at, |x| got = Some(x.clone()));
        got
    }

    /// Checked in an Apply field: returns the session, its list and the time.
    fn checked() -> (Session, Suggestions, Instant) {
        let mut s = Session::default();
        let t0 = Instant::now();
        feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t0);
        let got = feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t0 + DEBOUNCE).unwrap();
        assert_eq!(got.mode, FixMode::Apply);
        assert!(s.kept.is_some());
        (s, got, t0 + DEBOUNCE)
    }

    #[test]
    fn a_fix_field_keeps_its_paragraph_and_the_job_carries_the_checkers_words() {
        let (s, got, _) = checked();
        assert_eq!(got.app, "Notepad");
        assert_eq!(s.kept.as_ref().unwrap().text, "We recieved the payment.");
        let job = s.job(got.check_id, 0, 0).unwrap();
        assert_eq!((job.paragraph.as_str(), job.start, job.end, job.replacement.as_str()), ("We recieved the payment.", 3, 11, "received"));
        assert_eq!(job.field_start, 8);
    }

    /// A field with three problems in its paragraph, checked in an Apply field.
    fn checked_three() -> (Session, Suggestions) {
        const THREE: &str = "Hi Sam,
We recieved the payment tomorow, posible.
Best regards,
Anthony";
        let mut s = Session::default();
        let t0 = Instant::now();
        feed(&mut s, true, Some(THREE), Some(12), FixMode::Apply, false, t0);
        let got = feed(&mut s, true, Some(THREE), Some(12), FixMode::Apply, false, t0 + DEBOUNCE).unwrap();
        (s, got)
    }

    #[test]
    fn apply_all_goes_from_the_end_backward_and_chains_the_paragraph() {
        let (s, got) = checked_three();
        let problems: Vec<&str> = got.items.iter().map(|i| i.problem.as_str()).collect();
        assert_eq!(problems, ["recieved", "tomorow", "posible"]);
        let (jobs, skipped) = s.jobs_all(got.check_id).unwrap();
        assert_eq!(skipped, 0);
        let did: Vec<(usize, String)> = jobs.iter().map(|j| (j.start, j.replacement.clone())).collect();
        // Last problem first; each start is before the one before it.
        assert!(did.windows(2).all(|w| w[0].0 > w[1].0), "{did:?}");
        assert_eq!(jobs.len(), 3);
        // Each job sees the paragraph the later fixes left: re-verified word for word.
        assert_eq!(jobs[0].paragraph, "We recieved the payment tomorow, posible.");
        let after_first = format!("We recieved the payment tomorow, {}.", jobs[0].replacement);
        assert_eq!(jobs[1].paragraph, after_first);
        assert!(jobs[2].paragraph.contains(&jobs[1].replacement) && jobs[2].paragraph.starts_with("We recieved"));
        // The words are the checker's first choice for each row.
        for j in &jobs {
            let item = got.items.iter().find(|i| i.start - 8 == j.start).unwrap();
            assert_eq!(j.replacement, item.replacements[0]);
        }
    }

    /// Owner, 2026-10-12: Apply all skips a suggestion whose first option
    /// isn't clearly the best, and says how many it left.
    #[test]
    fn apply_all_leaves_unclear_suggestions_to_the_user() {
        let (mut s, got) = checked_three();
        assert!(got.items.iter().all(|i| i.clear), "plain misspellings are clear");
        s.shown.items[1].clear = false; // "tomorow"
        let (jobs, skipped) = s.jobs_all(got.check_id).unwrap();
        assert_eq!(skipped, 1);
        let fixed: Vec<&str> = jobs.iter().map(|j| j.replacement.as_str()).collect();
        assert_eq!(fixed, ["possible", "received"]);
        // The skipped word is left as typed in the paragraph the next fix expects.
        assert_eq!(jobs[1].paragraph, "We recieved the payment tomorow, possible.");
        for item in &mut s.shown.items {
            item.clear = false;
        }
        assert_eq!(s.jobs_all(got.check_id).unwrap_err(), Outcome::Unclear);
    }

    #[test]
    fn apply_all_refuses_a_stale_list_and_works_only_in_apply_fields() {
        let (s, got) = checked_three();
        assert_eq!(s.jobs_all(got.check_id + 1).unwrap_err(), Outcome::Changed);
        let (mut s, got, t) = checked();
        feed(&mut s, false, None, None, FixMode::View, false, t + Duration::from_millis(300));
        assert_eq!(s.jobs_all(got.check_id).unwrap_err(), Outcome::Away);
    }

    // ── 1c: the rewrite ──────────────────────────────────────────────────────

    /// A clean paragraph (no suggestions) in a field of `mode`.
    fn in_field(mode: FixMode, text: &str) -> (Session, Instant) {
        let mut s = Session::default();
        let t = Instant::now();
        feed(&mut s, true, Some(text), Some(3), mode, false, t);
        (s, t)
    }

    #[test]
    fn a_rewrite_sends_the_paragraph_the_caret_is_in_even_with_nothing_to_fix() {
        let text = "Hi Sam,
Thanks for the update on the lease.
Best regards,
Anthony";
        let (mut s, t) = in_field(FixMode::Apply, text);
        assert!(s.kept.is_none(), "no suggestions, nothing kept for a fix");
        // Caret at 3: the greeting line is a paragraph of its own.
        let src = s.rewrite_begin(t).unwrap();
        assert_eq!(src.paragraph, "Hi Sam,");
        assert!(!format!("{src:?}").contains("Sam"));
        // In the body line instead.
        feed(&mut s, true, Some(text), Some(15), FixMode::Apply, false, t);
        let src = s.rewrite_begin(t).unwrap();
        assert_eq!(src.paragraph, "Thanks for the update on the lease.");
        let shown = s.rewrite_answer(src.id, "Thank you for the update on the lease.".into()).unwrap();
        assert_eq!((shown.mode, shown.app.as_str(), shown.offerable), (FixMode::Apply, "Notepad", true));
        let job = s.rewrite_job(src.id).unwrap();
        assert_eq!((job.paragraph.as_str(), job.field_start, job.start, job.end), ("Thanks for the update on the lease.", 8, 0, 35));
        assert_eq!(job.replacement, "Thank you for the update on the lease.");
    }

    #[test]
    fn nothing_to_rewrite_away_from_a_checked_field_or_while_paused() {
        let mut s = Session::default();
        assert_eq!(s.rewrite_begin(Instant::now()).unwrap_err(), Outcome::Away);
        let (mut s, t) = in_field(FixMode::Apply, "We will send it.");
        // Focus went to an app the checker is off for.
        feed(&mut s, false, None, None, FixMode::View, false, t);
        assert_eq!(s.rewrite_begin(t).unwrap_err(), Outcome::Away);
        let (mut s, t) = in_field(FixMode::Apply, "We will send it.");
        let src = s.rewrite_begin(t).unwrap();
        feed(&mut s, true, Some("We will send it."), Some(3), FixMode::Apply, true, t);
        assert!(!s.rewrite_is(src.id), "Pause drops the rewrite");
        assert_eq!(s.rewrite_begin(t).unwrap_err(), Outcome::Away);
    }

    /// Owner, 2026-10-13: a rewrite of a paragraph holding a card number said
    /// "Click in the paragraph…". Capture never reads such a paragraph, so
    /// the session knows only that it was withheld, and says so.
    #[test]
    fn a_rewrite_in_a_withheld_field_is_blocked_not_away() {
        let (mut s, t) = in_field(FixMode::Apply, "We will send it.");
        let withheld = Observed { same: true, text: None, caret: None, rules: Rules::Full, mode: FixMode::Apply, app: "Outlook", paused: false, sensitive: true };
        s.observe_in(&withheld, t, |_| {});
        assert_eq!(s.rewrite_begin(t).unwrap_err(), Outcome::Blocked);
        // Back to an ordinary field without text (another app): Away again.
        feed(&mut s, false, None, None, FixMode::View, false, t);
        assert_eq!(s.rewrite_begin(t).unwrap_err(), Outcome::Away);
        // Paused wins: "Glim is paused" is checked before the paragraph.
        let paused = Observed { paused: true, ..withheld };
        s.observe_in(&paused, t, |_| {});
        assert_eq!(s.rewrite_begin(t).unwrap_err(), Outcome::Away);
    }

    #[test]
    fn only_the_latest_rewrite_counts_and_only_where_fixes_are_typed() {
        let (mut s, t) = in_field(FixMode::Apply, "We will send it.");
        let first = s.rewrite_begin(t).unwrap();
        let second = s.rewrite_begin(t).unwrap();
        assert!(s.rewrite_answer(first.id, "x".into()).is_none(), "an older answer is dropped");
        assert_eq!(s.rewrite_job(first.id).unwrap_err(), Outcome::Changed);
        // Not answered yet: nothing to type.
        assert_eq!(s.rewrite_job(second.id).unwrap_err(), Outcome::Changed);
        s.rewrite_answer(second.id, "We'll send it.".into()).unwrap();
        assert!(s.rewrite_answer(second.id, "again".into()).is_none(), "answered once");
        assert!(s.rewrite_job(second.id).is_ok());
        s.rewrite_clear();
        assert_eq!(s.rewrite_job(second.id).unwrap_err(), Outcome::Changed);
        // A Copy field: the text can be copied, never typed.
        let (mut s, t) = in_field(FixMode::Copy, "We will send it.");
        let src = s.rewrite_begin(t).unwrap();
        s.rewrite_answer(src.id, "We'll send it.".into()).unwrap();
        assert_eq!(s.rewrite_job(src.id).unwrap_err(), Outcome::Blocked);
        assert_eq!(s.rewrite_copy(src.id).unwrap(), "We'll send it.");
        // A view-only app: shown, not typed.
        let (mut s, t) = in_field(FixMode::View, "We will send it.");
        let src = s.rewrite_begin(t).unwrap();
        assert_eq!(s.rewrite_answer(src.id, "We'll send it.".into()).unwrap().mode, FixMode::View);
        assert_eq!(s.rewrite_job(src.id).unwrap_err(), Outcome::Blocked);
    }

    #[test]
    fn an_answer_glim_would_not_type_is_never_typed() {
        for (answer, copy) in [("Line one.
Line two.", true), ("Card 4111 1111 1111 1111 attached.", false)] {
            let (mut s, t) = in_field(FixMode::Apply, "We will send it.");
            let src = s.rewrite_begin(t).unwrap();
            assert!(!s.rewrite_answer(src.id, answer.into()).unwrap().offerable);
            assert_eq!(s.rewrite_job(src.id).unwrap_err(), Outcome::Blocked);
            assert_eq!(s.rewrite_copy(src.id).is_ok(), copy, "{answer}");
        }
    }

    #[test]
    fn a_stale_list_or_a_bad_index_is_refused() {
        let (s, got, _) = checked();
        assert_eq!(s.job(got.check_id + 1, 0, 0).unwrap_err(), Outcome::Changed);
        assert_eq!(s.job(got.check_id, 9, 0).unwrap_err(), Outcome::Changed);
        assert_eq!(s.job(got.check_id, 0, 9).unwrap_err(), Outcome::Changed);
    }

    #[test]
    fn dismissing_drops_the_paragraph_and_hides_the_list_until_the_text_changes() {
        let (mut s, got, t) = checked();
        let mut sent = Vec::new();
        s.dismiss(|x| sent.push(x.items.len()));
        assert_eq!(sent, [0]);
        assert!(s.kept.is_none());
        assert_eq!(s.job(got.check_id, 0, 0).unwrap_err(), Outcome::Changed);
        // The same text: still nothing, however long.
        for ms in [300, 900, 5000] {
            assert!(feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t + Duration::from_millis(ms)).is_none());
        }
        // Changed text: checked again.
        let edited = "Hi Sam,\nWe recieved the payments.\nBest regards,\nAnthony";
        feed(&mut s, true, Some(edited), Some(12), FixMode::Apply, false, t + Duration::from_secs(6));
        let back = feed(&mut s, true, Some(edited), Some(12), FixMode::Apply, false, t + Duration::from_secs(7)).unwrap();
        assert_eq!(back.items.len(), 1);
    }

    #[test]
    fn moving_to_another_field_or_app_drops_the_paragraph_and_the_buttons_say_so() {
        // Another app (nothing read there).
        let (mut s, got, t) = checked();
        let away = feed(&mut s, false, None, None, FixMode::View, false, t + Duration::from_millis(300)).unwrap();
        assert!(s.kept.is_none());
        assert_eq!((away.mode, away.items.len(), away.check_id), (FixMode::Away, 1, got.check_id), "the list stays, view-only");
        assert_eq!(s.job(got.check_id, 0, 0).unwrap_err(), Outcome::Away);
        // Back in the field: checked again, kept again, same list id.
        feed(&mut s, false, Some(FIELD), Some(12), FixMode::Apply, false, t + Duration::from_secs(2));
        let back = feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t + Duration::from_secs(3)).unwrap();
        assert_eq!((back.mode, back.check_id), (FixMode::Apply, got.check_id));
        assert!(s.job(got.check_id, 0, 0).is_ok());
        // Another field that is read: dropped at once.
        let (mut s, _, t) = checked();
        feed(&mut s, false, Some("Another field, no typos."), Some(3), FixMode::Apply, false, t + Duration::from_millis(300));
        assert!(s.kept.is_none());
    }

    #[test]
    fn moving_to_another_paragraph_or_editing_this_one_drops_it() {
        let (mut s, _, t) = checked();
        // The caret in "Hi Sam," (another paragraph).
        feed(&mut s, true, Some(FIELD), Some(2), FixMode::Apply, false, t + Duration::from_millis(300));
        assert!(s.kept.is_none(), "another paragraph");
        let (mut s, _, t) = checked();
        let edited = "Hi Sam,\nWe recieved the payment!\nBest regards,\nAnthony";
        feed(&mut s, true, Some(edited), Some(12), FixMode::Apply, false, t + Duration::from_millis(300));
        assert!(s.kept.is_none(), "edited");
    }

    #[test]
    fn pausing_drops_the_paragraph_and_the_list_and_checks_nothing() {
        let (mut s, _, t) = checked();
        let shown = feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, true, t + Duration::from_millis(300)).unwrap();
        assert!(s.kept.is_none());
        assert!(shown.items.is_empty());
        for ms in [600, 1200, 5000] {
            assert!(feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, true, t + Duration::from_millis(ms)).is_none());
        }
        assert!(s.kept.is_none());
    }

    #[test]
    fn the_paragraph_is_kept_ten_minutes_at_most() {
        assert_eq!(KEEP_FOR, Duration::from_secs(600));
        let (mut s, _, t) = checked();
        feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t + KEEP_FOR - Duration::from_secs(1));
        assert!(s.kept.is_some());
        feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t + KEEP_FOR);
        assert!(s.kept.is_none(), "dropped at 10 minutes");
        // Still in the field: checked (and kept) again after the usual wait.
        feed(&mut s, true, Some(FIELD), Some(12), FixMode::Apply, false, t + KEEP_FOR + DEBOUNCE);
        assert!(s.kept.is_some());
    }

    #[test]
    fn a_paragraph_holding_a_card_number_is_never_kept_or_shown() {
        // A reading that reaches the checker has passed capture's scan; this
        // is the second scan, before keeping (owner, 2026-10-11).
        let field = "Charge 4111 1111 1111 1111 for the recieved order.";
        let mut s = Session::default();
        let t0 = Instant::now();
        feed(&mut s, true, Some(field), Some(5), FixMode::Apply, false, t0);
        let got = feed(&mut s, true, Some(field), Some(5), FixMode::Apply, false, t0 + DEBOUNCE);
        assert!(s.kept.is_none());
        assert!(got.is_none_or(|g| g.items.is_empty()));
    }

    #[test]
    fn view_only_and_copy_fields_keep_nothing() {
        for mode in [FixMode::View, FixMode::Copy] {
            let mut s = Session::default();
            let t0 = Instant::now();
            feed(&mut s, true, Some(FIELD), Some(12), mode, false, t0);
            let got = feed(&mut s, true, Some(FIELD), Some(12), mode, false, t0 + DEBOUNCE).unwrap();
            assert_eq!((got.mode, got.items.len()), (mode, 1));
            assert!(s.kept.is_none(), "{mode:?}");
            assert_eq!(s.job(got.check_id, 0, 0).unwrap_err(), Outcome::Blocked);
            let copied = s.copy_text(got.check_id, 0, 0);
            if mode == FixMode::Copy {
                assert_eq!(copied.unwrap(), "received");
            } else {
                assert_eq!(copied.unwrap_err(), Outcome::Blocked);
            }
        }
    }

    #[test]
    fn after_a_fix_the_field_is_checked_again_at_once() {
        let (mut s, _, t) = checked();
        s.after_fix();
        assert!(s.kept.is_none());
        let fixed = "Hi Sam,\nWe received the payment.\nBest regards,\nAnthony";
        let got = feed(&mut s, true, Some(fixed), Some(12), FixMode::Apply, false, t + Duration::from_millis(10)).unwrap();
        assert!(got.items.is_empty());
    }

    #[test]
    fn only_printable_replacements_without_sensitive_numbers_are_offered() {
        assert!(offerable("received") && offerable("") && offerable("it's") && offerable("Unit 4B"));
        for bad in ["a\nb", "a\rb", "tab\there", "\u{7}", "4111 1111 1111 1111", "123-45-6789"] {
            assert!(!offerable(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_fix_modes_follow_the_app_and_the_pattern() {
        assert_eq!(fix_mode("WINWORD.EXE", "TextPattern2", false), FixMode::Apply);
        assert_eq!(fix_mode("Notepad.exe", "TextPattern", false), FixMode::Apply);
        assert_eq!(fix_mode("chrome.exe", "TextPattern2", false), FixMode::Apply, "Gmail");
        assert_eq!(fix_mode("chrome.exe", "ValuePattern", false), FixMode::Copy);
        assert_eq!(fix_mode("slack.exe", "TextPattern2", false), FixMode::View);
        assert_eq!(fix_mode("msedge.exe", "TextPattern2", false), FixMode::View);
        assert_eq!(fix_mode("slack.exe", "ValuePattern", false), FixMode::View, "chat apps: view-only, no Copy");
        assert_eq!(fix_mode("Notepad.exe", "none", false), FixMode::View);
        // A web chat app (claude.ai in Chrome): view-only, no Copy either.
        assert_eq!(fix_mode("chrome.exe", "TextPattern2", true), FixMode::View);
        assert_eq!(fix_mode("chrome.exe", "ValuePattern", true), FixMode::View);
    }

    /// The island can only say which fix was clicked, never what to type
    /// (§7.1 rule 2): the commands take numbers and nothing else.
    #[test]
    fn the_islands_commands_carry_ids_only() {
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        for cmd in ["fn writing_fix(", "fn writing_copy("] {
            let at = lib.find(cmd).unwrap_or_else(|| panic!("{cmd}"));
            let sig = &lib[at..at + lib[at..].find('{').unwrap()];
            assert_eq!(sig.trim_end(), format!("{cmd}check_id: u64, index: usize, choice: usize)"), "{sig}");
        }
        assert!(lib.contains("fn writing_fix_all(check_id: u64) {"), "Apply all: the list id only");
        // The rewrite (1c): a tone id or a rewrite id, never text.
        assert!(lib.contains("fn writing_rewrite(tone: String) {"));
        assert!(lib.contains("let Some(tone) = ai::claude::Tone::from_id(&tone) else { return };"));
        for cmd in ["fn writing_rewrite_apply(id: u64) {", "fn writing_rewrite_copy(id: u64) {", "fn writing_rewrite_dismiss() {"] {
            assert!(lib.contains(cmd), "{cmd}");
        }
        assert!(lib.contains("fn writing_dismiss() {"));
        // Pause is Rust's own since 1d (pause.rs): the island has no command for it.
        assert!(!lib.contains("fn writing_set_paused("));
        // The one-time prompt's answer: the app's exe and a word, nothing else.
        assert!(lib.contains("fn writing_ask_answer(app: AppHandle, shared: State<Shared>, exe: String, answer: String) {"));
    }

    #[test]
    fn a_job_never_shows_its_words() {
        let j = Job { paragraph: "SENTINEL-p".into(), field_start: 0, start: 0, end: 1, replacement: "SENTINEL-r".into(), app: "Word".into() };
        assert!(!format!("{j:?} {j:#?}").contains("SENTINEL"));
    }

    #[test]
    fn formatting_suggestions_never_shows_their_words() {
        let s = Suggestions { check_id: 1, mode: FixMode::Apply, app: "Word".into(), items: vec![Suggestion { start: 0, end: 1, problem: "SENTINEL-p".into(), before: "SENTINEL-b".into(), after: "SENTINEL-a".into(), kind: "Spelling".into(), message: "SENTINEL-m".into(), replacements: vec!["SENTINEL-r".into()], clear: true }] };
        let shown = format!("{s:?} {:?} {:#?}", s.items[0], s);
        assert!(!shown.contains("SENTINEL"), "{shown}");
    }

    #[test]
    fn nothing_in_the_writing_code_logs_or_writes_text() {
        // Every logging or file-writing call in the writing module and the
        // capture thread: none may mention text, paragraphs or suggestions.
        let files = [
            ("writing/mod.rs", include_str!("mod.rs")),
            ("writing/text.rs", include_str!("text.rs")),
            ("writing/checker.rs", include_str!("checker.rs")),
            ("writing/apps.rs", include_str!("apps.rs")),
            ("writing/apply.rs", include_str!("apply.rs")),
            ("capture/uia.rs", include_str!("../capture/uia.rs")),
        ];
        for (file, src) in files {
            let code = src.split("#[cfg(test)]").next().unwrap();
            for (i, line) in code.lines().enumerate() {
                let writes = ["log::line", "eprintln!", "println!", "writeln!", "std::fs::write", "dbg!"]
                    .iter()
                    .any(|w| line.contains(w));
                if writes {
                    for banned in [".text", "paragraph", "problem", "message", "replacement", "before", "after", "items", "excerpt", "title"] {
                        assert!(!line.contains(banned), "{file}:{} writes {banned}: {}", i + 1, line.trim());
                    }
                }
            }
        }
    }

    // ── "Check with AI" (docs/phase1-plan.md §15.1) ─────────────────────────

    const AGREE: &str = "Alex and I was at the unit, and we recieved the keys.";

    fn issue(text: &str, quote: &str, replacement: &str) -> crate::ai::check::Issue {
        let chars: Vec<char> = text.chars().collect();
        let q: Vec<char> = quote.chars().collect();
        let start = (0..chars.len()).find(|&i| chars[i..].starts_with(&q)).unwrap();
        crate::ai::check::Issue { start, end: start + q.len(), message: "AI says so.".into(), replacement: replacement.into() }
    }

    /// Checked by Harper in an Apply field (one finding: "recieved").
    fn harper_checked(text: &str) -> (Session, Instant) {
        let mut s = Session::default();
        let t0 = Instant::now();
        feed(&mut s, true, Some(text), Some(3), FixMode::Apply, false, t0);
        feed(&mut s, true, Some(text), Some(3), FixMode::Apply, false, t0 + DEBOUNCE).unwrap();
        (s, t0 + DEBOUNCE)
    }

    #[test]
    fn the_ais_suggestions_join_harpers_marked_and_fixable() {
        let (mut s, _) = harper_checked(AGREE);
        assert_eq!(s.shown.items.iter().map(|i| i.problem.as_str()).collect::<Vec<_>>(), ["recieved"]);
        let src = s.ai_begin(false).unwrap();
        assert_eq!(src.paragraph, AGREE);
        assert_eq!(s.ai_pending(src.id), Some(false));
        // One new problem, one on Harper's span (left out).
        let issues = [issue(AGREE, "Alex and I was", "Alex and I were"), issue(AGREE, "recieved", "got")];
        let mut got = None;
        assert_eq!(s.ai_answer(src.id, &issues, |x| got = Some(x.clone())), Some(1));
        let list = got.unwrap();
        assert_eq!(list.mode, FixMode::Apply);
        assert_eq!(list.items.iter().map(|i| (i.problem.as_str(), i.kind.as_str())).collect::<Vec<_>>(), [("Alex and I was", AI_KIND), ("recieved", "Spelling")]);
        assert!(!list.items[0].clear, "Apply all leaves the AI's to the user");
        // Applying it is the usual path, word for word.
        let job = s.job(list.check_id, 0, 0).unwrap();
        assert_eq!((job.start, job.end, job.replacement.as_str()), (0, 14, "Alex and I were"));
        assert_eq!(s.ai_pending(src.id), None, "answered");
    }

    #[test]
    fn the_ai_finds_something_when_harper_found_nothing() {
        let text = "Alex and I was at the unit on Monday.";
        let (mut s, _) = in_field(FixMode::Apply, text);
        assert!(s.shown.items.is_empty());
        let src = s.ai_begin(false).unwrap();
        let mut got = None;
        assert_eq!(s.ai_answer(src.id, &[issue(text, "Alex and I was", "Alex and I were")], |x| got = Some(x.clone())), Some(1));
        let list = got.unwrap();
        assert_eq!((list.mode, list.items.len()), (FixMode::Apply, 1));
        assert!(s.kept.is_some(), "kept for the fix");
        // Nothing found: nothing changes, and it says 0.
        let src = s.ai_begin(false).unwrap();
        assert_eq!(s.ai_answer(src.id, &[], |_| {}), Some(0));
    }

    #[test]
    fn an_answer_for_an_edited_paragraph_or_an_older_check_is_dropped() {
        let (mut s, t) = harper_checked(AGREE);
        let src = s.ai_begin(false).unwrap();
        let edited = "Alex and I was at the unit, and we recieved the key.";
        feed(&mut s, true, Some(edited), Some(3), FixMode::Apply, false, t);
        let mut emitted = false;
        assert_eq!(s.ai_answer(src.id, &[issue(AGREE, "Alex and I was", "x")], |_| emitted = true), None);
        assert!(!emitted);
        // Two clicks: only the latest counts.
        let (mut s, _) = harper_checked(AGREE);
        let first = s.ai_begin(false).unwrap();
        let second = s.ai_begin(false).unwrap();
        assert_eq!(s.ai_answer(first.id, &[], |_| {}), None);
        assert_eq!(s.ai_answer(second.id, &[], |_| {}), Some(0));
    }

    #[test]
    fn the_ais_suggestions_stay_with_their_paragraph_version_only() {
        let (mut s, t) = harper_checked(AGREE);
        let src = s.ai_begin(false).unwrap();
        s.ai_answer(src.id, &[issue(AGREE, "Alex and I was", "Alex and I were")], |_| {});
        // Checked again (after a fix elsewhere): still there.
        s.after_fix();
        let again = feed(&mut s, true, Some(AGREE), Some(3), FixMode::Apply, false, t);
        let items = again.map(|g| g.items).unwrap_or_else(|| s.shown.items.clone());
        assert!(items.iter().any(|i| i.kind == AI_KIND), "{items:?}");
        // Edited: gone with the old version.
        let edited = "Alex and I were at the unit, and we recieved the keys.";
        feed(&mut s, true, Some(edited), Some(3), FixMode::Apply, false, t);
        let got = feed(&mut s, true, Some(edited), Some(3), FixMode::Apply, false, t + DEBOUNCE).unwrap();
        assert!(got.items.iter().all(|i| i.kind != AI_KIND));
    }

    #[test]
    fn the_automatic_check_waits_for_a_pause_and_goes_once_per_version() {
        let mut s = Session::default();
        let t0 = Instant::now();
        feed(&mut s, true, Some(AGREE), Some(3), FixMode::Apply, false, t0);
        assert!(!s.ai_auto_take(t0 + Duration::from_millis(1900)), "still typing");
        assert!(s.ai_auto_take(t0 + AI_AUTO_AFTER));
        assert!(!s.ai_auto_take(t0 + Duration::from_secs(10)), "once per version");
        let src = s.ai_begin(true).unwrap();
        assert_eq!(s.ai_pending(src.id), Some(true));
        // A new version waits again.
        let t1 = t0 + Duration::from_secs(20);
        let edited = "Alex and I were at the unit, and we recieved the keys.";
        s.ai_clear(src.id);
        feed(&mut s, true, Some(edited), Some(3), FixMode::Apply, false, t1);
        assert!(!s.ai_auto_take(t1 + Duration::from_secs(1)));
        assert!(s.ai_auto_take(t1 + AI_AUTO_AFTER));
        // Too short to bother.
        let (mut s, t) = in_field(FixMode::Apply, "Thanks!");
        assert!(!s.ai_auto_take(t + Duration::from_secs(5)));
    }

    #[test]
    fn nothing_goes_to_the_ai_from_a_withheld_field_or_while_paused() {
        let (mut s, t) = in_field(FixMode::Apply, AGREE);
        let withheld = Observed { same: true, text: None, caret: None, rules: Rules::Full, mode: FixMode::Apply, app: "Outlook", paused: false, sensitive: true };
        s.observe_in(&withheld, t, |_| {});
        assert_eq!(s.ai_begin(false).unwrap_err(), Outcome::Blocked);
        assert!(!s.ai_auto_take(t + Duration::from_secs(5)));
        let (mut s, t) = in_field(FixMode::Apply, AGREE);
        let src = s.ai_begin(false).unwrap();
        feed(&mut s, true, Some(AGREE), Some(3), FixMode::Apply, true, t);
        assert_eq!(s.ai_pending(src.id), None, "Pause drops it");
        assert_eq!(s.ai_begin(false).unwrap_err(), Outcome::Away);
    }
}
