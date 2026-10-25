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

#[derive(Default)]
pub struct Budget {
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
        (self.count % WINDOW as u64 == 0).then(|| self.line()).flatten()
    }

    /// The line for what is noted now; None before any request.
    pub fn line(&self) -> Option<String> {
        let ms = median(self.recent.iter().map(|r| r.0).collect())?;
        let nanos = median(self.recent.iter().map(|r| r.1).collect())?;
        let over = ms > MAX_MEDIAN_MS || nanos > MAX_MEDIAN_NANOS;
        Some(format!(
            "budget: last {} ask median {:.1}s ${:.6}/request{}",
            self.recent.len(),
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

static BUDGET: Mutex<Budget> = Mutex::new(Budget { recent: VecDeque::new(), count: 0 });

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
