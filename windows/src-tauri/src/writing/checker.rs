// The instant checker: Harper (harper-core), in-process, American English.
// See docs/phase1-plan.md §3 for what it catches and what it misses.

use std::collections::BTreeMap;

use harper_core::linting::{LintGroup, LintKind, Suggestion as Edit};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

/// One problem in the checked paragraph, offsets in characters from the
/// paragraph's start.
#[derive(Clone, PartialEq)]
pub struct Finding {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub message: String,
    /// Up to MAX_REPLACEMENTS; "" means "remove it".
    pub replacements: Vec<String>,
    /// The first replacement is clearly the best: "Apply all" takes only
    /// these (`rank`).
    pub clear: bool,
}

impl std::fmt::Debug for Finding {
    /// Only offsets and the kind: replacements are words from the user's text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finding").field("start", &self.start).field("end", &self.end).field("kind", &self.kind).finish()
    }
}

pub const MAX_REPLACEMENTS: usize = 3;

/// Rules switched off: wrong on everyday property-management text (tried
/// 2026-10-10). "unit 4B" → "Did you mean bytes?"; "need to discus" → a
/// replacement of "the".
pub const DISABLED_RULES: [&str; 2] = ["ExpandMemoryShorthands", "NeedToNoun"];

/// Harper's initialism rules (FYI, ASAP …) ask to spell them out: a style
/// preference, not an error. They share this message.
const INITIALISM_MESSAGE: &str = "Try expanding this initialism";

/// Which rules apply: chat apps get a relaxed set (the owner, 2026-10-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rules {
    /// Email, documents, browsers, Notepad: everything.
    Full,
    /// Chat apps: no capitalization lints (sentence starts, "i", the
    /// all-caps "canonical spelling" of "u", "btw" ...) and no findings on
    /// common chat words. Real misspellings are still found.
    Chat,
}

/// Chat words never flagged in chat apps, whatever the rule (spelling,
/// "ok" -> "okay", "tho" -> "though", the canonical-spelling lints).
pub const CHAT_WORDS: &[&str] = &[
    "u", "ur", "r", "ya", "yall", "y'all", "tmrw", "tmr", "tbh", "lol", "lmao", "lmfao", "rofl", "idk", "idc",
    "brb", "btw", "thx", "thnx", "ty", "tysm", "np", "pls", "plz", "sry", "rn", "omg", "lmk", "imo", "imho",
    "dm", "asap", "fyi", "gonna", "wanna", "gotta", "kinda", "sorta", "tho", "ok", "k", "kk", "ikr", "nvm",
    "smh", "fr", "ngl", "irl", "bc", "cuz", "b4", "ppl", "msg", "tbd", "eta", "wfh", "ooo", "hbu", "wyd",
    "ttyl", "gn", "gm", "haha", "hahaha", "jk", "afaik", "bday", "convo", "pic", "pics", "yep", "yup", "nope",
    "hmu", "ofc", "i'm", "i'll", "i've", "i'd", "im", "ive",
];

fn chat_word(problem: &str) -> bool {
    let w = problem.trim().to_lowercase().replace('\u{2019}', "'");
    CHAT_WORDS.contains(&w.as_str())
}

/// Settings → Writing → Strictness (1d, docs/phase1-plan.md §14.4): one
/// global level. A level only takes findings away; chat rules still apply on
/// top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strictness {
    /// Spelling and repeated words ("the the").
    Spelling,
    /// Spelling, plus grammar, agreement, word choice and punctuation.
    Standard,
    /// Everything Harper's curated set offers (today's behaviour).
    #[default]
    Full,
}

impl Strictness {
    /// From `checkerStrictness` in settings.json; anything else is Full.
    pub fn from_id(id: &str) -> Strictness {
        match id {
            "spelling" => Strictness::Spelling,
            "standard" => Strictness::Standard,
            _ => Strictness::Full,
        }
    }
}

/// The lowest level that shows a kind of finding. Every kind Harper 2.11.0
/// has is listed with no catch-all, so a Harper update that adds one fails
/// to build here instead of slipping in unsorted.
pub fn level_of(kind: LintKind) -> Strictness {
    match kind {
        LintKind::Spelling | LintKind::Repetition => Strictness::Spelling,
        LintKind::Agreement
        | LintKind::BoundaryError
        | LintKind::Capitalization
        | LintKind::Eggcorn
        | LintKind::Formatting
        | LintKind::Grammar
        | LintKind::Malapropism
        | LintKind::Punctuation
        | LintKind::Typo
        | LintKind::Usage
        | LintKind::WordChoice
        | LintKind::WordOrder => Strictness::Standard,
        LintKind::Enhancement
        | LintKind::Miscellaneous
        | LintKind::Nonstandard
        | LintKind::Readability
        | LintKind::Redundancy
        | LintKind::Regionalism
        | LintKind::Style => Strictness::Full,
    }
}

/// Checked once at setup to warm Harper's caches; never shown.
const WARM_UP: &str = "Hi Sarah, I wanted to folow up on the the lease. Their going to send the payment tomorrow, and we will deposit it on Monday.";

pub struct Checker {
    group: LintGroup,
}

impl Checker {
    /// Loads the dictionary and the rules, then checks a sample paragraph:
    /// ~0.5 s once (release build). Harper fills its caches on the first
    /// check (~25 ms, 60-70 ms live), so that one is paid here and not on the
    /// user's first paragraph; later checks take well under 1 ms.
    pub fn new() -> Self {
        let mut group = LintGroup::new_curated(FstDictionary::curated(), Dialect::American);
        for rule in DISABLED_RULES {
            group.config.set_rule_enabled(rule, false);
        }
        let mut checker = Checker { group };
        checker.check(WARM_UP, Rules::Full);
        checker
    }

    pub fn check(&mut self, paragraph: &str, rules: Rules) -> Vec<Finding> {
        self.check_at(paragraph, rules, Strictness::Full)
    }

    /// `check`, keeping only the findings `level` shows.
    pub fn check_at(&mut self, paragraph: &str, rules: Rules, level: Strictness) -> Vec<Finding> {
        let chars: Vec<char> = paragraph.chars().collect();
        let doc = Document::new_plain_english_curated(paragraph);
        // Lints on the same span become one finding: the most important
        // message (lowest priority number) and every distinct replacement.
        let mut by_span: BTreeMap<(usize, usize), (u8, Finding)> = BTreeMap::new();
        for (rule, lints) in self.group.organized_lints(&doc) {
            for lint in lints {
                let (start, end) = (lint.span.start, lint.span.end.min(chars.len()));
                if start >= end || lint.message.starts_with(INITIALISM_MESSAGE) || level_of(lint.lint_kind) > level {
                    continue;
                }
                if rule == "SpellCheck" && !worth_spelling(&chars, start, end) {
                    continue;
                }
                let problem: String = chars[start..end].iter().collect();
                if rules == Rules::Chat && (lint.lint_kind == LintKind::Capitalization || chat_word(&problem)) {
                    continue;
                }
                let replacements: Vec<String> = lint
                    .suggestions
                    .iter()
                    .filter_map(|s| replacement(s, &problem))
                    .collect();
                let kind = format!("{:?}", lint.lint_kind);
                let message = message(&rule, &lint.message, &chars, end);
                let entry = by_span.entry((start, end)).or_insert_with(|| {
                    (lint.priority, Finding { start, end, kind: kind.clone(), message: message.clone(), replacements: Vec::new(), clear: true })
                });
                if lint.priority < entry.0 {
                    entry.0 = lint.priority;
                    entry.1.kind = kind;
                    entry.1.message = message;
                }
                for r in replacements {
                    if !entry.1.replacements.contains(&r) {
                        entry.1.replacements.push(r);
                    }
                }
            }
        }
        by_span
            .into_values()
            .map(|(_, mut f)| {
                if f.kind == "Spelling" {
                    let problem: String = chars[f.start..f.end].iter().collect();
                    f.clear = rank(&problem, &mut f.replacements);
                }
                f.replacements.truncate(MAX_REPLACEMENTS);
                f
            })
            .collect()
    }
}

/// Folds case and curly apostrophes, for comparing words.
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| if c == '\u{2019}' { '\'' } else { c }).flat_map(char::to_lowercase).collect()
}

/// A spelling replacement that adds a possessive or a plural the user didn't
/// type: "possible's" or "possibles" for "possibleeee" (live in Outlook,
/// 2026-10-12: Harper put "possible's" first). A plural counts only when the
/// word without its "s" is offered too, so "was" for "wa" is a plain word.
fn adds_ending(problem: &str, replacement: &str, all: &[String]) -> bool {
    let (p, r) = (fold(problem), fold(replacement));
    let possessive = (r.ends_with(&['\'', 's']) || r.ends_with(&['s', '\''])) && !p.contains(&'\'');
    let plural = r.last() == Some(&'s') && p.last() != Some(&'s') && all.iter().any(|o| fold(o) == r[..r.len() - 1]);
    possessive || plural
}

/// Edit distance, an adjacent swap counting as one ("recieve" → "receive").
fn distance(a: &[char], b: &[char]) -> usize {
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Orders a spelling finding's replacements: plain words before ones that add
/// a possessive or a plural, otherwise Harper's own order. Returns whether the
/// first is clearly the best, for "Apply all": it adds no ending, and every
/// other plain word is further from what was typed, or as far but Harper also
/// put the first one first.
fn rank(problem: &str, replacements: &mut [String]) -> bool {
    let harper_first = replacements.first().cloned();
    let all = replacements.to_vec();
    replacements.sort_by_key(|r| adds_ending(problem, r, &all));
    let Some(top) = replacements.first() else { return false };
    if adds_ending(problem, top, &all) {
        return false;
    }
    let typed = fold(problem);
    let far = distance(&typed, &fold(top));
    replacements[1..].iter().filter(|r| !adds_ending(problem, r, &all)).all(|r| {
        let d = distance(&typed, &fold(r));
        d > far || (d == far && harper_first.as_deref() == Some(top.as_str()))
    })
}

/// Harper's message for a space before a sentence's final mark.
const SPACE_AT_THE_END: &str = "Unnecessary space at the end of the sentence";

/// The message to show for a lint. Harper's Spaces rule calls the space in
/// "word ?" an "Unnecessary space at the end of the sentence." (2.11.0): it
/// is the space before the mark, so say which mark.
fn message(rule: &str, harper: &str, chars: &[char], end: usize) -> String {
    if rule == "Spaces" && harper.starts_with(SPACE_AT_THE_END) {
        match chars[end..].iter().find(|c| !c.is_whitespace()) {
            Some('?') => return "Space before the question mark.".into(),
            Some('!') => return "Space before the exclamation mark.".into(),
            Some('.') => return "Space before the period.".into(),
            _ => {}
        }
    }
    harper.to_string()
}

/// The replacement text for one of Harper's edits, or None for one that only
/// changes spacing or leaves the text as it is.
fn replacement(edit: &Edit, problem: &str) -> Option<String> {
    let text = match edit {
        Edit::ReplaceWith(chars) => chars.iter().collect::<String>(),
        Edit::InsertAfter(chars) => format!("{problem}{}", chars.iter().collect::<String>()),
        Edit::Remove => String::new(),
    };
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    (text.is_empty() || squash(&text) != squash(problem)).then_some(text)
}

/// Whether a word the spell checker flagged is worth showing. Not: single
/// letters ("W-9"), words with digits ("2C"), words next to a digit or a
/// hyphen and a digit, all-capital acronyms ("HVAC"), and capitalized words
/// inside a sentence (names: "Acme Management").
fn worth_spelling(chars: &[char], start: usize, end: usize) -> bool {
    let word = &chars[start..end];
    if word.len() < 2 || word.iter().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let next = chars.get(end).copied();
    let after_next = chars.get(end + 1).copied();
    let prev = start.checked_sub(1).map(|i| chars[i]);
    if next.is_some_and(|c| c.is_ascii_digit())
        || (next == Some('-') && after_next.is_some_and(|c| c.is_ascii_digit()))
        || prev.is_some_and(|c| c.is_ascii_digit())
    {
        return false;
    }
    let letters: Vec<&char> = word.iter().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return false;
    }
    if word[0].is_uppercase() && !starts_sentence(chars, start) {
        return false;
    }
    true
}

/// The word at `start` begins a sentence: only spaces since the start of the
/// text, a line break, or ". ! ?".
fn starts_sentence(chars: &[char], start: usize) -> bool {
    chars[..start]
        .iter()
        .rev()
        .find(|c| !c.is_whitespace() || **c == '\n' || **c == '\r')
        .is_none_or(|c| matches!(c, '.' | '!' | '?' | '\n' | '\r'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// One checker per test thread: building it takes ~0.4 s (seconds in
        /// a debug build). Harper's rules aren't Send, as in the app, where
        /// the checker lives in the capture thread.
        static CHECKER: RefCell<Checker> = RefCell::new(Checker::new());
    }

    fn check(text: &str) -> Vec<Finding> {
        CHECKER.with(|c| c.borrow_mut().check(text, Rules::Full))
    }

    fn chat_problems(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        CHECKER.with(|c| c.borrow_mut().check(text, Rules::Chat)).iter().map(|f| chars[f.start..f.end].iter().collect()).collect()
    }

    /// Lines that drew capitalization, canonical-spelling and slang findings
    /// with the full rules (Harper 2.11.0, probed 2026-10-10).
    const CHAT: [&str; 6] = [
        "u coming tmrw? lol idk, brb. i think so",
        "thx btw, gonna be late. ur the best omg lmk",
        "ok np, ty! wanna grab lunch? i'm free rn",
        "hey can u send me the lease pls, i need it asap. sry idk where it went",
        "yeah sounds good, ill be there in 5. kinda busy tho",
        "imo its fine. dm me",
    ];

    #[test]
    fn chat_apps_skip_capitalization_and_chat_words() {
        for line in CHAT {
            assert!(!problems(line).is_empty(), "the full rules flag {line:?}");
            assert_eq!(chat_problems(line), Vec::<String>::new(), "{line:?}");
        }
    }

    #[test]
    fn chat_apps_still_get_real_mistakes() {
        let found = chat_problems("hey i recieved the lease, will send it tomorow. the the owner wants it");
        for p in ["recieved", "tomorow", "the the"] {
            assert!(found.iter().any(|f| f == p), "{p} in {found:?}");
        }
        // "u" is let through in chat apps only.
        assert!(problems("can u send it").iter().any(|f| f == "u"));
        assert!(chat_word("I\u{2019}m") && chat_word(" LOL "));
        assert!(CHAT_WORDS.iter().all(|w| !w.chars().any(|c| c.is_uppercase())));
    }

    /// The owner's retest (2026-10-10): "timez" looked missed in Outlook but
    /// was on the list's second page (nearest-caret order). Both typos are
    /// found, with the full rules and the chat rules alike. "He how" ("Hey
    /// how") is a real-word error: out of scope here, a 1c (AI rewrite) case.
    #[test]
    fn timez_and_weee_are_found() {
        let text = "He how many timez do weee have to keep doing this";
        assert_eq!(problems(text), ["timez", "weee"]);
        assert_eq!(chat_problems(text), ["timez", "weee"]);
    }

    #[test]
    #[ignore = "timing probe: cargo test --release -- --ignored --nocapture"]
    fn timing_probe() {
        let t = std::time::Instant::now();
        let mut c = Checker::new();
        println!("setup {:?}", t.elapsed());
        let texts = ["We recieved the payment and will deposit it tomorow.", "We got the payment and will bank it on Monday.", "Hi Sarah, I wanted to follow up on the the lease. We recieved the payment tomorow and will send the receipt to the owner by Friday afternoon at the latest."];
        for round in 0..3 {
            for x in texts {
                let t = std::time::Instant::now();
                c.check(x, Rules::Full);
                println!("round {round} {} chars {:?}", x.len(), t.elapsed());
            }
        }
    }

    fn problems(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        check(text).iter().map(|f| chars[f.start..f.end].iter().collect()).collect()
    }

    #[test]
    fn everyday_mistakes_are_found() {
        let found = problems("Hi Sarah, I wanted to follow up on the the lease. We recieved the payment tomorow.");
        for p in ["the the", "recieved", "tomorow"] {
            assert!(found.iter().any(|f| f == p), "{p} not found in {found:?}");
        }
        let f = check("We recieved it.");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].replacements.first().map(String::as_str), Some("received"));
        assert!(!f[0].message.is_empty() && f[0].kind == "Spelling");
    }

    /// Live in Outlook (2026-10-12): Harper offered "possible's" first for
    /// "possibleeee", and Apply all typed it.
    #[test]
    fn plain_words_come_before_possessives_and_plurals() {
        let first = |text: &str, word: &str| {
            let chars: Vec<char> = text.chars().collect();
            let f = check(text).into_iter().find(|f| chars[f.start..f.end].iter().collect::<String>() == word).unwrap();
            (f.replacements[0].clone(), f.clear)
        };
        let p = "We rreceive the payment tomorrow, possibleeee";
        assert_eq!(first(p, "possibleeee"), ("possible".into(), true));
        assert_eq!(first(p, "rreceive"), ("receive".into(), true));
        // "receive" and "relieve" are both one edit from "recieve": Harper's first stands.
        assert_eq!(first("We recieve it.", "recieve"), ("receive".into(), true));
        assert_eq!(first("We recieved it.", "recieved"), ("received".into(), true));
    }

    #[test]
    fn apply_all_takes_only_a_clearly_best_first_option() {
        let ranked = |problem: &str, options: &[&str]| {
            let mut r: Vec<String> = options.iter().map(|s| s.to_string()).collect();
            let clear = rank(problem, &mut r);
            (r, clear)
        };
        // Possessive and plural go last, in Harper's order otherwise.
        let (r, clear) = ranked("possibleeee", &["possible's", "possible", "possibles"]);
        assert_eq!((r, clear), (vec!["possible".into(), "possible's".into(), "possibles".into()], true));
        // An "s" is a plural only when the word without it is offered too.
        assert_eq!(ranked("wa", &["was", "way"]).0[0], "was");
        // A typed apostrophe or "s" is kept as typed.
        assert!(ranked("tenant's", &["tenant's", "tenants"]).1);
        // Only possessives on offer: none is clearly right.
        assert!(!ranked("Sarahh", &["Sarah's"]).1);
        // Another plain word is closer to what was typed than the first: not clear.
        assert!(!ranked("paymnt", &["pant", "payment"]).1);
        // As close, but Harper didn't put it first: not clear.
        assert!(!ranked("abcd", &["abc's", "abce", "abcf"]).1);
        assert!(ranked("abcd", &["abce", "abc's", "abcf"]).1, "Harper's first, tied: it stands");
        // Nothing offered.
        assert!(!ranked("zzz", &[]).1);
        assert_eq!(distance(&fold("recieve"), &fold("receive")), 1);
    }

    #[test]
    fn lints_on_the_same_span_become_one_finding() {
        // Harper flags "friday" twice (spelling, capitalization) and "Their"
        // twice (there, they're).
        let text = "Their is a few things to do before friday.";
        let f = check(text);
        let spans: Vec<(usize, usize)> = f.iter().map(|f| (f.start, f.end)).collect();
        let mut unique = spans.clone();
        unique.dedup();
        assert_eq!(spans, unique, "{f:?}");
        let friday = f.iter().find(|f| f.start == text.find("friday").unwrap()).expect("friday");
        assert_eq!(friday.replacements.iter().filter(|r| *r == "Friday").count(), 1);
        assert!(friday.replacements.len() <= MAX_REPLACEMENTS);
    }

    #[test]
    fn real_world_property_management_text_has_nothing_to_report() {
        for text in [
            "The tenant at unit 4B reported a leak.",
            "Units 101 102 103 104 need new smoke detectors.",
            "Please call Jane Doe at Acme Management LLC about 123 Main Street",
            "Sent the 3-day notice to the tenant in Apt 2C, balance $1,234.56 due 10/31.",
            "FYI the HVAC guy said the AC compressor is shot, ETA Tues.",
            "Can you send me the W-9 ASAP?",
            "Rent is $2,450.00 per month, due on the 1st.",
            "We need to discuss the renewal before Friday.",
        ] {
            assert_eq!(problems(text), Vec::<String>::new(), "{text}");
        }
    }

    #[test]
    fn a_lowercase_misspelling_at_the_start_is_still_found() {
        assert!(problems("Recieved your note.").iter().any(|p| p == "Recieved"));
    }

    /// The owner, 2026-10-11: "word ?" said "space at the end of the
    /// sentence"; it is the space before the question mark.
    #[test]
    fn a_space_before_the_mark_is_named_as_such() {
        let said = |text: &str| check(text).into_iter().map(|f| (f.message, f.replacements)).collect::<Vec<_>>();
        let only = |m: &str| vec![(m.to_string(), vec![String::new()])];
        assert_eq!(said("Can you send it ?"), only("Space before the question mark."));
        assert_eq!(said("Is it done ? Thanks."), only("Space before the question mark."));
        assert_eq!(said("Really !"), only("Space before the exclamation mark."));
        assert_eq!(said("Done ."), only("Space before the period."));
        // Anything else is Harper's own message.
        assert_eq!(message("Spaces", "Unnecessary space at the end of the sentence.", &['a', ' '], 2), "Unnecessary space at the end of the sentence.");
        assert_eq!(message("Other", "Unnecessary space at the end of the sentence.", &[' ', '?'], 1), "Unnecessary space at the end of the sentence.");
        assert_eq!(said("Wait , no."), only("Don't use a space before a comma."));
    }

    /// The paragraph in docs/manual-tests.md §19: spelling, grammar and
    /// wordiness, so each level shows something the one below doesn't.
    const STRICTNESS_FIXTURE: &str = "Hi Sarah, I recieved the the lease. Their is a few things to do before friday , ok? The plumber can come at 8am in the morning, and the repair is for free of charge.";

    /// Harper 2.11 misses these (agreement across "and" and in "that"
    /// clauses), at every level: the AI check is what catches them
    /// (docs/phase1-plan.md, "Check with AI"). If a Harper update starts
    /// flagging them, update Settings' level lines and this test.
    #[test]
    fn known_agreement_gaps_stay_documented() {
        assert!(problems_at("Alex and I was at the unit on Monday.", Strictness::Full).is_empty());
        assert!(problems_at("There are a few issues that needs fixing.", Strictness::Full).is_empty());
    }

    fn problems_at(text: &str, level: Strictness) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        CHECKER.with(|c| c.borrow_mut().check_at(text, Rules::Full, level)).iter().map(|f| chars[f.start..f.end].iter().collect()).collect()
    }

    /// §14.4: each level gives what it says, and only takes findings away.
    #[test]
    fn strictness_levels_keep_only_their_findings() {
        let text = STRICTNESS_FIXTURE;
        let spelling = problems_at(text, Strictness::Spelling);
        let standard = problems_at(text, Strictness::Standard);
        let full = problems_at(text, Strictness::Full);
        for p in ["recieved", "the the"] {
            assert!(spelling.iter().any(|f| f == p), "{p} in {spelling:?}");
        }
        assert!(!spelling.iter().any(|f| f == "Their" || f.trim() == ","), "{spelling:?}");
        assert!(standard.iter().any(|f| f == "Their"), "grammar at Standard: {standard:?}");
        assert!(spelling.len() < standard.len() && standard.len() < full.len(), "{spelling:?} {standard:?} {full:?}");
        // Full adds wordiness the other levels leave alone (§19 retest,
        // 2026-10-14: without these, Full looked the same as Standard).
        for p in ["am in the morning", "for free of charge"] {
            assert!(full.iter().any(|f| f == p) && !standard.iter().any(|f| f == p), "{p}: {standard:?} {full:?}");
        }
        for f in &spelling {
            assert!(standard.contains(f) && full.contains(f), "{f}");
        }
        assert_eq!(full, problems(text), "Full is what the checker did before 1d");
        // Chat rules still apply on top.
        let chars: Vec<char> = "u coming tmrw? i recieved it".chars().collect();
        let chat: Vec<String> = CHECKER
            .with(|c| c.borrow_mut().check_at("u coming tmrw? i recieved it", Rules::Chat, Strictness::Standard))
            .iter()
            .map(|f| chars[f.start..f.end].iter().collect())
            .collect();
        assert_eq!(chat, ["recieved"]);
    }

    #[test]
    fn every_harper_kind_has_a_level_and_settings_ids_read_right() {
        use LintKind::*;
        let all = [
            Agreement, BoundaryError, Capitalization, Eggcorn, Enhancement, Formatting, Grammar, Malapropism,
            Miscellaneous, Nonstandard, Punctuation, Readability, Redundancy, Regionalism, Repetition, Spelling, Style,
            Typo, Usage, WordChoice, WordOrder,
        ];
        let at = |l: Strictness| all.iter().filter(|k| level_of(**k) == l).count();
        assert_eq!((at(Strictness::Spelling), at(Strictness::Standard), at(Strictness::Full)), (2, 12, 7));
        assert_eq!(Strictness::from_id("spelling"), Strictness::Spelling);
        assert_eq!(Strictness::from_id("standard"), Strictness::Standard);
        assert_eq!(Strictness::from_id("full"), Strictness::Full);
        assert_eq!(Strictness::from_id("loud"), Strictness::Full);
        assert_eq!(Strictness::default(), Strictness::Full);
    }

    #[test]
    fn formatting_a_finding_never_shows_its_words() {
        let f = Finding { start: 1, end: 2, kind: "Spelling".into(), message: "SENTINEL-msg".into(), replacements: vec!["SENTINEL-rep".into()], clear: true };
        assert!(!format!("{f:?}").contains("SENTINEL"));
    }
}


