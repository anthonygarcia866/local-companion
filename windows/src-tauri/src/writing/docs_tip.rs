// Google Docs is not supported yet (Decision D, 2026-10-17): Docs draws its
// pages on a canvas and types into a hidden helper field, so UI Automation
// never gives Glim the document's text, even with screen reader support on
// (Anthony's tests 2026-10-14 and 2026-10-16; docs/capture-results.md, "Google
// Docs"). Glim reads nothing in a Docs document (capture::uia::read skips it)
// and shows a one-time notice, "Glim can't read Google Docs yet", once a Docs
// document has had the focus for AFTER. "Got it" stores that it was dismissed
// (`writingDocsTipDismissed`, Rust-owned like the ask dates); "Not now" just
// closes it (it may come back next run). Only the site is looked at.

use std::time::{Duration, Instant};

/// The island's event: the notice, no payload.
pub const EVENT: &str = "writing-docs-tip";

/// A Google Docs document (Sheets and Slides are other editors).
pub const DOCS_SITES: [&str; 1] = ["docs.google.com/document"];

/// How long a Docs document has to keep the focus before the notice shows:
/// passing through a Docs tab shouldn't get it at once.
pub const AFTER: Duration = Duration::from_secs(3);

/// What the capture panel and the log call a Docs field: nothing is read.
pub const SKIPPED_DOCS: &str = "skipped (Google Docs)";

/// What a reading in Docs leads to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Log one metadata line: a Docs document just got the focus.
    pub log: bool,
    /// Show the notice now.
    pub offer: bool,
}

/// The notice's clock, in the capture thread: offered at most once per run,
/// and never once dismissed.
#[derive(Default)]
pub struct DocsTip {
    since: Option<Instant>,
    offered: bool,
}

impl DocsTip {
    /// `in_docs`: the focus is in a Google Docs document (and the checker
    /// is on in the browser). Anything in the document counts, so typing
    /// never restarts the wait (what kept the old tip from ever showing).
    pub fn feed(&mut self, in_docs: bool, dismissed: bool, now: Instant) -> Step {
        if !in_docs {
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
    fn leaving_docs_restarts_the_wait() {
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
