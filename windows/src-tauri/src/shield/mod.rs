// The privacy shield (owner, 2026-11-03): before any of the user's text goes
// to Claude (the rewrite, "Check with AI", dictation's clean-up, Ask Glim,
// and the live revisions built on it), emails, phone numbers, street
// addresses, unit numbers and people's names are swapped, on this PC, for
// numbered placeholders ([PERSON1], [EMAIL1], [PHONE1], [ADDRESS1],
// [UNIT1]). The answer gets the real values back before anything is shown or
// typed.
//
// The answer must carry the placeholders back exactly. For a text Claude
// rewrites (the rewrite, a revision, dictated text: `Check::Whole`), every
// placeholder sent must come back, none more often than it was sent, and
// none changed; otherwise the answer is discarded. For an answer that is not
// the whole text (a "Check with AI" issue, an Ask Glim action:
// `Check::Partial`), a placeholder may be left out, but none may be changed
// or made up.
//
// The map from placeholder to real value lives in a `Masked`, in memory, for
// one request; it is never logged, stored or sent, and its Debug shows
// counts only. Names come from Settings → Privacy (the user's list), the
// recipients remembered from "Draft email" cards, and classic Outlook's
// address book while Outlook runs (address_book.rs); all in memory.
//
// The never-send filter (never_send.rs) is the other half: a request with a
// card, ID or bank number, an SSN, a date of birth or a labelled income or
// benefit amount is not sent at all (claude::send).

pub mod detect;
pub mod never_send;
#[cfg(windows)]
pub mod address_book;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// The sentence every system prompt carries (ai/claude.rs, check.rs,
/// dictate.rs; a test checks each).
pub const PROMPT_NOTE: &str = "Bracketed placeholders like [PERSON1] or [PHONE1] stand for private details: copy each one exactly, never change, add or drop one.";

/// What a placeholder stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Person,
    Email,
    Phone,
    Address,
    Unit,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Person, Kind::Email, Kind::Phone, Kind::Address, Kind::Unit];

    /// The placeholder's word: [PERSON1].
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Person => "PERSON",
            Kind::Email => "EMAIL",
            Kind::Phone => "PHONE",
            Kind::Address => "ADDRESS",
            Kind::Unit => "UNIT",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One value swapped out: its placeholder, the real text, and how many
/// times the placeholder is in the text sent.
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    placeholder: String,
    original: String,
    sent: usize,
}

/// The text as sent, and (in memory only) what each placeholder stands for.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Masked {
    pub text: String,
    entries: Vec<Entry>,
    /// Placeholder-looking words already in the user's text, as written.
    foreign: Vec<String>,
}

impl std::fmt::Debug for Masked {
    /// Counts only: never the text, a value or a placeholder's meaning.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Masked").field("chars", &self.text.chars().count()).field("counts", &self.counts_line()).finish()
    }
}

impl Masked {
    /// Nothing swapped (the text as it was; the shield is off).
    pub fn plain(text: &str) -> Masked {
        let foreign = placeholder_like(text).into_iter().map(|(s, e)| text[s..e].to_string()).collect();
        Masked { text: text.to_string(), entries: Vec::new(), foreign }
    }

    /// How many distinct values of each kind were swapped, in Kind::ALL's order.
    pub fn counts(&self) -> [usize; 5] {
        let mut out = [0; 5];
        for e in &self.entries {
            out[e.kind.index()] += 1;
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// "person=2 email=1 phone=0 address=0 unit=0": for the log.
    pub fn counts_line(&self) -> String {
        let c = self.counts();
        Kind::ALL.iter().map(|k| format!("{}={}", k.tag().to_lowercase(), c[k.index()])).collect::<Vec<_>>().join(" ")
    }

    /// The placeholders sent, in order (tests and the prompt builders).
    pub fn placeholders(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.placeholder.as_str()).collect()
    }
}

/// How strictly an answer must carry the placeholders back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// A rewritten text: every placeholder back, none more often than sent.
    Whole,
    /// Part of an answer (an issue, an action): only none changed or made up.
    Partial,
}

/// Why an answer (or a text) can't go through the shield. Codes only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// A placeholder sent is not in the answer.
    Missing,
    /// A placeholder is in the answer more often than it was sent.
    Duplicated,
    /// Something that looks like a placeholder but isn't one sent.
    Altered,
    /// Several texts couldn't be masked as one (mask_many): not sent.
    Collision,
}

impl Mismatch {
    pub fn code(self) -> &'static str {
        match self {
            Mismatch::Missing => "shield-missing",
            Mismatch::Duplicated => "shield-duplicated",
            Mismatch::Altered => "shield-altered",
            Mismatch::Collision => "shield-collision",
        }
    }
}

/// The names to look for, compiled: whole names (lowercase words) and the
/// single words of any name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameSet {
    pub(crate) full: Vec<Vec<String>>,
    pub(crate) words: HashSet<String>,
}

impl NameSet {
    /// From name entries as written anywhere ("Maria Lopez", "Lopez, Maria",
    /// "Maria Lopez (Leasing)", "maria"). Addresses and bare stop words are
    /// left out.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a str>) -> NameSet {
        let mut set = NameSet::default();
        for entry in entries {
            let (words, reversed) = detect::name_words(entry);
            if words.is_empty() {
                continue;
            }
            for w in &words {
                if !detect::is_stop_word(w) {
                    set.words.insert(w.clone());
                }
            }
            // A whole name, unless every word of it is a common one ("Leasing Office").
            if words.len() >= 2 && !words.iter().all(|w| detect::is_stop_word(w)) {
                if !set.full.contains(&words) {
                    set.full.push(words.clone());
                }
                if let Some(r) = reversed.filter(|r| !set.full.contains(r)) {
                    set.full.push(r);
                }
            }
        }
        // Longest first, so "Maria Lopez Garcia" wins over "Maria Lopez".
        set.full.sort_by_key(|w| std::cmp::Reverse(w.len()));
        set
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// Swaps every value `detect` finds in `text` for its placeholder. The same
/// text gets the same placeholder each time it appears. Placeholder-looking
/// words the user wrote themselves ("Unit1") are left as they are, may come
/// back as often as they were written, and no placeholder Glim makes is
/// spelled like one of them.
pub fn mask_with(text: &str, names: &NameSet) -> Result<Masked, Mismatch> {
    let foreign: Vec<String> = placeholder_like(text).into_iter().map(|(s, e)| text[s..e].to_string()).collect();
    let spans = detect::find(text, names);
    let mut out = String::with_capacity(text.len());
    let mut entries: Vec<Entry> = Vec::new();
    let mut next = [1usize; 5];
    let mut at = 0;
    for s in spans {
        if s.start < at {
            continue;
        }
        out.push_str(&text[at..s.start]);
        let original = &text[s.start..s.end];
        let i = match entries.iter().position(|e| e.kind == s.kind && e.original == original) {
            Some(i) => i,
            None => {
                let mut placeholder = format!("[{}{}]", s.kind.tag(), next[s.kind.index()]);
                while foreign.contains(&placeholder) {
                    next[s.kind.index()] += 1;
                    placeholder = format!("[{}{}]", s.kind.tag(), next[s.kind.index()]);
                }
                next[s.kind.index()] += 1;
                entries.push(Entry { kind: s.kind, placeholder, original: original.to_string(), sent: 0 });
                entries.len() - 1
            }
        };
        entries[i].sent += 1;
        out.push_str(&entries[i].placeholder);
        at = s.end;
    }
    out.push_str(&text[at..]);
    Ok(Masked { text: out, entries, foreign })
}

/// Several texts of one request, masked with one map (an Ask request's
/// transcript and its earlier question, a draft and the change asked).
pub fn mask_many(texts: &[&str], names: &NameSet) -> Result<(Vec<String>, Masked), Mismatch> {
    // A separator no text holds and no detector reads across.
    const SEP: &str = "\u{1F}\n\u{1F}";
    if texts.iter().any(|t| t.contains('\u{1F}')) {
        return Err(Mismatch::Collision);
    }
    let joined = texts.join(SEP);
    let masked = mask_with(&joined, names)?;
    let parts: Vec<String> = masked.text.split(SEP).map(str::to_string).collect();
    if parts.len() != texts.len() {
        return Err(Mismatch::Collision);
    }
    Ok((parts, masked))
}

/// A placeholder-looking token in an answer: where, and which entry it is
/// (None: it isn't one that was sent).
struct Found {
    start: usize,
    end: usize,
    entry: Option<usize>,
}

/// Every placeholder-looking token in `text`: [PERSON1] and its altered
/// forms ([person 1], [PERSON], PERSON1, Person_1, EMAIL-2 …).
fn placeholder_like(text: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    // Bracketed: anything in [ ] whose letters are a tag.
    for (i, c) in text.char_indices() {
        if c != '[' || out.last().is_some_and(|&(_, e)| i < e) {
            continue;
        }
        let close = text[i + 1..].char_indices().take(24).find(|(_, c)| *c == ']' || *c == '[');
        if let Some((len, ']')) = close {
            if is_tag_like(&text[i + 1..i + 1 + len]) {
                out.push((i, i + 1 + len + 1));
            }
        }
    }
    // Bare: a tag's letters (any case) and a number, as one word (PERSON1),
    // or joined by _ or - (PERSON_1, EMAIL-2).
    for (start, end) in alnum_runs(text) {
        if out.iter().any(|&(s, e)| start < e && s < end) {
            continue;
        }
        let w = &text[start..end];
        let letters = w.chars().take_while(char::is_ascii_alphabetic).count();
        let (tag, rest) = w.split_at(letters);
        if !Kind::ALL.iter().any(|k| tag.eq_ignore_ascii_case(k.tag())) {
            continue;
        }
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            out.push((start, end));
        } else if rest.is_empty() {
            let after = &text[end..];
            let joined = after.starts_with(['_', '-']) && after[1..].starts_with(|c: char| c.is_ascii_digit());
            if joined {
                let digits = after[1..].chars().take_while(char::is_ascii_digit).count();
                out.push((start, end + 1 + digits));
            }
        }
    }
    out.sort();
    out
}

/// Runs of ASCII letters and digits (byte offsets).
fn alnum_runs(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_ascii_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Whether the inside of [ ] reads as a placeholder: a tag's letters, any
/// case, with or without a number, spaces, _ - or #.
fn is_tag_like(inner: &str) -> bool {
    if inner.chars().any(|c| !c.is_ascii_alphanumeric() && !matches!(c, ' ' | '_' | '-' | '#')) {
        return false;
    }
    let letters: String = inner.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    Kind::ALL.iter().any(|k| letters.eq_ignore_ascii_case(k.tag()))
}

fn find_placeholders(answer: &str, m: &Masked) -> Vec<Found> {
    placeholder_like(answer)
        .into_iter()
        .map(|(start, end)| Found { start, end, entry: m.entries.iter().position(|e| e.placeholder == answer[start..end]) })
        .collect()
}

/// Puts the real values back into the answer's texts (one request's: an
/// Ask action's fields, a draft's subject and body), checked as `check`
/// says, counted over all of them.
pub fn unmask_many(answers: &[&str], m: &Masked, check: Check) -> Result<Vec<String>, Mismatch> {
    let mut seen = vec![0usize; m.entries.len()];
    let mut foreign_left = m.foreign.clone();
    let mut found_all = Vec::new();
    for a in answers {
        let mut found = find_placeholders(a, m);
        let mut ours = Vec::new();
        for f in found.drain(..) {
            match f.entry {
                Some(i) => {
                    seen[i] += 1;
                    ours.push(f);
                }
                // The user's own "Unit1", no more often than they wrote it.
                None => match foreign_left.iter().position(|w| *w == a[f.start..f.end]) {
                    Some(k) => {
                        foreign_left.swap_remove(k);
                    }
                    None => return Err(Mismatch::Altered),
                },
            }
        }
        found_all.push(ours);
    }
    if check == Check::Whole {
        for (e, n) in m.entries.iter().zip(&seen) {
            if *n == 0 {
                return Err(Mismatch::Missing);
            }
            if *n > e.sent {
                return Err(Mismatch::Duplicated);
            }
        }
    }
    Ok(answers
        .iter()
        .zip(found_all)
        .map(|(a, found)| {
            let mut out = String::with_capacity(a.len());
            let mut at = 0;
            for f in found {
                out.push_str(&a[at..f.start]);
                if let Some(i) = f.entry {
                    out.push_str(&m.entries[i].original);
                }
                at = f.end;
            }
            out.push_str(&a[at..]);
            out
        })
        .collect())
}

/// One answer text: `unmask_many` with one.
pub fn unmask(answer: &str, m: &Masked, check: Check) -> Result<String, Mismatch> {
    unmask_many(&[answer], m, check).map(|mut v| v.remove(0))
}

/// `Check::Partial`, and also: every placeholder in `kept` (the masked text
/// that was edited, when the request carried context too) comes back, none
/// more often than it is there. A placeholder only in the context may be
/// left out.
pub fn unmask_keeping(answer: &str, m: &Masked, kept: &str) -> Result<String, Mismatch> {
    let back = unmask(answer, m, Check::Partial)?;
    for e in &m.entries {
        let need = kept.matches(e.placeholder.as_str()).count();
        if need == 0 {
            continue;
        }
        let got = answer.matches(e.placeholder.as_str()).count();
        if got == 0 {
            return Err(Mismatch::Missing);
        }
        if got > need {
            return Err(Mismatch::Duplicated);
        }
    }
    Ok(back)
}

// ── The app's state: on/off and the names (lib.rs sets them) ───────────────

/// Settings → Privacy → "Privacy shield": on unless turned off.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Where the names come from, each kept apart so one can change alone.
#[derive(Default)]
struct Sources {
    own: Vec<String>,
    recipients: Vec<String>,
    book: Vec<String>,
    compiled: NameSet,
}

static SOURCES: RwLock<Option<Sources>> = RwLock::new(None);

fn update(f: impl FnOnce(&mut Sources)) {
    let mut guard = SOURCES.write().unwrap_or_else(|e| e.into_inner());
    let s = guard.get_or_insert_with(Sources::default);
    f(s);
    let all = s.own.iter().chain(&s.recipients).chain(&s.book).map(String::as_str);
    s.compiled = NameSet::from_entries(all);
}

/// From the settings (at launch and on every save): the switch, the user's
/// names list and the remembered recipients' names.
pub fn apply_settings(s: &crate::settings::Settings) {
    ENABLED.store(s.privacy_shield, Ordering::Relaxed);
    let own = s.shield_names.clone();
    let recipients: Vec<String> = s.draft_recipients.keys().cloned().collect();
    update(|src| {
        src.own = own;
        src.recipients = recipients;
    });
}

/// The names read from Outlook's address book (address_book.rs).
pub fn set_address_book(names: Vec<String>) {
    update(|src| src.book = names);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// How many name words the shield knows (the log's count).
pub fn name_count() -> usize {
    SOURCES.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(0, |s| s.compiled.len())
}

/// `text`, masked with the app's names; as it is when the shield is off.
pub fn mask(text: &str) -> Result<Masked, Mismatch> {
    if !enabled() {
        return Ok(Masked::plain(text));
    }
    let guard = SOURCES.read().unwrap_or_else(|e| e.into_inner());
    let empty = NameSet::default();
    mask_with(text, guard.as_ref().map_or(&empty, |s| &s.compiled))
}

/// Several texts of one request, masked with the app's names (`mask_many`).
pub fn mask_texts(texts: &[&str]) -> Result<(Vec<String>, Masked), Mismatch> {
    if !enabled() {
        let joined = texts.join("\n");
        return Ok((texts.iter().map(|t| t.to_string()).collect(), Masked::plain(&joined)));
    }
    let guard = SOURCES.read().unwrap_or_else(|e| e.into_inner());
    let empty = NameSet::default();
    mask_many(texts, guard.as_ref().map_or(&empty, |s| &s.compiled))
}

/// Tests that switch the shield or change its names hold this, so tests
/// that rely on the default (on, no names) don't race them.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;
