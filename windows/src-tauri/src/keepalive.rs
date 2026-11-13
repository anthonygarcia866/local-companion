// The keep-alive switch (local agency v1, part 3; owner 2026-11-12).
//
// Settings → Remote → "Keep Claude Code reachable" (off by default; a
// one-time consent card first). While it is on, Glim starts each entry on
// its list when it starts (at login, with launch at startup) and when the
// switch or an entry is turned on, and starts it again with backoff when it
// exits: 30 s, 2 min, 10 min, then every 30 min; a run of 30 min or more
// resets that. Three restarts that ended within an hour ping the phone once
// (phone.rs, Kind::KeepAlive; Settings → Phone must be on), at most hourly.
//
// The list is Rust's (lib.rs keeps it out of the page's settings saves):
// - a project folder (a full local path, an existing folder, not a drive,
//   Windows, Program Files or the whole profile), whose command is FIXED:
//   `claude remote-control --spawn same-dir --capacity 1 --name "<folder name> (PC)"`,
//   run through `cmd /d /s /c` in that folder in a console of its own (Windows
//   Terminal on Windows 11, as the default terminal), with
//   CLAUDE_CODE_DISABLE_BG_SHELL_PRESSURE_REAP=1 in its environment;
// - optionally the Claude desktop app (started through Windows, as voice's
//   open_app does; "running" = its packaged claude.exe is running).
// Nothing else is ever started: there is no free-form command anywhere.
//
// Glim itself makes no network call here; Claude Code's remote control,
// once started, connects to Anthropic (the consent card says so).
//
// Tests use a fake launcher and clock (backoff, alert, the list), and one
// test runs a fake command for real.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const ENV_NAME: &str = "CLAUDE_CODE_DISABLE_BG_SHELL_PRESSURE_REAP";
pub const ENV_VALUE: &str = "1";
/// Seconds before restart #1, #2, #3; then every LATER_S.
pub const BACKOFF_S: [u64; 3] = [30, 120, 600];
pub const LATER_S: u64 = 1800;
/// A run this long resets the backoff.
pub const STABLE_S: u64 = 1800;
pub const ALERT_RESTARTS: usize = 3;
pub const ALERT_WINDOW_S: u64 = 3600;
pub const MAX_FOLDERS: usize = 8;
/// The desktop app's id in the list (and in status).
pub const DESKTOP: &str = "desktop";

/// Settings → Remote, as settings.json keeps it. Rust-owned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeepAliveSettings {
    /// "Keep Claude Code reachable".
    pub on: bool,
    /// The one-time consent card was answered "Turn on".
    pub consented: bool,
    /// Project folders, each with the fixed command.
    pub folders: Vec<String>,
    /// Also keep the Claude desktop app open.
    pub desktop_app: bool,
}

/// A folder's name as it goes in `--name "<…> (PC)"`: letters, digits,
/// spaces, `-_.` only, at most 40; "project" when nothing is left.
pub fn display_name(folder: &str) -> String {
    let last = folder.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')).take(40).collect();
    let clean = clean.trim().to_string();
    if clean.is_empty() { "project".into() } else { clean }
}

/// The fixed command for a folder.
pub fn command_line(folder: &str) -> String {
    format!("claude remote-control --spawn same-dir --capacity 1 --name \"{} (PC)\"", display_name(folder))
}

/// Why a typed folder can't go on the list, or None.
pub fn folder_problem(raw: &str, list: &[String]) -> Option<String> {
    let raw = raw.trim();
    if list.len() >= MAX_FOLDERS {
        return Some(format!("at most {MAX_FOLDERS} folders"));
    }
    if let Err(e) = crate::runbooks::allowed_folder(raw) {
        return Some(e);
    }
    if list.iter().any(|f| f.eq_ignore_ascii_case(raw)) {
        return Some("already on the list".into());
    }
    None
}

/// One thing to keep running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Folder(String),
    Desktop,
}

impl Spec {
    pub fn id(&self) -> String {
        match self {
            Spec::Folder(f) => f.to_ascii_lowercase(),
            Spec::Desktop => DESKTOP.into(),
        }
    }
}

/// What should be running now, from Settings: nothing unless on and consented.
pub fn wanted(s: &KeepAliveSettings) -> Vec<Spec> {
    if !(s.on && s.consented) {
        return Vec::new();
    }
    let mut out: Vec<Spec> = s.folders.iter().take(MAX_FOLDERS).cloned().map(Spec::Folder).collect();
    if s.desktop_app {
        out.push(Spec::Desktop);
    }
    out
}

/// A started entry.
pub trait Running: Send {
    /// Some(exit code; -1 unknown) once it has ended.
    fn exited(&mut self) -> Option<i32>;
    fn stop(&mut self);
}

/// Starts entries (real in the app, fake in tests).
pub trait Launcher {
    fn start(&self, spec: &Spec) -> Result<Box<dyn Running>, String>;
}

/// The backoff and alert counting for one entry.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    /// Restarts in a row without a stable run.
    pub consecutive: usize,
    /// When restarted runs ended, within the last hour.
    pub failed: VecDeque<u64>,
    pub alerted_at: Option<u64>,
}

impl Backoff {
    /// A run ended at `now` after `ran_s` seconds (`restart`: it was itself
    /// a restart). The wait before the next start, and whether to ping.
    pub fn exited(&mut self, now: u64, ran_s: u64, restart: bool) -> (u64, bool) {
        if ran_s >= STABLE_S {
            self.consecutive = 0;
            self.failed.clear();
        }
        let delay = BACKOFF_S.get(self.consecutive).copied().unwrap_or(LATER_S);
        self.consecutive += 1;
        if restart && ran_s < STABLE_S {
            self.failed.push_back(now);
        }
        while self.failed.front().is_some_and(|t| now.saturating_sub(*t) >= ALERT_WINDOW_S) {
            self.failed.pop_front();
        }
        let alert = self.failed.len() >= ALERT_RESTARTS && self.alerted_at.is_none_or(|t| now.saturating_sub(t) >= ALERT_WINDOW_S);
        if alert {
            self.alerted_at = Some(now);
        }
        (delay, alert)
    }
}

enum Phase {
    Running { child: Box<dyn Running>, since: u64, restart: bool },
    Waiting { until: u64, last_exit: i32 },
    /// Couldn't start: tried again like an exit.
    StartFailed { until: u64, why: String },
}

struct Slot {
    spec: Spec,
    phase: Phase,
    backoff: Backoff,
}

/// What Settings shows for an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: String,
    /// The folder, or "" for the desktop app.
    pub folder: String,
    /// The fixed command, or "" for the desktop app.
    pub command: String,
    /// "running" | "waiting" | "start-failed" | "off".
    pub state: &'static str,
    /// Seconds since 1970: running since / next try.
    pub at: u64,
    pub last_exit: Option<i32>,
    pub why: String,
    /// Restarts that ended within the last hour.
    pub failed_last_hour: usize,
}

/// Something the supervisor wants said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Started(String),
    Exited(String, i32),
    /// Three restarts ended within an hour: ping the phone (the folder's
    /// name, or "" for the desktop app).
    Alert(String),
}

#[derive(Default)]
pub struct Supervisor {
    slots: BTreeMap<String, Slot>,
}

impl Supervisor {
    /// One look at everything: starts what's wanted and not running, notes
    /// exits and their backoff, stops what's no longer wanted.
    pub fn tick(&mut self, wanted: &[Spec], now: u64, launcher: &dyn Launcher) -> Vec<Event> {
        let mut events = Vec::new();
        let ids: Vec<String> = wanted.iter().map(Spec::id).collect();
        // No longer on the list (or the switch went off): stopped, forgotten.
        let gone: Vec<String> = self.slots.keys().filter(|k| !ids.contains(k)).cloned().collect();
        for k in gone {
            if let Some(mut s) = self.slots.remove(&k) {
                if let Phase::Running { child, .. } = &mut s.phase {
                    child.stop();
                }
            }
        }
        for spec in wanted {
            let id = spec.id();
            let name = match spec {
                Spec::Folder(f) => display_name(f),
                Spec::Desktop => String::new(),
            };
            let slot = self.slots.entry(id.clone());
            let slot = match slot {
                std::collections::btree_map::Entry::Vacant(v) => {
                    let phase = launch(spec, now, false, launcher, &mut events, &id);
                    v.insert(Slot { spec: spec.clone(), phase, backoff: Backoff::default() });
                    continue;
                }
                std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
            };
            let next = match &mut slot.phase {
                Phase::Running { child, since, restart } => child.exited().map(|code| {
                    events.push(Event::Exited(id.clone(), code));
                    let (delay, alert) = slot.backoff.exited(now, now.saturating_sub(*since), *restart);
                    if alert {
                        events.push(Event::Alert(name.clone()));
                    }
                    Phase::Waiting { until: now + delay, last_exit: code }
                }),
                Phase::Waiting { until, .. } | Phase::StartFailed { until, .. } if now >= *until => Some(launch(spec, now, true, launcher, &mut events, &id)),
                _ => None,
            };
            match next {
                // A restart that couldn't even start counts as a failed one.
                Some(Phase::StartFailed { why, .. }) => {
                    let (delay, alert) = slot.backoff.exited(now, 0, true);
                    if alert {
                        events.push(Event::Alert(name.clone()));
                    }
                    slot.phase = Phase::StartFailed { until: now + delay, why };
                }
                Some(p) => slot.phase = p,
                None => {}
            }
        }
        events
    }

    pub fn views(&self, s: &KeepAliveSettings) -> Vec<EntryView> {
        let mut specs: Vec<Spec> = s.folders.iter().cloned().map(Spec::Folder).collect();
        if s.desktop_app {
            specs.push(Spec::Desktop);
        }
        specs
            .iter()
            .map(|spec| {
                let id = spec.id();
                let (folder, command) = match spec {
                    Spec::Folder(f) => (f.clone(), command_line(f)),
                    Spec::Desktop => (String::new(), String::new()),
                };
                let mut v = EntryView { id: id.clone(), folder, command, state: "off", at: 0, last_exit: None, why: String::new(), failed_last_hour: 0 };
                if let Some(slot) = self.slots.get(&id) {
                    v.failed_last_hour = slot.backoff.failed.len();
                    match &slot.phase {
                        Phase::Running { since, .. } => (v.state, v.at) = ("running", *since),
                        Phase::Waiting { until, last_exit } => (v.state, v.at, v.last_exit) = ("waiting", *until, Some(*last_exit)),
                        Phase::StartFailed { until, why } => (v.state, v.at, v.why) = ("start-failed", *until, why.clone()),
                    }
                }
                v
            })
            .collect()
    }

    /// Stops everything (Glim quitting, or the switch going off at once).
    pub fn stop_all(&mut self) {
        for (_, mut s) in std::mem::take(&mut self.slots) {
            if let Phase::Running { child, .. } = &mut s.phase {
                child.stop();
            }
        }
    }
}

fn launch(spec: &Spec, now: u64, restart: bool, launcher: &dyn Launcher, events: &mut Vec<Event>, id: &str) -> Phase {
    match launcher.start(spec) {
        Ok(child) => {
            events.push(Event::Started(id.to_string()));
            Phase::Running { child, since: now, restart }
        }
        Err(why) => Phase::StartFailed { until: now + BACKOFF_S[0], why },
    }
}

// ── The real launcher ─────────────────────────────────────────────────────────

/// A folder entry's process, or the desktop app (watched by its process).
struct Child(std::process::Child);

impl Running for Child {
    fn exited(&mut self) -> Option<i32> {
        match self.0.try_wait() {
            Ok(Some(s)) => Some(s.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }
    fn stop(&mut self) {
        crate::platform::kill_tree(self.0.id());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Desktop {
    checked: u64,
    gone: bool,
}

impl Running for Desktop {
    fn exited(&mut self) -> Option<i32> {
        // The process list every 15 s, not every tick.
        let now = crate::learn::now();
        if now.saturating_sub(self.checked) >= 15 {
            self.checked = now;
            self.gone = !crate::platform::claude_desktop_running();
        }
        self.gone.then_some(0)
    }
    fn stop(&mut self) {
        // The desktop app is the user's: switching keep-alive off never closes it.
    }
}

/// Starts the real thing: the fixed command in its folder, in a console of
/// its own, or the desktop app.
pub struct RealLauncher {
    /// Tests only: a command line instead of the fixed one.
    #[cfg(test)]
    pub fake_command: Option<String>,
    /// Tests only: no console window.
    #[cfg(test)]
    pub hidden: bool,
}

impl RealLauncher {
    pub fn new() -> Self {
        RealLauncher {
            #[cfg(test)]
            fake_command: None,
            #[cfg(test)]
            hidden: false,
        }
    }

    fn command_for(&self, folder: &str) -> String {
        #[cfg(test)]
        if let Some(c) = &self.fake_command {
            return c.clone();
        }
        command_line(folder)
    }

    fn new_console(&self) -> bool {
        #[cfg(test)]
        if self.hidden {
            return false;
        }
        true
    }
}

impl Launcher for RealLauncher {
    fn start(&self, spec: &Spec) -> Result<Box<dyn Running>, String> {
        match spec {
            Spec::Desktop => {
                if crate::platform::claude_desktop_running() || crate::platform::open_known_app("claude") {
                    Ok(Box::new(Desktop { checked: crate::learn::now() + 30, gone: false }))
                } else {
                    Err("Windows couldn't start the Claude app".into())
                }
            }
            Spec::Folder(folder) => {
                // Checked again: the folder may have gone since it was added.
                if let Err(e) = crate::runbooks::allowed_folder(folder) {
                    return Err(e);
                }
                let child = crate::platform::start_in_console(&self.command_for(folder), std::path::Path::new(folder), &[(ENV_NAME, ENV_VALUE)], self.new_console())?;
                Ok(Box::new(Child(child)))
            }
        }
    }
}

// ── In the app ────────────────────────────────────────────────────────────────

static SUPERVISOR: std::sync::Mutex<Option<Supervisor>> = std::sync::Mutex::new(None);

/// Settings → Remote's status lines.
pub fn views(s: &KeepAliveSettings) -> Vec<EntryView> {
    SUPERVISOR.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|sup| sup.views(s)).unwrap_or_else(|| Supervisor::default().views(s))
}

/// Starts the supervisor thread (once, at Glim's start): every 2 s it looks
/// at Settings and its entries. `settings` reads the current settings.
pub fn start(settings: impl Fn() -> crate::settings::Settings + Send + 'static, alert: impl Fn(crate::settings::Settings, String) + Send + 'static) {
    {
        let mut sup = SUPERVISOR.lock().unwrap_or_else(|e| e.into_inner());
        if sup.is_some() {
            return;
        }
        *sup = Some(Supervisor::default());
    }
    std::thread::spawn(move || {
        let launcher = RealLauncher::new();
        loop {
            let s = settings();
            let want = wanted(&s.keep_alive);
            let events = {
                let mut sup = SUPERVISOR.lock().unwrap_or_else(|e| e.into_inner());
                sup.as_mut().map(|sup| sup.tick(&want, crate::learn::now(), &launcher)).unwrap_or_default()
            };
            for e in events {
                match e {
                    Event::Started(_) => crate::log::line("keep-alive: started one"),
                    Event::Exited(_, code) => crate::log::line(format!("keep-alive: one exited (code {code}); restarting with backoff")),
                    Event::Alert(name) => {
                        crate::log::line("keep-alive: 3 restarts ended within an hour; phone ping");
                        alert(s.clone(), name);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

/// Glim is quitting: the consoles it started go with it.
pub fn stop_all() {
    if let Some(sup) = SUPERVISOR.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        sup.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fake {
        started: RefCell<Vec<String>>,
        /// Each started child's "exit now" switch, by start order.
        kills: RefCell<Vec<Arc<Mutex<Option<i32>>>>>,
        stopped: Arc<Mutex<usize>>,
        refuse: RefCell<bool>,
    }

    struct FakeChild(Arc<Mutex<Option<i32>>>, Arc<Mutex<usize>>);

    impl Running for FakeChild {
        fn exited(&mut self) -> Option<i32> {
            *self.0.lock().unwrap()
        }
        fn stop(&mut self) {
            *self.1.lock().unwrap() += 1;
        }
    }

    impl Launcher for Fake {
        fn start(&self, spec: &Spec) -> Result<Box<dyn Running>, String> {
            if *self.refuse.borrow() {
                return Err("refused".into());
            }
            self.started.borrow_mut().push(match spec {
                Spec::Folder(f) => command_line(f),
                Spec::Desktop => "desktop".into(),
            });
            let k = Arc::new(Mutex::new(None));
            self.kills.borrow_mut().push(k.clone());
            Ok(Box::new(FakeChild(k, self.stopped.clone())))
        }
    }

    fn exit_last(f: &Fake, code: i32) {
        *f.kills.borrow().last().unwrap().lock().unwrap() = Some(code);
    }

    const FOLDER: &str = "C:\\path\\to\\example-project";

    #[test]
    fn the_command_is_fixed_with_the_folder_name() {
        assert_eq!(command_line(FOLDER), "claude remote-control --spawn same-dir --capacity 1 --name \"example-project (PC)\"");
        assert_eq!(display_name("C:\\x\\we\"ird&name|"), "weirdname");
        assert_eq!(display_name("C:\\"), "project");
        assert_eq!(display_name("C:\\a\\b\\"), "b");
    }

    #[test]
    fn nothing_runs_unless_on_and_consented() {
        let mut s = KeepAliveSettings { on: true, consented: false, folders: vec![FOLDER.into()], desktop_app: true };
        assert!(wanted(&s).is_empty());
        s.consented = true;
        assert_eq!(wanted(&s), [Spec::Folder(FOLDER.into()), Spec::Desktop]);
        s.on = false;
        assert!(wanted(&s).is_empty());
        assert_eq!(KeepAliveSettings::default(), KeepAliveSettings { on: false, consented: false, folders: vec![], desktop_app: false }, "off by default");
    }

    #[test]
    fn restarts_back_off_30s_2min_10min_then_30min() {
        let f = Fake::default();
        let mut sup = Supervisor::default();
        let want = [Spec::Folder(FOLDER.into())];
        let mut now = 1_000_000;
        sup.tick(&want, now, &f);
        assert_eq!(f.started.borrow().len(), 1);
        let mut waits = Vec::new();
        for _ in 0..5 {
            now += 5;
            exit_last(&f, 1);
            sup.tick(&want, now, &f);
            let at = sup.views(&KeepAliveSettings { folders: vec![FOLDER.into()], ..Default::default() })[0].at;
            waits.push(at - now);
            // Nothing starts before the wait is over.
            sup.tick(&want, at - 1, &f);
            let n = f.started.borrow().len();
            now = at;
            sup.tick(&want, now, &f);
            assert_eq!(f.started.borrow().len(), n + 1);
        }
        assert_eq!(waits, [30, 120, 600, 1800, 1800]);
        // Every start was the fixed command for the listed folder.
        assert!(f.started.borrow().iter().all(|c| *c == command_line(FOLDER)));
    }

    #[test]
    fn a_long_run_resets_the_backoff() {
        let mut b = Backoff::default();
        assert_eq!(b.exited(100, 5, false).0, 30);
        assert_eq!(b.exited(200, 5, true).0, 120);
        assert_eq!(b.exited(5000, STABLE_S, true).0, 30, "ran 30 min: back to 30 s");
    }

    #[test]
    fn three_failed_restarts_in_an_hour_ping_once() {
        let mut b = Backoff::default();
        let t = 10_000;
        assert!(!b.exited(t, 5, false).1, "the first run's end is no failed restart");
        assert!(!b.exited(t + 60, 5, true).1);
        assert!(!b.exited(t + 300, 5, true).1);
        assert!(b.exited(t + 1000, 5, true).1, "the third failed restart within the hour");
        assert!(!b.exited(t + 3000, 5, true).1, "once an hour at most");
        // Spread over more than an hour: never.
        let mut b = Backoff::default();
        for i in 0..6 {
            assert!(!b.exited(t + i * 2000, 5, true).1, "exit {i}");
        }
    }

    #[test]
    fn the_supervisor_alerts_with_the_folder_name_only() {
        let f = Fake::default();
        let mut sup = Supervisor::default();
        let want = [Spec::Folder(FOLDER.into())];
        let mut now = 50_000;
        sup.tick(&want, now, &f);
        let mut alerts = Vec::new();
        for _ in 0..4 {
            now += 10;
            exit_last(&f, 3);
            alerts.extend(sup.tick(&want, now, &f).into_iter().filter(|e| matches!(e, Event::Alert(_))));
            now = sup.views(&KeepAliveSettings { folders: vec![FOLDER.into()], ..Default::default() })[0].at;
            sup.tick(&want, now, &f);
        }
        assert_eq!(alerts, [Event::Alert("example-project".into())]);
    }

    #[test]
    fn a_removed_entry_or_switching_off_stops_it_and_starts_nothing_else() {
        let f = Fake::default();
        let mut sup = Supervisor::default();
        sup.tick(&[Spec::Folder(FOLDER.into()), Spec::Desktop], 1, &f);
        assert_eq!(f.started.borrow().len(), 2);
        sup.tick(&[Spec::Desktop], 2, &f);
        assert_eq!(*f.stopped.lock().unwrap(), 1, "the removed folder's process stopped");
        sup.tick(&[], 3, &f);
        assert_eq!(*f.stopped.lock().unwrap(), 2);
        for t in 4..100 {
            sup.tick(&[], t, &f);
        }
        assert_eq!(f.started.borrow().len(), 2, "nothing started that isn't on the list");
    }

    #[test]
    fn a_start_that_fails_is_retried_with_backoff() {
        let f = Fake::default();
        *f.refuse.borrow_mut() = true;
        let mut sup = Supervisor::default();
        let want = [Spec::Folder(FOLDER.into())];
        sup.tick(&want, 100, &f);
        let v = sup.views(&KeepAliveSettings { folders: vec![FOLDER.into()], ..Default::default() });
        assert_eq!((v[0].state, v[0].at), ("start-failed", 130));
        *f.refuse.borrow_mut() = false;
        sup.tick(&want, 129, &f);
        assert!(f.started.borrow().is_empty());
        sup.tick(&want, 130, &f);
        assert_eq!(f.started.borrow().len(), 1);
    }

    /// The real launcher with a fake command: the process starts in the
    /// folder with the keep-alive variable set, and its exit is seen.
    #[cfg(windows)]
    #[test]
    fn a_fake_command_runs_for_real_with_the_variable_set() {
        let dir = std::env::temp_dir().join(format!("glim-keepalive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = dir.to_string_lossy().into_owned();
        let launcher = RealLauncher { fake_command: Some(format!("if \"%{ENV_NAME}%\"==\"1\" (exit 7) else (exit 9)")), hidden: true };
        let mut child = launcher.start(&Spec::Folder(folder.clone())).unwrap();
        let mut code = None;
        for _ in 0..100 {
            code = child.exited();
            if code.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(code, Some(7), "ran with {ENV_NAME}=1");
        // And through the supervisor: started, exited, waiting 30 s.
        let mut sup = Supervisor::default();
        let want = [Spec::Folder(folder.clone())];
        sup.tick(&want, 1000, &launcher);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let events = sup.tick(&want, 1002, &launcher);
        assert_eq!(events, [Event::Exited(folder.to_ascii_lowercase(), 7)]);
        let v = sup.views(&KeepAliveSettings { folders: vec![folder], ..Default::default() });
        assert_eq!((v[0].state, v[0].last_exit, v[0].at), ("waiting", Some(7), 1032));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
