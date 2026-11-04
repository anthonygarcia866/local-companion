// The cost and latency budget (docs/glim-learns-plan.md §6): the last 50
// Ask Glim requests (the Ask key, answers to Glim's questions, draft
// changes), in memory only: how long each took and what it cost, never
// text. Every 50th one, and on quit, one log line with the medians against
// the plan's limits:
//
//   budget: last 50 ask median 1.3s $0.000400/request
//   budget: last 50 ask median 2.4s $0.000400/request OVER
//
// The limits are half of the 4 s cancel (docs/voice-actions-plan.md §4.1)
// and a tenth of a cent. They are constants here, easy to change.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many requests the medians cover.
pub const WINDOW: usize = 50;
/// The median time an Ask request may take: half of the 4 s cancel.
pub const MAX_MEDIAN_MS: u64 = 2_000;
/// The median cost an Ask request may have: $0.001.
pub const MAX_MEDIAN_NANOS: u64 = 1_000_000;

pub struct Budget {
    /// What the line calls these requests ("ask", "live").
    label: &'static str,
    /// The limits a median may reach.
    max_ms: u64,
    max_nanos: u64,
    /// (milliseconds, nano-dollars), oldest first.
    recent: VecDeque<(u64, u64)>,
    /// Requests noted since launch, to say every 50th.
    count: u64,
}

impl Budget {
    /// Notes one request; on every 50th, the line to log.
    pub fn note(&mut self, ms: u64, nanos: u64) -> Option<String> {
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back((ms, nanos));
        self.count += 1;
        self.count.is_multiple_of(WINDOW as u64).then(|| self.line()).flatten()
    }

    /// The line for what is noted now; None before any request.
    pub fn line(&self) -> Option<String> {
        let ms = median(self.recent.iter().map(|r| r.0).collect())?;
        let nanos = median(self.recent.iter().map(|r| r.1).collect())?;
        let over = ms > self.max_ms || nanos > self.max_nanos;
        Some(format!(
            "budget: last {} {} median {:.1}s ${:.6}/request{}",
            self.recent.len(),
            self.label,
            ms as f64 / 1000.0,
            nanos as f64 / 1e9,
            if over { " OVER" } else { "" }
        ))
    }
}

/// The middle value; for an even count, the mean of the two middle ones
/// (rounded down). None for none.
pub fn median(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 { values[mid] } else { (values[mid - 1] + values[mid]) / 2 })
}

impl Budget {
    /// The Ask lane's, with the plan's limits.
    pub const fn ask() -> Budget {
        Budget { label: "ask", max_ms: MAX_MEDIAN_MS, max_nanos: MAX_MEDIAN_NANOS, recent: VecDeque::new(), count: 0 }
    }

    /// Live revisions': sent to shown within 1.5 s, a tenth of a cent.
    pub const fn live() -> Budget {
        Budget { label: "live", max_ms: LIVE_MAX_MEDIAN_MS, max_nanos: MAX_MEDIAN_NANOS, recent: VecDeque::new(), count: 0 }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Budget::ask()
    }
}

/// Live revisions: the median from a pause's request going out to its
/// revision shown (owner's target, 2026-11-03).
pub const LIVE_MAX_MEDIAN_MS: u64 = 1_500;

static BUDGET: Mutex<Budget> = Mutex::new(Budget::ask());
static LIVE: Mutex<Budget> = Mutex::new(Budget::live());

/// One live revision shown: sent-to-answer time and cost. Every 50th, and
/// on quit, the medians' line ("budget: last 50 live median …").
pub fn note_live(ms: u64, nanos: u64) {
    let line = LIVE.lock().unwrap_or_else(|e| e.into_inner()).note(ms, nanos);
    if let Some(line) = line {
        crate::log::line(line);
    }
}

/// One Ask-lane request to Claude that came back with its usage.
pub fn note_ask(ms: u64, nanos: u64) {
    let line = BUDGET.lock().unwrap().note(ms, nanos);
    if let Some(line) = line {
        crate::log::line(line);
    }
}

/// On quit: the line for what was noted, if anything was.
pub fn on_quit() {
    let line = BUDGET.lock().unwrap().line();
    if let Some(line) = line {
        crate::log::line(line);
    }
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner()).line();
    if let Some(line) = live {
        crate::log::line(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_of_odd_even_and_empty_windows() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![7]), Some(7));
        assert_eq!(median(vec![3, 1, 2]), Some(2));
        assert_eq!(median(vec![4, 1, 3, 2]), Some(2));
        assert_eq!(median(vec![10, 20]), Some(15));
        assert_eq!(median(vec![1, 1, 1, 100]), Some(1), "one slow request doesn't move it");
    }

    #[test]
    fn the_window_keeps_the_last_fifty_and_drops_the_oldest() {
        let mut b = Budget::default();
        // 50 slow, expensive requests, then 50 quick, cheap ones.
        for _ in 0..WINDOW {
            b.note(5_000, 5_000_000);
        }
        assert!(b.line().unwrap().ends_with(" OVER"));
        for _ in 0..WINDOW {
            b.note(800, 300_000);
        }
        assert_eq!(b.recent.len(), WINDOW);
        assert_eq!(b.line().unwrap(), "budget: last 50 ask median 0.8s $0.000300/request");
    }

    #[test]
    fn a_line_comes_every_fiftieth_request() {
        let mut b = Budget::default();
        assert_eq!(b.line(), None);
        for i in 1..=120u64 {
            let line = b.note(1_000, 400_000);
            assert_eq!(line.is_some(), i % 50 == 0, "request {i}");
        }
        assert_eq!(b.line().unwrap(), "budget: last 50 ask median 1.0s $0.000400/request");
    }

    #[test]
    fn live_revisions_have_their_own_line_and_a_1_5_s_target() {
        let mut b = Budget::live();
        b.note(1_400, 30_000);
        assert_eq!(b.line().unwrap(), "budget: last 1 live median 1.4s $0.000030/request");
        b.note(1_700, 30_000);
        b.note(1_800, 30_000);
        assert!(b.line().unwrap().ends_with(" OVER"), "median 1.7 s is over 1.5 s");
    }

    #[test]
    fn over_either_limit_is_flagged() {
        let mut b = Budget::default();
        b.note(MAX_MEDIAN_MS, MAX_MEDIAN_NANOS);
        assert!(!b.line().unwrap().contains("OVER"), "at the limit is within it");
        let mut slow = Budget::default();
        slow.note(MAX_MEDIAN_MS + 1, 1);
        assert!(slow.line().unwrap().ends_with(" OVER"));
        let mut dear = Budget::default();
        dear.note(1, MAX_MEDIAN_NANOS + 1);
        assert!(dear.line().unwrap().ends_with(" OVER"));
        assert_eq!(dear.line().unwrap(), "budget: last 1 ask median 0.0s $0.001000/request OVER");
    }
}
