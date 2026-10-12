// Phase 1b: typing a fix into the field it came from (docs/phase1-plan.md §7.3).
//
// The steps live here, over a `Field` that the capture thread implements with
// UI Automation (capture/fix.rs) and the tests implement with a fake. Every
// step fails closed: the paragraph must be found word for word, every range is
// read back before it is used, the selection is read back before anything is
// typed, and the result is read back after. A refusal before typing leaves the
// field as it was (the caret goes back if Glim had selected something).
//
// Privacy: nothing here logs, and the field's text never leaves this module
// except as the comparison results above.

use super::{Job, Outcome};

/// What the apply steps need from the field.
pub trait Field {
    /// A text range in the field, in the app's own units.
    type Range: Clone;
    /// Steps 1-2: still the same field, in front, and every PR #8 rule passes
    /// on it now (password, payment labels, paused app or site, read-only).
    fn gate(&mut self) -> Result<(), Outcome>;
    /// The paragraph the caret is in, if the app can say.
    fn caret_paragraph(&mut self) -> Option<Self::Range>;
    /// The whole field.
    fn document(&mut self) -> Option<Self::Range>;
    fn text(&mut self, r: &Self::Range) -> Option<String>;
    /// Characters from the start of the field to the start of `r`.
    fn offset(&mut self, r: &Self::Range) -> Option<usize>;
    /// The caret, in characters from the start of the field.
    fn caret(&mut self) -> Option<usize>;
    /// `len` characters of `r` from `from`, in the app's units (which may not
    /// be ours: always read back before use).
    fn sub(&mut self, r: &Self::Range, from: usize, len: usize) -> Option<Self::Range>;
    fn select(&mut self, r: &Self::Range) -> bool;
    /// The app's selection's text, when there is exactly one.
    fn selection(&mut self) -> Option<String>;
    /// Step 6: no key or mouse button held (waits a little for a release).
    fn keys_ready(&mut self) -> Result<(), Outcome>;
    /// Step 7: types `text` over the selection in one go; an empty `text` is
    /// one Backspace. False if Windows didn't take the input.
    fn type_over(&mut self, text: &str) -> bool;
    /// A short pause while the app takes the keys: `SendInput` only queues
    /// them, so the field changes a moment later, a key at a time.
    fn pause(&mut self);
}

/// How many times the result is read back while the keys land (with `pause`
/// between: about 1.5 s in all).
pub const SETTLE_TRIES: usize = 50;

/// Where a paragraph was found: the range searched, and the paragraph's
/// character index in that range's text.
struct Found<R> {
    scope: R,
    at: usize,
}

/// Character indices of every occurrence of `needle` in `hay`.
fn occurrences(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    hay.match_indices(needle).map(|(byte, _)| hay[..byte].chars().count()).collect()
}

/// Finds `paragraph` by its content: in the caret's paragraph if it is there
/// exactly once, else in the whole field (once; or, if it is there more than
/// once, the copy at `field_start`). None: the text changed.
fn locate<F: Field>(f: &mut F, paragraph: &str, field_start: usize) -> Option<Found<F::Range>> {
    if let Some(p) = f.caret_paragraph() {
        if let Some(t) = f.text(&p) {
            if let [at] = occurrences(&t, paragraph)[..] {
                return Some(Found { scope: p, at });
            }
        }
    }
    let doc = f.document()?;
    let t = f.text(&doc)?;
    match occurrences(&t, paragraph)[..] {
        [at] => Some(Found { scope: doc, at }),
        ref many => many.iter().find(|&&at| at == field_start).map(|&at| Found { scope: doc, at }),
    }
}

/// `len` characters at `from` in the found range, read back: Some only if the
/// app's range holds exactly `expect`.
fn checked_sub<F: Field>(f: &mut F, found: &Found<F::Range>, from: usize, len: usize, expect: &str) -> Option<F::Range> {
    let r = f.sub(&found.scope, found.at + from, len)?;
    (f.text(&r).as_deref() == Some(expect)).then_some(r)
}

/// Where the caret goes after the span `start..end` became `new_len`
/// characters: unchanged before it, shifted after it, at the end of the
/// replacement if it was inside.
pub fn moved_caret(caret: usize, start: usize, end: usize, new_len: usize) -> usize {
    if caret <= start {
        caret
    } else if caret >= end {
        caret - (end - start) + new_len
    } else {
        start + new_len
    }
}

/// Puts the caret at `at` characters into the found paragraph, after reading
/// back the text before it.
fn place_caret<F: Field>(f: &mut F, found: &Found<F::Range>, paragraph: &str, at: usize) {
    let before: String = paragraph.chars().take(at).collect();
    if checked_sub(f, found, 0, at, &before).is_some() {
        if let Some(r) = f.sub(&found.scope, found.at + at, 0) {
            f.select(&r);
        }
    }
}

/// Applies one fix (§7.3). Nothing is typed unless every check before it
/// passed.
pub fn run<F: Field>(f: &mut F, job: &Job) -> Outcome {
    // 1-2. The same field, in front, and allowed now.
    if let Err(o) = f.gate() {
        return o;
    }
    // 3. The paragraph, by its content, read back whole.
    let Some(found) = locate(f, &job.paragraph, job.field_start) else { return Outcome::Changed };
    let len = job.paragraph.chars().count();
    let Some(whole) = checked_sub(f, &found, 0, len, &job.paragraph) else { return Outcome::Changed };
    // 4. The problem word, read back.
    let chars: Vec<char> = job.paragraph.chars().collect();
    if job.start > job.end || job.end > chars.len() {
        return Outcome::Changed;
    }
    let problem: String = chars[job.start..job.end].iter().collect();
    let Some(target) = checked_sub(f, &found, job.start, job.end - job.start, &problem) else { return Outcome::Changed };
    // Where the user's caret is in the paragraph, to put it back after.
    let caret_in = match (f.caret(), f.offset(&whole)) {
        (Some(c), Some(p)) if c >= p && c <= p + len => Some(c - p),
        _ => None,
    };
    // 6 (first look): nothing held, before anything is selected.
    if let Err(o) = f.keys_ready() {
        return o;
    }
    // 5. Select it, and read the app's selection back.
    let undo_select = |f: &mut F| {
        if let Some(c) = caret_in {
            place_caret(f, &found, &job.paragraph, c);
        }
    };
    if !f.select(&target) || f.selection().as_deref() != Some(problem.as_str()) {
        undo_select(f);
        return Outcome::Unsupported;
    }
    // 6. Last checks, immediately before typing.
    if let Err(o) = f.gate().and_then(|_| f.keys_ready()) {
        undo_select(f);
        return o;
    }
    // 7. Type it.
    if !f.type_over(&job.replacement) {
        undo_select(f);
        return Outcome::Unsupported;
    }
    // 8. Read back: the paragraph with exactly that span replaced. The keys
    // land a moment after SendInput (a half-typed word reads back first), so
    // the result is waited for before anything is decided.
    let fixed: String = chars[..job.start].iter().collect::<String>() + &job.replacement + &chars[job.end..].iter().collect::<String>();
    let fixed_len = fixed.chars().count();
    let mut landed = None;
    for _ in 0..SETTLE_TRIES {
        if let Some(found) = locate(f, &fixed, job.field_start) {
            if checked_sub(f, &found, 0, fixed_len, &fixed).is_some() {
                landed = Some(found);
                break;
            }
        }
        f.pause();
    }
    let Some(after) = landed else {
        // Nothing changed at all: the keys didn't land (an app that ignores
        // them, or Windows blocked them). Otherwise something else happened.
        let untouched = locate(f, &job.paragraph, job.field_start).is_some();
        return if untouched { Outcome::Unsupported } else { Outcome::NotClean };
    };
    // The caret back where the user was typing (owner, 2026-10-11).
    if let Some(c) = caret_in {
        let at = moved_caret(c, job.start, job.end, job.replacement.chars().count());
        place_caret(f, &after, &fixed, at);
    }
    Outcome::Applied
}

/// "Apply all": the jobs in order (Session::jobs_all: from the end of the
/// paragraph backward), each through every step of `run`, stopping at the
/// first that doesn't apply. Returns how many applied, and the outcome that
/// stopped it (Applied when all did).
pub fn run_all<F: Field>(f: &mut F, jobs: &[Job]) -> (usize, Outcome) {
    for (done, job) in jobs.iter().enumerate() {
        let outcome = run(f, job);
        if outcome != Outcome::Applied {
            return (done, outcome);
        }
    }
    (jobs.len(), Outcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field as a string of characters, with paragraphs split by '\n'.
    /// Ranges are (start, end) in characters; `skew` shifts every `sub` to
    /// mimic an app whose units aren't ours.
    struct Fake {
        doc: Vec<char>,
        caret: usize,
        sel: Option<(usize, usize)>,
        skew: usize,
        paragraph_unit: bool,
        select_works: bool,
        gate: Vec<Result<(), Outcome>>,
        keys: Result<(), Outcome>,
        /// What actually lands when typing (None: what was asked).
        lands: Option<String>,
        /// Keys still on their way: (start, end, text, pauses left). Until
        /// they land, the field shows a half-typed word.
        pending: Option<(usize, usize, String, usize)>,
        lag: usize,
        pauses: usize,
        typing_fails: bool,
        typed: Vec<String>,
        selects: usize,
    }

    impl Fake {
        fn new(doc: &str, caret_at: &str) -> Fake {
            let caret = doc[..doc.find(caret_at).unwrap()].chars().count() + caret_at.chars().count();
            Fake {
                doc: doc.chars().collect(),
                caret,
                sel: None,
                skew: 0,
                paragraph_unit: true,
                select_works: true,
                gate: Vec::new(),
                keys: Ok(()),
                lands: None,
                typing_fails: false,
                typed: Vec::new(),
                selects: 0,
                pending: None,
                lag: 0,
                pauses: 0,
            }
        }
        fn text(&self) -> String {
            self.doc.iter().collect()
        }
    }

    impl Field for Fake {
        type Range = (usize, usize);
        fn gate(&mut self) -> Result<(), Outcome> {
            if self.gate.is_empty() { Ok(()) } else { self.gate.remove(0) }
        }
        fn caret_paragraph(&mut self) -> Option<(usize, usize)> {
            if !self.paragraph_unit {
                return None;
            }
            let start = self.doc[..self.caret].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
            let end = self.doc[self.caret..].iter().position(|&c| c == '\n').map_or(self.doc.len(), |i| self.caret + i);
            Some((start, end))
        }
        fn document(&mut self) -> Option<(usize, usize)> {
            Some((0, self.doc.len()))
        }
        fn text(&mut self, r: &(usize, usize)) -> Option<String> {
            (r.1 <= self.doc.len() && r.0 <= r.1).then(|| self.doc[r.0..r.1].iter().collect())
        }
        fn offset(&mut self, r: &(usize, usize)) -> Option<usize> {
            Some(r.0)
        }
        fn caret(&mut self) -> Option<usize> {
            Some(self.caret)
        }
        fn sub(&mut self, r: &(usize, usize), from: usize, len: usize) -> Option<(usize, usize)> {
            let s = r.0 + from + self.skew;
            (s + len <= self.doc.len()).then_some((s, s + len))
        }
        fn select(&mut self, r: &(usize, usize)) -> bool {
            self.selects += 1;
            if !self.select_works {
                return false;
            }
            self.sel = Some(*r);
            self.caret = r.1;
            true
        }
        fn selection(&mut self) -> Option<String> {
            self.sel.map(|(s, e)| self.doc[s..e].iter().collect())
        }
        fn keys_ready(&mut self) -> Result<(), Outcome> {
            self.keys
        }
        fn type_over(&mut self, text: &str) -> bool {
            if self.typing_fails {
                return false;
            }
            self.typed.push(text.to_string());
            let (s, e) = self.sel.take().unwrap();
            let lands = self.lands.clone().unwrap_or_else(|| text.to_string());
            if self.lag > 0 {
                // The first key has landed, the rest are on their way.
                let first: String = lands.chars().take(1).collect();
                self.doc.splice(s..e, first.chars());
                self.caret = s + 1;
                self.pending = Some((s, s + 1, lands, self.lag));
                return true;
            }
            self.doc.splice(s..e, lands.chars());
            self.caret = s + lands.chars().count();
            true
        }
        fn pause(&mut self) {
            self.pauses += 1;
            if let Some((s, e, text, left)) = self.pending.take() {
                if left <= 1 {
                    self.doc.splice(s..e, text.chars());
                    self.caret = s + text.chars().count();
                } else {
                    self.pending = Some((s, e, text, left - 1));
                }
            }
        }
    }

    const DOC: &str = "Hi Sam,\nWe recieved the payment today.\nBest regards,\nAnthony";

    fn job(paragraph: &str, problem: &str, replacement: &str, field_start: usize) -> Job {
        let start = paragraph[..paragraph.find(problem).unwrap()].chars().count();
        Job {
            paragraph: paragraph.into(),
            field_start,
            start,
            end: start + problem.chars().count(),
            replacement: replacement.into(),
            app: "Notepad".into(),
        }
    }

    fn fix() -> Job {
        job("We recieved the payment today.", "recieved", "received", 8)
    }

    #[test]
    fn applies_the_fix_and_puts_the_caret_back_where_the_user_was_typing() {
        let mut f = Fake::new(DOC, "payment today.");
        let caret_before = f.caret;
        assert_eq!(run(&mut f, &fix()), Outcome::Applied);
        assert_eq!(f.text(), "Hi Sam,\nWe received the payment today.\nBest regards,\nAnthony");
        assert_eq!(f.typed, ["received"]);
        // "received" is as long as "recieved": the caret is where it was.
        assert_eq!(f.caret, caret_before);
    }

    #[test]
    fn the_caret_moves_with_the_text_after_the_fix() {
        let mut f = Fake::new(DOC, "payment");
        let before = f.caret;
        let j = job("We recieved the payment today.", "recieved", "got", 8);
        assert_eq!(run(&mut f, &j), Outcome::Applied);
        assert_eq!(f.caret, before - 5);
        assert_eq!(moved_caret(2, 3, 11, 8), 2, "before the fix: unchanged");
        assert_eq!(moved_caret(5, 3, 11, 8), 11, "inside: end of the replacement");
        assert_eq!(moved_caret(20, 3, 11, 3), 15, "after: shifted");
    }

    #[test]
    fn an_edit_inside_the_paragraph_refuses_and_types_nothing() {
        let mut f = Fake::new("Hi Sam,\nWe recieved the payment yesterday.\nBest", "payment");
        assert_eq!(run(&mut f, &fix()), Outcome::Changed);
        assert!(f.typed.is_empty());
        assert_eq!(f.selects, 0, "nothing was even selected");
    }

    #[test]
    fn an_edit_elsewhere_in_the_field_does_not_block_the_fix() {
        let mut f = Fake::new("Hello Samantha and team,\nWe recieved the payment today.\nBest", "today");
        assert_eq!(run(&mut f, &fix()), Outcome::Applied);
        assert!(f.text().contains("We received the payment today."));
    }

    #[test]
    fn a_paragraph_twice_in_the_field_is_found_by_the_caret_or_its_position() {
        let doc = "We recieved the payment today.\nWe recieved the payment today.";
        // The caret is in the second copy: that one is fixed.
        let mut f = Fake::new(doc, "today.\nWe recieved the payment");
        assert_eq!(run(&mut f, &job("We recieved the payment today.", "recieved", "received", 31)), Outcome::Applied);
        assert_eq!(f.text(), "We recieved the payment today.\nWe received the payment today.");
        // No paragraph unit from the app: the copy at the stored position.
        let mut f = Fake::new(doc, "We");
        f.paragraph_unit = false;
        assert_eq!(run(&mut f, &job("We recieved the payment today.", "recieved", "received", 31)), Outcome::Applied);
        assert_eq!(f.text(), "We recieved the payment today.\nWe received the payment today.");
        // Neither copy where it was checked: refused.
        let mut f = Fake::new(doc, "We");
        f.paragraph_unit = false;
        assert_eq!(run(&mut f, &job("We recieved the payment today.", "recieved", "received", 5)), Outcome::Changed);
        assert!(f.typed.is_empty());
    }

    #[test]
    fn an_app_whose_units_are_not_ours_is_caught_by_the_read_back() {
        let mut f = Fake::new(DOC, "payment");
        f.skew = 1;
        assert_eq!(run(&mut f, &fix()), Outcome::Changed);
        assert!(f.typed.is_empty());
        assert_eq!(f.text(), DOC);
    }

    #[test]
    fn an_app_that_ignores_the_selection_is_refused_before_typing() {
        let mut f = Fake::new(DOC, "payment");
        f.select_works = false;
        assert_eq!(run(&mut f, &fix()), Outcome::Unsupported);
        assert!(f.typed.is_empty());
        assert_eq!(f.text(), DOC);
    }

    #[test]
    fn a_held_key_refuses_before_anything_is_selected() {
        let mut f = Fake::new(DOC, "payment");
        f.keys = Err(Outcome::KeysHeld);
        assert_eq!(run(&mut f, &fix()), Outcome::KeysHeld);
        assert!(f.typed.is_empty());
        assert_eq!(f.selects, 0);
    }

    #[test]
    fn the_gate_refuses_before_anything_is_read() {
        for o in [Outcome::Away, Outcome::Blocked, Outcome::Paused, Outcome::Elevated] {
            let mut f = Fake::new(DOC, "payment");
            f.gate = vec![Err(o)];
            assert_eq!(run(&mut f, &fix()), o);
            assert!(f.typed.is_empty());
            assert_eq!(f.selects, 0);
        }
    }

    #[test]
    fn focus_lost_after_selecting_types_nothing_and_puts_the_caret_back() {
        let mut f = Fake::new(DOC, "payment");
        let caret = f.caret;
        f.gate = vec![Ok(()), Err(Outcome::Away)];
        assert_eq!(run(&mut f, &fix()), Outcome::Away);
        assert!(f.typed.is_empty());
        assert_eq!(f.text(), DOC);
        assert_eq!(f.caret, caret, "the selection was undone");
    }

    #[test]
    fn keys_that_never_land_change_nothing_and_say_so() {
        let mut f = Fake::new(DOC, "payment");
        f.typing_fails = true;
        assert_eq!(run(&mut f, &fix()), Outcome::Unsupported);
        assert_eq!(f.text(), DOC);
        let mut f = Fake::new(DOC, "payment");
        f.lands = Some("recieved".into()); // e.g. Windows dropped them (an elevated app)
        assert_eq!(run(&mut f, &fix()), Outcome::Unsupported);
    }

    /// Found live in Notepad (2026-10-11): SendInput only queues the keys,
    /// so the first read-back saw a half-typed word and reported not-clean.
    #[test]
    fn keys_that_land_a_moment_later_are_waited_for() {
        let mut f = Fake::new(DOC, "payment today.");
        let caret = f.caret;
        f.lag = 5;
        assert_eq!(run(&mut f, &fix()), Outcome::Applied);
        assert_eq!(f.text(), "Hi Sam,\nWe received the payment today.\nBest regards,\nAnthony");
        assert_eq!(f.caret, caret, "the caret is put back once they have landed");
        assert!(f.pauses >= 5);
        // Keys that never finish landing: reported, after the wait.
        let mut f = Fake::new(DOC, "payment");
        f.lag = SETTLE_TRIES + 10;
        assert_eq!(run(&mut f, &fix()), Outcome::NotClean);
        assert_eq!(f.pauses, SETTLE_TRIES);
    }

    #[test]
    fn a_result_that_does_not_read_back_is_reported_not_repaired() {
        let mut f = Fake::new(DOC, "payment");
        f.lands = Some("RECEIVED!!".into());
        assert_eq!(run(&mut f, &fix()), Outcome::NotClean);
        assert_eq!(f.typed, ["received"], "typed once, never again, no undo sent");
    }

    #[test]
    fn a_removal_is_one_backspace_over_the_selection() {
        let mut f = Fake::new("We will the the deposit it.", "deposit");
        let j = job("We will the the deposit it.", "the ", "", 0);
        assert_eq!(run(&mut f, &j), Outcome::Applied);
        assert_eq!(f.typed, [""]);
        assert_eq!(f.text(), "We will the deposit it.");
    }

    const THREE: &str = "Hi Sam,
We recieved the payment tomorow, posible.
Best regards,
Anthony";

    /// The three jobs Session::jobs_all makes for THREE: back to front, each
    /// on the paragraph the later fixes left.
    fn three_jobs() -> Vec<Job> {
        vec![
            job("We recieved the payment tomorow, posible.", "posible", "possible", 8),
            job("We recieved the payment tomorow, possible.", "tomorow", "tomorrow", 8),
            job("We recieved the payment tomorrow, possible.", "recieved", "received", 8),
        ]
    }

    #[test]
    fn apply_all_types_every_fix_back_to_front_and_verifies_each() {
        let mut f = Fake::new(THREE, "We");
        let (done, outcome) = run_all(&mut f, &three_jobs());
        assert_eq!((done, outcome), (3, Outcome::Applied));
        assert_eq!(f.text(), "Hi Sam,
We received the payment tomorrow, possible.
Best regards,
Anthony");
        assert_eq!(f.typed, ["possible", "tomorrow", "received"]);
    }

    #[test]
    fn apply_all_stops_at_the_first_fix_that_fails() {
        // The user leaves the field after the first fix: two gates per fix pass, the third fails.
        let mut f = Fake::new(THREE, "We");
        f.gate = vec![Ok(()), Ok(()), Err(Outcome::Away)];
        let (done, outcome) = run_all(&mut f, &three_jobs());
        assert_eq!((done, outcome), (1, Outcome::Away));
        assert_eq!(f.typed, ["possible"], "nothing after the failure");
        // The text changed under it (an edit between fixes): the next is refused, not typed.
        let mut f = Fake::new(THREE, "We");
        let mut jobs = three_jobs();
        jobs[1].paragraph = "We recieved the payment tomorow, possibly.".into();
        let (done, outcome) = run_all(&mut f, &jobs);
        assert_eq!((done, outcome), (1, Outcome::Changed));
        assert_eq!(f.typed, ["possible"]);
    }
}
