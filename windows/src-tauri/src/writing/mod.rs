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

pub mod apps;
pub mod checker;
pub mod text;

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
}

impl std::fmt::Debug for Suggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggestion").field("start", &self.start).field("end", &self.end).field("kind", &self.kind).finish()
    }
}

/// What the island shows: the current field's suggestions (empty = none).
#[derive(Clone, Default, PartialEq, Serialize)]
pub struct Suggestions {
    pub items: Vec<Suggestion>,
}

impl std::fmt::Debug for Suggestions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggestions").field("count", &self.items.len()).finish()
    }
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
}

impl Debouncer {
    pub fn feed(&mut self, key: u64, now: Instant) -> Step {
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
    /// Check times since the last report, in microseconds.
    times: Vec<u128>,
    /// Paragraph lengths checked since the last report, in characters.
    lengths: Vec<usize>,
}

impl Session {
    /// One reading of the focused field. `text` is None when nothing may be
    /// checked (an app that is off, a skipped or dropped reading); `same` is
    /// false when focus moved to another field. `emit` gets the island's new
    /// suggestions whenever they change.
    pub fn observe(
        &mut self,
        same: bool,
        text: Option<&str>,
        caret: Option<usize>,
        rules: Rules,
        now: Instant,
        mut emit: impl FnMut(&Suggestions),
    ) {
        if !same {
            self.debounce.reset();
            self.show(Suggestions::default(), &mut emit);
        }
        let Some(paragraph) = text.and_then(|t| paragraph_to_check(t, caret)) else {
            self.debounce.reset();
            self.show(Suggestions::default(), &mut emit);
            return;
        };
        if self.debounce.feed(key_of(&paragraph), now) == Step::Check {
            let found = self.check(&paragraph, caret, rules);
            self.show(found, &mut emit);
        }
    }

    /// The paragraph's suggestions, nearest the caret first (ties: earlier in
    /// the text), so the first page is about what the user just wrote.
    fn check(&mut self, p: &Paragraph, caret: Option<usize>, rules: Rules) -> Suggestions {
        let checker = self.checker.get_or_insert_with(Checker::new);
        let t = Instant::now();
        let findings = checker.check(&p.text, rules);
        self.times.push(t.elapsed().as_micros());
        self.lengths.push(p.text.chars().count());
        let chars: Vec<char> = p.text.chars().collect();
        let mut items: Vec<Suggestion> = findings.iter().map(|f| suggestion(&chars, p.start, f)).collect();
        if let Some(caret) = caret {
            items.sort_by_key(|s| (distance(caret, s.start, s.end), s.start));
        }
        Suggestions { items }
    }

    fn show(&mut self, s: Suggestions, emit: &mut impl FnMut(&Suggestions)) {
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

/// How far the caret is from a problem, in characters: 0 inside it.
fn distance(caret: usize, start: usize, end: usize) -> usize {
    if caret < start {
        start - caret
    } else {
        caret.saturating_sub(end)
    }
}

fn suggestion(chars: &[char], offset: usize, f: &Finding) -> Suggestion {
    let s: String = chars[f.start..f.end].iter().collect();
    let before: String = chars[f.start.saturating_sub(CONTEXT_CHARS)..f.start].iter().collect();
    let after: String = chars[f.end..(f.end + CONTEXT_CHARS).min(chars.len())].iter().collect();
    Suggestion {
        start: offset + f.start,
        end: offset + f.end,
        problem: s,
        before,
        after,
        kind: f.kind.clone(),
        message: f.message.clone(),
        replacements: f.replacements.clone(),
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
    fn suggestions_come_nearest_the_caret_first() {
        let field = "Teh lease is recieved. We will deposit it tomorow and send the reciept.";
        let caret = field.find("send").unwrap();
        let mut s = Session::default();
        let mut got = Suggestions::default();
        let t0 = Instant::now();
        for ms in [0, 600] {
            s.observe(true, Some(field), Some(caret), Rules::Full, t0 + Duration::from_millis(ms), |x| got = x.clone());
        }
        let order: Vec<&str> = got.items.iter().map(|i| i.problem.as_str()).collect();
        assert_eq!(order, ["tomorow", "reciept", "recieved", "Teh"]);
        assert_eq!((distance(5, 3, 8), distance(2, 3, 8), distance(10, 3, 8)), (0, 1, 2));
    }

    #[test]
    fn a_session_shows_suggestions_after_the_pause_and_clears_on_another_field() {
        let mut s = Session::default();
        let caret = Some(12);
        let emitted = run(&mut s, &[
            (true, Some(FIELD), caret, 0),
            (true, Some(FIELD), caret, 300),
            (true, Some(FIELD), caret, 1100), // checked: "recieved"
            (true, Some(FIELD), caret, 1400), // unchanged: nothing sent
            (false, Some("Clean text."), Some(3), 1700), // another field: cleared
        ]);
        assert_eq!(emitted, vec![1, 0]);
        assert!(s.take_report().unwrap().starts_with("writing: 1 checks"));
        assert!(s.take_report().is_none());
    }

    #[test]
    fn a_reading_that_may_not_be_checked_clears_the_suggestions() {
        let mut s = Session::default();
        let emitted = run(&mut s, &[(true, Some(FIELD), Some(12), 0), (true, Some(FIELD), Some(12), 1000), (true, None, None, 1300)]);
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

    #[test]
    fn formatting_suggestions_never_shows_their_words() {
        let s = Suggestions { items: vec![Suggestion { start: 0, end: 1, problem: "SENTINEL-p".into(), before: "SENTINEL-b".into(), after: "SENTINEL-a".into(), kind: "Spelling".into(), message: "SENTINEL-m".into(), replacements: vec!["SENTINEL-r".into()] }] };
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
}
