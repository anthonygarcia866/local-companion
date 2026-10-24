// The calendar glance's thread: a COM STA that reads Outlook's calendar
// every minute (sooner when Settings change), keeps what it read in
// memory, and sends the island what it shows ("calendar"). The meeting links
// stay here: the island asks to join an event by (generation, index), and
// only a checked Teams/Zoom link is opened (calendar::meeting_link).
//
// Every path ends in the same loop: Outlook open at launch, opened later,
// closed and reopened, or the PC waking from sleep. The next read is a
// deadline (Due) that only a read moves: a Settings list or a Join in
// between never pushes it back (the 2026-10-24 bug: each Settings list
// restarted a 5-minute wait, so a meeting made 12 minutes ahead was read
// too late to glow).
//
// The log gets counts, codes and minutes; never a title, a location, a
// calendar's name or a link. A read is logged only when its counts change.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::{in_window, next, summary_due, today, Event, Glance, ShownEvent, WINDOW_MS};

pub const EVENT: &str = "calendar";
/// To the Settings window: the number of calendars in the running Outlook
/// changed (found after a retry, Outlook started later, or it closed), so
/// Settings → Calendar asks for the list again.
pub const FOLDERS_EVENT: &str = "calendar-folders";
/// How often Outlook is read (a meeting made a few minutes ahead still
/// glows at its lead time).
pub const EVERY: Duration = Duration::from_secs(60);
/// A gap between reads longer than this (the PC slept) is logged once.
const GAP: Duration = Duration::from_secs(3 * 60);
/// While Outlook is running but answers no calendars yet (starting, its
/// profile loading, not yet reachable through COM): look again after these,
/// then every EVERY. Zero is never taken as final while Outlook runs.
pub const BACKOFF: [Duration; 3] = [Duration::from_secs(10), Duration::from_secs(30), Duration::from_secs(60)];
/// While Outlook isn't running: how often its process is looked for (the
/// process list only; no COM, nothing logged). Glim never starts it.
pub const WATCH: Duration = Duration::from_secs(30);
/// Classic Outlook's process.
const OUTLOOK_EXE: &str = "OUTLOOK.EXE";

/// What one look at Outlook found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// OUTLOOK.EXE isn't running: nothing to try.
    NotRunning,
    /// Outlook runs but gave no calendars (couldn't attach yet, or 0).
    Zero,
    /// Outlook runs and has this many calendars open.
    Found(usize),
}

/// `running`: OUTLOOK.EXE is up. `calendars`: how many it answered, None if
/// Glim couldn't attach.
pub fn classify(running: bool, calendars: Option<usize>) -> Look {
    match (running, calendars) {
        (false, _) => Look::NotRunning,
        (true, Some(n)) if n > 0 => Look::Found(n),
        (true, _) => Look::Zero,
    }
}

/// What the worker does after one look.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    /// When to look again.
    pub wait: Duration,
    /// A log line (counts only), if there is something new to say.
    pub log: Option<String>,
    /// The calendar count changed: Settings asks for its list again.
    pub folders_changed: bool,
}

/// When and why the worker looks at Outlook (pure; the worker feeds it what
/// it saw). Retries with BACKOFF while Outlook runs with no calendars, stays
/// quiet while Outlook isn't running.
#[derive(Debug, Default)]
pub struct Tracker {
    /// Looks since Outlook was first seen running with no calendars.
    attempt: u32,
    /// Calendars seen in the running Outlook (0 = none yet).
    known: usize,
    /// Settings showed an empty list: keep looking even with the glance off,
    /// so the list fills in when Outlook is ready.
    want_list: bool,
}

impl Tracker {
    /// Calendars seen in the running Outlook (0 = none yet).
    pub fn known(&self) -> usize {
        self.known
    }

    /// Whether to look at Outlook at all: the glance is on, or Settings is
    /// waiting for calendars.
    pub fn wanted(&self, glance_on: bool) -> bool {
        glance_on || self.want_list
    }

    /// One look's result.
    pub fn saw(&mut self, running: bool, calendars: Option<usize>) -> Step {
        let look = classify(running, calendars);
        let attempt = self.attempt + 1;
        match look {
            Look::NotRunning => {
                let changed = self.known != 0;
                self.known = 0;
                self.attempt = 0;
                Step { wait: WATCH, log: changed.then(|| "calendar: Outlook closed; waiting for it (Glim never starts it)".to_string()), folders_changed: changed }
            }
            Look::Zero => {
                let changed = self.known != 0;
                self.known = 0;
                let wait = BACKOFF.get(self.attempt as usize).copied().unwrap_or(EVERY);
                // The backoff's looks and the first slow one; then quiet.
                let log = (attempt as usize <= BACKOFF.len() + 1).then(|| format!("calendar: 0 found (attempt {attempt}), Outlook is running; next look in {} s", wait.as_secs()));
                self.attempt += 1;
                Step { wait, log, folders_changed: changed }
            }
            Look::Found(n) => {
                let changed = n != self.known;
                self.known = n;
                self.attempt = 0;
                if changed {
                    self.want_list = false;
                }
                Step { wait: EVERY, log: changed.then(|| format!("calendar: {n} found (attempt {attempt})")), folders_changed: changed }
            }
        }
    }

    /// Settings got `n` calendars. Empty: keep looking (sooner if Outlook
    /// runs) until some appear; returns the new wait then.
    pub fn listed(&mut self, n: usize, running: bool) -> Option<Duration> {
        if n > 0 {
            self.known = n;
            self.want_list = false;
            self.attempt = 0;
            return None;
        }
        self.want_list = true;
        Some(if running { BACKOFF[self.attempt.min(BACKOFF.len() as u32 - 1) as usize] } else { WATCH })
    }
}

/// When the next read is due, as time since the worker started (a
/// monotonic clock). Only a read sets it; a sooner retry can bring it
/// forward; nothing pushes it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due(pub Duration);

impl Due {
    /// After a read at `now` whose next look is `wait` away.
    pub fn after_read(now: Duration, wait: Duration) -> Due {
        Due(now + wait)
    }

    /// Settings wants a look within `wait` (an empty list): sooner only.
    pub fn sooner(self, now: Duration, wait: Duration) -> Due {
        Due(self.0.min(now + wait))
    }

    /// How long to wait for a job before reading: never past the deadline,
    /// and never more than EVERY, so a clock that stood still while the PC
    /// slept costs at most one more minute.
    pub fn timeout(self, now: Duration) -> Duration {
        self.0.saturating_sub(now).min(EVERY)
    }
}

/// The counts a read is logged by: events, and those with a meeting link.
pub fn counts(events: &[Event]) -> (usize, usize) {
    (events.len(), events.iter().filter(|e| e.join.is_some()).count())
}

/// A calendar Settings can offer: Outlook's folder path, its name, and the
/// mailbox (or data file) it belongs to, for the label.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarFolder {
    pub id: String,
    pub name: String,
    pub mailbox: String,
    pub default: bool,
}

/// The mailbox in a folder path: `\\me@example.com\Calendar\Team` → `me@example.com`.
pub fn mailbox_of(path: &str) -> String {
    path.trim_start_matches('\\').split('\\').next().unwrap_or("").to_string()
}

enum Job {
    Refresh,
    List(Sender<Vec<CalendarFolder>>),
    Join { generation: u64, index: usize },
}

static JOBS: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

fn post(app: &AppHandle, job: Job) {
    let tx = JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let app = app.clone();
        std::thread::spawn(move || worker(app, rx));
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(job);
}

/// Starts the thread (at launch) and reads now.
pub fn start(app: &AppHandle) {
    post(app, Job::Refresh);
}

/// Settings changed (on/off, calendars, all-day): read again now.
pub fn refresh(app: &AppHandle) {
    post(app, Job::Refresh);
}

/// The calendars Outlook has open, for Settings (empty if it isn't running).
pub async fn list(app: &AppHandle) -> Vec<CalendarFolder> {
    let (tx, rx) = channel();
    post(app, Job::List(tx));
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20)).unwrap_or_default()).await.unwrap_or_default()
}

/// Join: the meeting link of event `index` of read `generation`.
pub fn join(app: &AppHandle, generation: u64, index: usize) {
    post(app, Job::Join { generation, index });
}

/// What the settings say now.
struct Prefs {
    on: bool,
    hide_all_day: bool,
    summary: bool,
    ids: Vec<String>,
    summary_shown: String,
}

fn prefs(app: &AppHandle) -> Prefs {
    let s = app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().clone()).unwrap_or_default();
    Prefs { on: s.calendar_on, hide_all_day: s.calendar_hide_all_day, summary: s.calendar_summary, ids: s.calendar_ids.clone(), summary_shown: s.calendar_summary_shown.clone() }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The glance for the island from what was read (pure, tested).
pub fn glance(events: &[Event], generation: u64, summary: bool) -> Glance {
    Glance { available: true, code: "", events: events.iter().map(ShownEvent::of).collect(), generation, summary }
}

/// One log line: counts, the next event's minutes away, never a title.
pub fn log_line(events: &[Event], now: i64) -> String {
    let joins = events.iter().filter(|e| e.join.is_some()).count();
    let next_in = next(events, now).map(|(i, _)| format!(", next in {} min", (events[i].start_ms - now).max(0) / 60_000)).unwrap_or_default();
    format!("calendar: {} events in the next 24 h ({} with a meeting link){next_in}", events.len(), joins)
}

fn worker(app: AppHandle, rx: Receiver<Job>) {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let mut events: Vec<Event> = Vec::new();
    let mut generation = 0u64;
    let mut last_log = String::new();
    let mut tracker = Tracker::default();
    let started = Instant::now();
    let mut due = Due(Duration::ZERO);
    // Wall-clock time of the last read: a long gap (sleep) is logged once.
    let mut last_read_ms: Option<i64> = None;
    // The last "nothing to show" glance sent, so a quiet WATCH look while
    // Outlook is closed doesn't resend the same one every 30 s.
    let mut last_sent: Option<&'static str> = None;
    let mut job = Job::Refresh;
    loop {
        match job {
            Job::Refresh => {
                let p = prefs(&app);
                let mut wait = EVERY;
                let wall = now_ms();
                if let Some(gap) = last_read_ms.map(|t| wall - t).filter(|g| *g > GAP.as_millis() as i64) {
                    crate::log::line(format!("calendar: reading again after {} min without a read (sleep?)", gap / 60_000));
                }
                last_read_ms = Some(wall);
                // Outlook's process first (no COM while it isn't running),
                // then attach; 0 is retried, never final, while it runs.
                let mut attached = Err(super::com::ComError::NotRunning);
                if tracker.wanted(p.on) {
                    let running = crate::platform::process_running(OUTLOOK_EXE);
                    if running {
                        attached = super::com::Calendar::attach();
                    }
                    let count = attached.as_ref().ok().map(|c| if tracker.known() > 0 { tracker.known() } else { c.calendars().len() });
                    let step = tracker.saw(running, count);
                    if let Some(line) = step.log {
                        crate::log::line(&line);
                    }
                    if step.folders_changed {
                        let _ = app.emit_to("settings", FOLDERS_EVENT, tracker.known());
                    }
                    wait = step.wait;
                }
                due = Due::after_read(started.elapsed(), wait);
                generation += 1;
                let g = if !p.on {
                    events.clear();
                    Glance { code: "off", generation, ..Default::default() }
                } else {
                    match attached {
                        Err(e) => {
                            events.clear();
                            Glance { code: e.code(), generation, ..Default::default() }
                        }
                        Ok(cal) => {
                            let now = now_ms();
                            let offset = super::com::utc_offset_minutes();
                            match cal.events(&p.ids, now, now + WINDOW_MS, offset) {
                                Ok(read) => {
                                    events = in_window(read, now, p.hide_all_day);
                                    let due = summary_due(p.summary, &p.summary_shown, now, offset) && !today(&events, now, offset).is_empty();
                                    if due {
                                        crate::calendar_summary_shown(&app, &super::local_date(now, offset));
                                    }
                                    // Logged when the counts change, not every minute.
                                    let (n, joins) = counts(&events);
                                    let key = format!("read {n}/{joins}");
                                    if key != last_log {
                                        crate::log::line(log_line(&events, now));
                                        last_log = key;
                                    }
                                    glance(&events, generation, due)
                                }
                                Err(e) => {
                                    events.clear();
                                    Glance { code: e.code(), generation, ..Default::default() }
                                }
                            }
                        }
                    }
                };
                if !g.available && last_log != g.code {
                    crate::log::line(format!("calendar: nothing to show ({})", g.code));
                    last_log = g.code.to_string();
                }
                if g.available || last_sent != Some(g.code) {
                    last_sent = (!g.available).then_some(g.code);
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, &g);
                }
            }
            Job::List(reply) => {
                let running = crate::platform::process_running(OUTLOOK_EXE);
                let attached = if running { super::com::Calendar::attach().ok() } else { None };
                let found: Vec<CalendarFolder> = match attached {
                    Some(cal) => {
                        let default = cal.default_calendar();
                        cal.calendars().into_iter().map(|(id, name)| CalendarFolder { default: Some(&id) == default.as_ref(), mailbox: mailbox_of(&id), id, name }).collect()
                    }
                    None => Vec::new(),
                };
                crate::log::line(format!("calendar: {} calendars found for Settings{}", found.len(), if found.is_empty() && running { " (Outlook is running; looking again)" } else { "" }));
                // Never pushes the next read back; an empty list may bring it forward.
                if let Some(w) = tracker.listed(found.len(), running) {
                    due = due.sooner(started.elapsed(), w);
                }
                let _ = reply.send(found);
            }
            Job::Join { generation: g, index } => {
                // Only the read the island is showing, only a checked link.
                let link = (g == generation).then(|| events.get(index)).flatten().and_then(|e| e.join.clone()).filter(|l| super::meeting_link(l));
                match link {
                    Some(url) => {
                        crate::log::line("calendar: join opened in the browser");
                        crate::platform::open_url(&url);
                    }
                    None => crate::log::line("calendar: join refused (no checked link for that event)"),
                }
            }
        }
        job = loop {
            match rx.recv_timeout(due.timeout(started.elapsed())) {
                Ok(j) => break j,
                // Capped at EVERY: not due yet (the clock stood still in
                // sleep, or the cap) → wait again for the rest.
                Err(RecvTimeoutError::Timeout) if due.timeout(started.elapsed()) > Duration::ZERO => continue,
                Err(RecvTimeoutError::Timeout) => break Job::Refresh,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_has_counts_and_minutes_never_a_title_or_link() {
        let now = 1_792_249_200_000;
        let e = |t: &str, start: i64, join: Option<&str>| Event {
            title: t.into(),
            location: "Room 9F".into(),
            start_ms: start,
            end_ms: start + 1_800_000,
            all_day: false,
            calendar: "\\\\me@example.com\\Calendar".into(),
            join: join.map(str::to_string),
        };
        let events = vec![e("Rent review with Jordan", now + 42 * 60_000, Some("https://zoom.us/j/1")), e("Lunch", now + 3 * 3_600_000, None)];
        let line = log_line(&events, now);
        assert_eq!(line, "calendar: 2 events in the next 24 h (1 with a meeting link), next in 42 min");
        for secret in ["Jordan", "Lunch", "9F", "zoom", "example.com"] {
            assert!(!line.contains(secret), "{secret}");
        }
        let g = glance(&events, 3, false);
        assert!(g.available && g.events.len() == 2 && g.generation == 3);
        assert!(!serde_json::to_string(&g).unwrap().contains("zoom.us"), "links stay in Rust");
        // The worker never logs what it read, and joins only a checked link.
        let src = include_str!("worker.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(code.contains(".filter(|l| super::meeting_link(l))"));
        assert!(!code.contains("log::line(format!(\"calendar: join {"), "the link is never logged");
    }

    /// A fake Outlook over time: (running, calendars it answers) at second t.
    /// Drives the real Tracker the way the worker does, from t = 0 (launch)
    /// to `until`, and returns each look: (second, look, log, folders_changed).
    fn run(outlook: impl Fn(u64) -> (bool, Option<usize>), until: u64) -> Vec<(u64, Look, Option<String>, bool)> {
        let mut tracker = Tracker::default();
        let mut t = 0;
        let mut out = Vec::new();
        while t <= until {
            let (running, calendars) = outlook(t);
            let step = tracker.saw(running, calendars);
            out.push((t, classify(running, calendars), step.log, step.folders_changed));
            t += step.wait.as_secs();
        }
        out
    }

    #[test]
    fn zero_at_startup_is_retried_until_the_calendars_show() {
        // The 22:45 case: Outlook is running but answers nothing for the
        // first ~25 s after Glim starts, then has 9 calendars.
        let looks = run(|t| (true, if t >= 25 { Some(9) } else { None }), 400);
        let times: Vec<u64> = looks.iter().map(|l| l.0).collect();
        assert_eq!(&times[..4], &[0, 10, 40, 100], "10 s, 30 s, then the normal 1 min");
        assert_eq!(looks[0].1, Look::Zero);
        assert_eq!(looks[2].1, Look::Found(9));
        assert_eq!(looks[0].2.as_deref(), Some("calendar: 0 found (attempt 1), Outlook is running; next look in 10 s"));
        assert_eq!(looks[2].2.as_deref(), Some("calendar: 9 found (attempt 3)"));
        assert!(looks[2].3, "Settings is told to ask again");
        assert_eq!(looks[3].2, None, "the same 9 again: nothing new to log");
        assert!(!looks[3].3);
    }

    #[test]
    fn zero_while_outlook_runs_backs_off_then_every_minute_and_goes_quiet() {
        let looks = run(|_| (true, Some(0)), 400);
        let times: Vec<u64> = looks.iter().map(|l| l.0).collect();
        assert_eq!(times, vec![0, 10, 40, 100, 160, 220, 280, 340, 400]);
        assert_eq!(looks.iter().filter(|l| l.2.is_some()).count(), 4, "the backoff and the first slow look are logged, then quiet");
        assert!(looks.iter().all(|l| l.1 == Look::Zero && !l.3));
    }

    #[test]
    fn outlook_not_running_means_no_retries_and_nothing_logged() {
        let looks = run(|_| (false, None), 600);
        assert!(looks.iter().all(|l| l.1 == Look::NotRunning && l.2.is_none() && !l.3), "quiet");
        assert!(looks.windows(2).all(|w| w[1].0 - w[0].0 == WATCH.as_secs()), "only the process list, every 30 s");
        // The glance off and Settings not waiting: the worker doesn't look at all.
        let t = Tracker::default();
        assert!(!t.wanted(false));
        assert!(t.wanted(true));
    }

    #[test]
    fn outlook_started_after_glim_is_found() {
        // Opened at 95 s; reachable through COM only from 125 s.
        let looks = run(|t| (t >= 95, if t >= 125 { Some(9) } else { None }), 500);
        let found = looks.iter().find(|l| l.1 == Look::Found(9)).expect("found");
        assert!(found.0 <= 140, "found within ~45 s of Outlook starting: {}", found.0);
        assert_eq!(found.2.as_deref(), Some("calendar: 9 found (attempt 2)"));
        assert!(found.3);
        assert!(looks.iter().take_while(|l| l.0 < 95).all(|l| l.2.is_none()), "nothing logged while it was closed");
    }

    #[test]
    fn outlook_closing_tells_settings_and_a_restart_is_found_again() {
        let looks = run(|t| if (200..400).contains(&t) { (false, None) } else { (true, Some(9)) }, 800);
        let closed = looks.iter().find(|l| l.1 == Look::NotRunning).unwrap();
        assert!(closed.3, "Settings refreshes to empty");
        let back = looks.iter().find(|l| l.0 >= 400 && l.1 == Look::Found(9)).unwrap();
        assert!(back.3 && back.0 <= 430);
    }

    #[test]
    fn an_empty_settings_list_keeps_the_worker_looking_until_calendars_appear() {
        let mut t = Tracker::default();
        assert_eq!(t.listed(0, true), Some(BACKOFF[0]), "Outlook running: soon");
        assert!(t.wanted(false), "even with the glance off");
        let step = t.saw(true, Some(9));
        assert!(step.folders_changed, "Settings asks again and fills its list");
        assert!(!t.wanted(false), "done: back to quiet");
        let mut closed = Tracker::default();
        assert_eq!(closed.listed(0, false), Some(WATCH), "Outlook closed: watch for it");
        assert_eq!(closed.listed(9, true), None);
        assert_eq!(closed.known(), 9);
    }

    #[test]
    fn the_worker_never_starts_outlook_and_checks_its_process_before_com() {
        let src = include_str!("worker.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(code.contains("crate::platform::process_running(OUTLOOK_EXE)"));
        assert!(code.contains("if running {\n                        attached = super::com::Calendar::attach();"), "COM only while Outlook runs");
        assert!(code.contains("rx.recv_timeout(due.timeout(started.elapsed()))"), "the next look is a deadline");
        for banned in ["ShellExecute", "Command::new", "open_known_app"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }

    #[test]
    fn only_a_read_moves_the_deadline() {
        let s = Duration::from_secs;
        // The worker: only a read sets `due`; a Settings list can only bring it forward.
        let src = include_str!("worker.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert_eq!(code.matches("due = Due::after_read(").count(), 1, "set by a read only");
        assert!(code.contains("due = due.sooner(started.elapsed(), w);"));
        let due = Due::after_read(s(100), EVERY);
        assert_eq!(due, Due(s(160)));
        assert_eq!(due.sooner(s(130), s(10)), Due(s(140)));
        assert_eq!(due.sooner(s(130), EVERY), due, "a later wish never pushes it back");
        assert_eq!(due.timeout(s(130)), s(30));
        assert_eq!(due.timeout(s(7_300)), Duration::ZERO, "after a long sleep: read at once");
        assert_eq!(Due(s(10_000)).timeout(s(0)), EVERY, "never waits more than a minute at a time");
    }

    #[test]
    fn the_mailbox_comes_from_the_folder_path() {
        assert_eq!(mailbox_of("\\\\me@example.com\\Calendar"), "me@example.com");
        assert_eq!(mailbox_of("\\\\shared@example.com\\Calendar\\Team"), "shared@example.com");
        assert_eq!(mailbox_of("\\\\Archive\\Calendar"), "Archive");
        assert_eq!(mailbox_of(""), "");
    }

    /// A fake Outlook over time, for the worker's whole loop.
    struct Fake {
        /// Is OUTLOOK.EXE running at second t.
        running: fn(u64) -> bool,
        /// Calendars answered through COM at second t (None = can't attach yet).
        calendars: fn(u64) -> Option<usize>,
        /// Events in the calendar at second t: (created at, starts at), seconds.
        events: Vec<(u64, u64)>,
    }

    /// Drives the worker's own Tracker and Due the way `worker` does, with
    /// Settings asking for its list at `lists` (seconds), from launch to
    /// `until`. Returns each read that got events: (second, events' starts).
    fn simulate(fake: &Fake, lists: &[u64], until: u64) -> Vec<(u64, Vec<u64>)> {
        let mut tracker = Tracker::default();
        let mut due = Due(Duration::ZERO);
        let mut reads = Vec::new();
        let mut lists = lists.to_vec();
        lists.sort_unstable();
        let mut lists = lists.into_iter().peekable();
        let mut t = 0u64;
        while t <= until {
            let now = Duration::from_secs(t);
            if due.timeout(now).is_zero() {
                // Job::Refresh (the glance is on).
                let running = (fake.running)(t);
                let attached = if running { (fake.calendars)(t) } else { None };
                let known = tracker.known();
                let step = tracker.saw(running, attached.map(|c| if known > 0 { known } else { c }));
                due = Due::after_read(now, step.wait);
                if attached.is_some_and(|c| c > 0) {
                    reads.push((t, fake.events.iter().filter(|(made, _)| *made <= t).map(|(_, start)| *start).collect()));
                }
            }
            while lists.peek().is_some_and(|l| *l == t) {
                lists.next();
                // Job::List: Settings asks; never pushes the read back.
                let running = (fake.running)(t);
                let n = if running { (fake.calendars)(t).unwrap_or(0) } else { 0 };
                if let Some(w) = tracker.listed(n, running) {
                    due = due.sooner(now, w);
                }
            }
            t += 1;
        }
        reads
    }

    /// The largest gap between reads from second `from` on.
    fn max_gap(reads: &[(u64, Vec<u64>)], from: u64) -> u64 {
        let times: Vec<u64> = reads.iter().map(|r| r.0).filter(|t| *t >= from).collect();
        times.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0)
    }

    #[test]
    fn the_2026_10_24_case_settings_lists_no_longer_starve_the_reads() {
        // Glim before Outlook; Outlook reachable at 40 s; Settings asked for
        // its list at 41 s, 71 s, 353 s and 515 s (as in that log), then every 30 s.
        let mut lists = vec![41, 71, 353, 515];
        lists.extend((540..1_800).step_by(30));
        let fake = Fake { running: |t| t >= 5, calendars: |t| (t >= 40).then_some(9), events: vec![(0, 100_000), (150, 150 + 12 * 60)] };
        let reads = simulate(&fake, &lists, 1_800);
        assert!(reads.len() >= 28, "a read every minute: {}", reads.len());
        assert_eq!(max_gap(&reads, 0), EVERY.as_secs(), "lists never push a read back");
        // The meeting made at 150 s for 12 minutes later is read within a
        // minute, before every lead time but the 15-minute one starts.
        let seen = reads.iter().find(|r| r.1.contains(&(150 + 12 * 60))).expect("read").0;
        assert!(seen <= 150 + EVERY.as_secs(), "seen at {seen}");
        for lead_min in [5, 10] {
            assert!(seen <= 150 + 12 * 60 - lead_min * 60, "glows at its {lead_min}-minute lead time");
        }
    }

    #[test]
    fn outlook_absent_then_present_then_reads_continue() {
        let fake = Fake { running: |t| t >= 95, calendars: |t| (t >= 125).then_some(9), events: vec![(0, 3_000)] };
        let reads = simulate(&fake, &[], 1_200);
        let first = reads.first().expect("found").0;
        assert!(first <= 160, "found within a minute of being reachable: {first}");
        assert_eq!(max_gap(&reads, first), EVERY.as_secs());
        assert!(reads.last().unwrap().0 >= 1_200 - EVERY.as_secs(), "still reading at the end");
    }

    #[test]
    fn outlook_closed_and_reopened_resumes_the_same_loop() {
        let fake = Fake { running: |t| !(300..500).contains(&t), calendars: |t| (!(300..520).contains(&t)).then_some(9), events: vec![(0, 3_000), (450, 1_200)] };
        let reads = simulate(&fake, &[310, 340], 1_500);
        assert!(reads.iter().all(|r| !(300..500).contains(&r.0)), "nothing read while closed");
        let back = reads.iter().find(|r| r.0 >= 500).expect("back").0;
        assert!(back <= 600, "back within ~1.5 min of reopening: {back}");
        assert!(reads.iter().find(|r| r.0 >= 500).unwrap().1.contains(&1_200), "and the event made while it was closed is there");
        assert_eq!(max_gap(&reads, back), EVERY.as_secs(), "then every minute again");
    }

    #[test]
    fn a_read_is_logged_only_when_its_counts_change() {
        let now = 1_792_249_200_000;
        let e = |start: i64, join: bool| Event { title: "x".into(), location: String::new(), start_ms: start, end_ms: start + 1_800_000, all_day: false, calendar: String::new(), join: join.then(|| "https://zoom.us/j/1".to_string()) };
        let a = vec![e(now + 42 * 60_000, true)];
        let b = vec![e(now + 41 * 60_000, true)];
        assert_eq!(counts(&a), counts(&b), "a minute later, same counts: not logged again");
        assert_ne!(log_line(&a, now), log_line(&b, now), "though the minutes in the line differ");
        assert_eq!(counts(&[a[0].clone(), e(now, false)]), (2, 1));
        let src = include_str!("worker.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(code.contains("let key = format!(\"read {n}/{joins}\");"), "keyed by counts");
    }
}
