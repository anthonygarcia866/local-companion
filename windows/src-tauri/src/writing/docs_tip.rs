// Google Docs draws its pages on a canvas: unless the document has screen
// reader support on (Tools → Accessibility, Ctrl+Alt+Z), UI Automation finds
// no text in the field the user types in, so Glim has nothing to check
// (Anthony's live test, 2026-10-14, docs/manual-tests.md §20). When that
// happens, the island shows a one-time tip; "Got it" stores that it was
// dismissed (`writingDocsTipDismissed`, Rust-owned like the ask dates), and
// it never shows again. Only the site and whether the field gave any text
// are looked at: nothing of the text is kept or logged.

use std::time::{Duration, Instant};

/// The island's event: the tip, no payload.
pub const EVENT: &str = "writing-docs-tip";

/// A Google Docs document (Sheets and Slides are other editors).
pub const DOCS_SITES: [&str; 1] = ["docs.google.com/document"];

/// How long the field has to stay unreadable before the tip shows: a new,
/// empty document the user is only looking at shouldn't get it at once.
pub const AFTER: Duration = Duration::from_secs(3);

/// A field that gave no text to check: read-only to UI Automation, no text
/// pattern at all, or a pattern with nothing (or only spaces) in it. A
/// reading dropped for a card number is not "unreadable": it had text.
pub fn unreadable(pattern: &str, read_only: bool, text: Option<&str>) -> bool {
    read_only || pattern == "none" || text.is_some_and(|t| t.trim().is_empty())
}

/// What a reading in Docs leads to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Log one metadata line: an unreadable Docs field was just entered.
    pub log: bool,
    /// Show the tip now.
    pub offer: bool,
}

/// The tip's clock, in the capture thread: offered at most once per run,
/// and never once dismissed.
#[derive(Default)]
pub struct DocsTip {
    since: Option<Instant>,
    offered: bool,
}

impl DocsTip {
    /// `unreadable_docs`: this reading is a Docs field with no text to
    /// check (and the checker is on in the browser).
    pub fn feed(&mut self, unreadable_docs: bool, dismissed: bool, now: Instant) -> Step {
        if !unreadable_docs {
            self.since = None;
            return Step::default();
        }
        let log = self.since.is_none();
        let since = *self.since.get_or_insert(now);
        let offer = !dismissed && !self.offered && now.duration_since(since) >= AFTER;
        if offer {
            self.offered = true;
        }
        Step { log, offer }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_means_no_text_to_check() {
        assert!(unreadable("none", false, None));
        assert!(unreadable("skipped (read-only)", true, None));
        assert!(unreadable("TextPattern", false, Some("")));
        assert!(unreadable("TextPattern2", false, Some("  ")));
        assert!(!unreadable("TextPattern2", false, Some("Hi Sarah")));
        // Dropped for a card number: it had text, so no tip.
        assert!(!unreadable("withheld", false, None));
    }

    #[test]
    fn only_a_google_docs_document_counts() {
        use crate::capture::sensitive::url_on_sites;
        assert!(url_on_sites("https://docs.google.com/document/d/abc/edit", &DOCS_SITES));
        assert!(!url_on_sites("https://docs.google.com/spreadsheets/d/abc/edit", &DOCS_SITES));
        assert!(!url_on_sites("https://mail.google.com/mail/u/0/", &DOCS_SITES));
        assert!(!url_on_sites("https://evil.example/docs.google.com/document", &DOCS_SITES));
    }

    #[test]
    fn the_tip_waits_three_seconds_then_shows_once_per_run() {
        let t0 = Instant::now();
        let mut tip = DocsTip::default();
        assert_eq!(tip.feed(true, false, t0), Step { log: true, offer: false });
        assert_eq!(tip.feed(true, false, t0 + Duration::from_secs(2)), Step::default());
        assert_eq!(tip.feed(true, false, t0 + AFTER), Step { log: false, offer: true });
        assert_eq!(tip.feed(true, false, t0 + Duration::from_secs(9)), Step::default());
        // Leaving and coming back: logged again, never offered again this run.
        assert_eq!(tip.feed(false, false, t0 + Duration::from_secs(10)), Step::default());
        assert_eq!(tip.feed(true, false, t0 + Duration::from_secs(11)), Step { log: true, offer: false });
        assert_eq!(tip.feed(true, false, t0 + Duration::from_secs(20)), Step::default());
    }

    #[test]
    fn readable_text_restarts_the_wait() {
        let t0 = Instant::now();
        let mut tip = DocsTip::default();
        tip.feed(true, false, t0);
        tip.feed(false, false, t0 + Duration::from_secs(2));
        assert!(!tip.feed(true, false, t0 + Duration::from_secs(4)).offer);
        assert!(tip.feed(true, false, t0 + Duration::from_secs(7)).offer);
    }

    #[test]
    fn once_dismissed_it_never_shows() {
        let t0 = Instant::now();
        let mut tip = DocsTip::default();
        tip.feed(true, true, t0);
        assert!(!tip.feed(true, true, t0 + Duration::from_secs(60)).offer);
    }
}
