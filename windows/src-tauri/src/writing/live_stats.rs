// Live revisions are never silent (owner, 2026-11-07): what became of them,
// as counts only. The last revision shown (when, in which app) and, over the
// last hour, why they didn't run, per reason (Settings → Writing → Live
// revisions status), and the same counts in a log line every 5 minutes.
// Never text: reasons and outcomes are fixed codes, apps are their short
// names (as the log already says them).
//
// A skip is counted once per stay: the same reason again in the same field
// is the same skip (the capture loop reads about three times a second).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// How often the counts go to the log.
pub const LOG_EVERY: Duration = Duration::from_secs(300);
/// Settings shows the last hour.
pub const WINDOW: Duration = Duration::from_secs(3600);
/// Events kept at most (an hour of the busiest typing is far fewer).
const KEEP: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Live revisions didn't run, and why.
    Skip,
    /// A request went out, or what became of it.
    Result,
    /// Where a shown revision's preview was placed (caret | field | island).
    Placed,
}

#[derive(Default)]
pub struct Stats {
    events: VecDeque<(Instant, Kind, String)>,
    /// The last revision shown: when (a clock time), the app, and its Instant.
    last_shown: Option<(Instant, String, String)>,
    /// The reason last counted in this stay, so it's counted once.
    streak: Option<String>,
}

/// What Settings shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "08:33", or "" when nothing was shown since Glim started.
    pub last_at: String,
    pub last_app: String,
    pub minutes_ago: u64,
    /// The last hour's skips, most first: (reason, count).
    pub skipped: Vec<(String, u32)>,
    /// The last hour's requests and outcomes: ("sent", n), ("shown", n) …
    pub results: Vec<(String, u32)>,
}

impl Stats {
    fn push(&mut self, at: Instant, kind: Kind, what: &str) {
        if self.events.len() == KEEP {
            self.events.pop_front();
        }
        self.events.push_back((at, kind, what.to_string()));
    }

    /// Live revisions didn't run here because of `reason` (counted once per stay).
    pub fn skip(&mut self, at: Instant, reason: &str) {
        if self.streak.as_deref() == Some(reason) {
            return;
        }
        self.streak = Some(reason.to_string());
        self.push(at, Kind::Skip, reason);
    }

    /// Another field, or a request went out: the next skip counts again.
    pub fn fresh(&mut self) {
        self.streak = None;
    }

    /// A request went out ("sent"), or what came of it ("unchanged", "failed" …).
    pub fn result(&mut self, at: Instant, what: &str) {
        self.streak = None;
        self.push(at, Kind::Result, what);
    }

    /// A shown revision's preview went by `how` ("caret", "field", "island").
    pub fn placed(&mut self, at: Instant, how: &str) {
        self.push(at, Kind::Placed, how);
    }

    /// A revision was shown, at clock time `clock` ("08:33"), in `app`.
    pub fn shown(&mut self, at: Instant, clock: &str, app: &str) {
        self.result(at, "shown");
        self.last_shown = Some((at, clock.to_string(), app.to_string()));
    }

    fn counts(&self, now: Instant, within: Duration, kind: Kind) -> Vec<(String, u32)> {
        let mut map: BTreeMap<&str, u32> = BTreeMap::new();
        for (at, k, what) in &self.events {
            if *k == kind && now.saturating_duration_since(*at) < within {
                *map.entry(what.as_str()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, u32)> = map.into_iter().map(|(w, n)| (w.to_string(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    pub fn status(&self, now: Instant) -> Status {
        let (last_at, last_app, minutes_ago) = match &self.last_shown {
            Some((at, clock, app)) => (clock.clone(), app.clone(), now.saturating_duration_since(*at).as_secs() / 60),
            None => (String::new(), String::new(), 0),
        };
        Status { last_at, last_app, minutes_ago, skipped: self.counts(now, WINDOW, Kind::Skip), results: self.counts(now, WINDOW, Kind::Result) }
    }

    /// The log's line for the last `within`: counts only.
    pub fn log_line(&self, now: Instant, within: Duration) -> String {
        let join = |v: Vec<(String, u32)>| v.iter().map(|(w, n)| format!("{w} {n}")).collect::<Vec<_>>().join(", ");
        let (results, skipped) = (self.counts(now, within, Kind::Result), self.counts(now, within, Kind::Skip));
        let placed = self.counts(now, within, Kind::Placed);
        let mins = within.as_secs() / 60;
        let mut line = match (results.is_empty(), skipped.is_empty()) {
            (true, true) => format!("live: last {mins} min: nothing"),
            (false, true) => format!("live: last {mins} min: {}", join(results)),
            (true, false) => format!("live: last {mins} min: skipped: {}", join(skipped)),
            (false, false) => format!("live: last {mins} min: {}; skipped: {}", join(results), join(skipped)),
        };
        if !placed.is_empty() {
            line.push_str(&format!("; placed: {}", join(placed)));
        }
        line
    }
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

/// The running Glim's counts.
pub fn with<T>(f: impl FnOnce(&mut Stats) -> T) -> T {
    let mut g = STATS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Stats::default))
}

/// Settings → Writing → Live revisions status.
pub fn status() -> Status {
    with(|s| s.status(Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_skip_counts_once_per_stay_and_again_in_the_next_field() {
        let t0 = Instant::now();
        let mut st = Stats::default();
        for i in 0..10 {
            st.skip(t0 + s(i), "password");
        }
        st.fresh();
        st.skip(t0 + s(11), "password");
        st.skip(t0 + s(12), "too-short");
        st.skip(t0 + s(13), "too-short");
        assert_eq!(st.status(t0 + s(20)).skipped, vec![("password".to_string(), 2), ("too-short".to_string(), 1)]);
    }

    #[test]
    fn the_status_has_the_last_suggestion_and_only_the_last_hours_counts() {
        let t0 = Instant::now();
        let mut st = Stats::default();
        assert_eq!(st.status(t0).last_at, "");
        st.skip(t0, "app-not-allowed");
        st.result(t0 + s(10), "sent");
        st.shown(t0 + s(12), "08:33", "Outlook");
        let later = st.status(t0 + s(12 + 120));
        assert_eq!((later.last_at.as_str(), later.last_app.as_str(), later.minutes_ago), ("08:33", "Outlook", 2));
        assert_eq!(later.results, vec![("sent".to_string(), 1), ("shown".to_string(), 1)]);
        // An hour on, the counts are gone but the last suggestion stays.
        let hour = st.status(t0 + s(3700));
        assert!(hour.skipped.is_empty() && hour.results.is_empty());
        assert_eq!(hour.last_app, "Outlook");
    }

    #[test]
    fn the_log_line_is_counts_only() {
        let t0 = Instant::now();
        let mut st = Stats::default();
        assert_eq!(st.log_line(t0, LOG_EVERY), "live: last 5 min: nothing");
        st.result(t0, "sent");
        st.result(t0, "unchanged");
        st.skip(t0, "too-short");
        st.skip(t0, "password");
        assert_eq!(st.log_line(t0 + s(1), LOG_EVERY), "live: last 5 min: sent 1, unchanged 1; skipped: password 1, too-short 1");
        assert_eq!(st.log_line(t0 + s(400), LOG_EVERY), "live: last 5 min: nothing");
    }

    #[test]
    fn the_log_line_names_where_previews_were_placed() {
        let t0 = Instant::now();
        let mut st = Stats::default();
        st.result(t0, "sent");
        st.shown(t0, "08:33", "Claude");
        st.placed(t0, "field");
        st.result(t0, "sent");
        st.shown(t0, "08:34", "Outlook");
        st.placed(t0, "caret");
        st.placed(t0, "field");
        assert_eq!(st.log_line(t0 + s(1), LOG_EVERY), "live: last 5 min: sent 2, shown 2; placed: field 2, caret 1");
        // Settings' status doesn't list placements.
        assert_eq!(st.status(t0 + s(1)).results, vec![("sent".to_string(), 2), ("shown".to_string(), 2)]);
    }

    #[test]
    fn this_module_never_logs_or_writes() {
        let src = include_str!("live_stats.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for w in ["log::line", "std::fs", "println!", "eprintln!"] {
            assert!(!code.contains(w), "{w}");
        }
    }
}
