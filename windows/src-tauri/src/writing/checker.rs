// The instant checker: Harper (harper-core), in-process, American English.
// See docs/phase1-plan.md §3 for what it catches and what it misses.

use std::collections::BTreeMap;

use harper_core::linting::{LintGroup, Suggestion as Edit};
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
        checker.check(WARM_UP);
        checker
    }

    pub fn check(&mut self, paragraph: &str) -> Vec<Finding> {
        let chars: Vec<char> = paragraph.chars().collect();
        let doc = Document::new_plain_english_curated(paragraph);
        // Lints on the same span become one finding: the most important
        // message (lowest priority number) and every distinct replacement.
        let mut by_span: BTreeMap<(usize, usize), (u8, Finding)> = BTreeMap::new();
        for (rule, lints) in self.group.organized_lints(&doc) {
            for lint in lints {
                let (start, end) = (lint.span.start, lint.span.end.min(chars.len()));
                if start >= end || lint.message.starts_with(INITIALISM_MESSAGE) {
                    continue;
                }
                if rule == "SpellCheck" && !worth_spelling(&chars, start, end) {
                    continue;
                }
                let problem: String = chars[start..end].iter().collect();
                let replacements: Vec<String> = lint
                    .suggestions
                    .iter()
                    .filter_map(|s| replacement(s, &problem))
                    .collect();
                let kind = format!("{:?}", lint.lint_kind);
                let entry = by_span.entry((start, end)).or_insert_with(|| {
                    (lint.priority, Finding { start, end, kind: kind.clone(), message: lint.message.clone(), replacements: Vec::new() })
                });
                if lint.priority < entry.0 {
                    entry.0 = lint.priority;
                    entry.1.kind = kind;
                    entry.1.message = lint.message.clone();
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
                f.replacements.truncate(MAX_REPLACEMENTS);
                f
            })
            .collect()
    }
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
/// inside a sentence (names: "Audii Management").
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
        CHECKER.with(|c| c.borrow_mut().check(text))
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
                c.check(x);
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
            "Please call Anthony Garcia at Audii Management LLC about 1408 Jefferson Ave.",
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

    #[test]
    fn formatting_a_finding_never_shows_its_words() {
        let f = Finding { start: 1, end: 2, kind: "Spelling".into(), message: "SENTINEL-msg".into(), replacements: vec!["SENTINEL-rep".into()] };
        assert!(!format!("{f:?}").contains("SENTINEL"));
    }
}
