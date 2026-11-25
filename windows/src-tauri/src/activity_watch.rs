// Settings → Privacy → Activity watch (owner 2026-11-24): an opt-in,
// local-only log to find what opens windows on its own. While it is on (for
// DAYS days from the click, then off by itself) it writes to
// %LOCALAPPDATA%\Glim\activity-watch.log:
//   (a) each new cmd / powershell / pwsh / Windows Terminal / conhost /
//       OpenConsole / Word / Outlook / wscript process: time, name, PID, its parent
//       (name + PID) and its command line, credentials masked
//       (remote::exec::redact_detail: known token prefixes, labelled
//       passwords/tokens/keys, URL passwords);
//   (b) each change of the window in front: time, process name, PID and the
//       window's title (masked the same way).
// Nothing typed, no screen, no window contents beyond the title; nothing
// leaves the PC (no network, no AI, no phone). glim.log only says when it
// started and ended.
//
// How: the process list (Toolhelp) and GetForegroundWindow every TICK_MS.
// A process that starts and ends between two looks is missed (best effort);
// the ones that open a window live long enough to be seen.
//
// The on/off state is Rust's own file (activity-watch.json: {"until": unix
// seconds, 0 = off}), not settings.json: the running Glim reads it every few
// seconds, so Settings (or the owner, by hand) can switch it without a
// restart and without racing the page's settings saves.

// The thread half only runs on Windows; the logic is tested everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session_window::Proc;

/// How long one "Turn on" lasts.
pub const DAYS: u64 = 7;
/// The processes whose start is logged (any case).
pub const WATCHED: [&str; 9] = [
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "WindowsTerminal.exe",
    "conhost.exe",
    "OpenConsole.exe",
    "WINWORD.EXE",
    "OUTLOOK.EXE",
    "wscript.exe",
];
/// How often the process list and the window in front are looked at.
const TICK_MS: u64 = 250;
/// How often the on/off file is read again.
const STATE_EVERY_TICKS: u32 = 8;
/// Past this the log moves to activity-watch.log.1 (the one before goes).
const MAX_LOG_BYTES: u64 = 5_000_000;
/// The longest command line or title kept (characters).
const MAX_CHARS: usize = 2000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    /// Unix seconds it stays on until; 0 = off.
    until: u64,
}

fn state_path() -> PathBuf {
    crate::settings::local_dir().join("activity-watch.json")
}

pub fn log_path() -> PathBuf {
    crate::settings::local_dir().join("activity-watch.log")
}

fn read_until(path: &Path) -> u64 {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<State>(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&b[..])).ok()).map(|s| s.until).unwrap_or(0)
}

fn write_until(path: &Path, until: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        crate::platform::ensure_private_dir(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&State { until }).unwrap_or_default())?;
    std::fs::rename(&tmp, path)
}

/// Whether it is on at `now`, given the stored `until`.
pub fn active(until: u64, now: u64) -> bool {
    until != 0 && now < until
}

/// Until when it is on (Unix seconds), or 0 when it is off or has run out.
pub fn status() -> u64 {
    let until = read_until(&state_path());
    if active(until, crate::learn::now()) {
        until
    } else {
        0
    }
}

/// Turns it on for DAYS days from now, or off. Answers the new `until`.
pub fn set(on: bool) -> Result<u64, String> {
    let until = if on { crate::learn::now() + DAYS * 86_400 } else { 0 };
    write_until(&state_path(), until).map_err(|e| format!("couldn't save: {e}"))?;
    crate::log::line(if on { format!("activity watch: turned on for {DAYS} days") } else { "activity watch: turned off".to_string() });
    Ok(until)
}

/// A command line or title as it goes in the log: one line, credentials
/// masked, at most MAX_CHARS characters.
pub fn mask(s: &str) -> String {
    let one_line = s.replace(['\r', '\n', '\t'], " ");
    let masked = crate::remote::exec::redact_detail(&one_line);
    if masked.chars().count() > MAX_CHARS {
        let cut: String = masked.chars().take(MAX_CHARS).collect();
        format!("{cut}…")
    } else {
        masked
    }
}

fn watched(exe: &str) -> bool {
    WATCHED.iter().any(|w| w.eq_ignore_ascii_case(exe))
}

/// What the window in front is: (handle, process ID, title).
pub type Front = (isize, u32, String);

/// The watch's memory between looks: the processes seen last time and the
/// window that was in front.
#[derive(Default)]
pub struct Watch {
    seen: Option<HashSet<u32>>,
    front: Option<isize>,
}

impl Watch {
    /// Forgets everything (it was switched off): the next look is a fresh start.
    pub fn reset(&mut self) {
        self.seen = None;
        self.front = None;
    }

    /// One look: a line for each watched process that wasn't there last
    /// time (none on the first look, which only learns what already runs)
    /// and one when the window in front changed. `command_line` is asked only
    /// for the new watched processes.
    pub fn look(&mut self, procs: &HashMap<u32, Proc>, command_line: impl Fn(u32) -> Option<String>, front: Option<Front>, exe_of: impl Fn(u32) -> Option<String>) -> Vec<String> {
        let mut lines = Vec::new();
        let now: HashSet<u32> = procs.keys().copied().collect();
        if let Some(before) = &self.seen {
            let mut new: Vec<u32> = now.difference(before).copied().filter(|pid| procs.get(pid).is_some_and(|p| watched(&p.exe))).collect();
            new.sort_unstable();
            for pid in new {
                let p = &procs[&pid];
                let parent = procs.get(&p.parent).map(|q| q.exe.clone()).unwrap_or_else(|| "(gone)".into());
                let cmd = command_line(pid).map(|c| mask(&c)).unwrap_or_else(|| "(not readable)".into());
                lines.push(format!("started {} pid {} | parent {} pid {} | {}", p.exe, pid, parent, p.parent, cmd));
            }
        } else {
            let running = procs.values().filter(|p| watched(&p.exe)).count();
            lines.push(format!("watching: {running} watched process(es) already running"));
        }
        self.seen = Some(now);
        if let Some((hwnd, pid, title)) = front {
            if self.front != Some(hwnd) {
                self.front = Some(hwnd);
                let exe = procs.get(&pid).map(|p| p.exe.clone()).or_else(|| exe_of(pid)).unwrap_or_else(|| "(unknown)".into());
                lines.push(format!("front {} pid {} | \"{}\"", exe, pid, mask(&title)));
            }
        }
        lines
    }
}

/// Appends `lines` to the log, each with the local time.
fn append(path: &Path, lines: &[String]) {
    use std::io::Write;
    if lines.is_empty() {
        return;
    }
    if let Some(dir) = path.parent() {
        if crate::platform::ensure_private_dir(dir).is_err() {
            return;
        }
    }
    crate::log::rotate(path, MAX_LOG_BYTES);
    let t = crate::platform::local_time();
    let stamp = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        for l in lines {
            let _ = writeln!(f, "{stamp} {l}");
        }
    }
}

/// The watcher thread (Windows; started once at launch). Idle while off: it
/// only reads the small state file every couple of seconds.
#[cfg(windows)]
pub fn start() {
    std::thread::Builder::new()
        .name("activity-watch".into())
        .spawn(|| {
            let mut watch = Watch::default();
            let mut until = 0u64;
            let mut was_on = false;
            let mut tick: u32 = 0;
            loop {
                if tick == 0 {
                    until = read_until(&state_path());
                }
                tick = (tick + 1) % STATE_EVERY_TICKS;
                let now = crate::learn::now();
                let on = active(until, now);
                if on != was_on {
                    if on {
                        append(&log_path(), &["--- activity watch on (local only; nothing typed is logged) ---".to_string()]);
                    } else {
                        append(&log_path(), &["--- activity watch off ---".to_string()]);
                        watch.reset();
                        // Ran out by itself: the file says off again.
                        if until != 0 && now >= until {
                            let _ = write_until(&state_path(), 0);
                            crate::log::line(format!("activity watch: turned off by itself after {DAYS} days"));
                            until = 0;
                        }
                    }
                    was_on = on;
                }
                if on {
                    let procs = crate::platform::processes();
                    let lines = watch.look(&procs, crate::platform::command_line, crate::platform::foreground_window(), |pid| crate::platform::processes().remove(&pid).map(|p| p.exe));
                    append(&log_path(), &lines);
                }
                std::thread::sleep(std::time::Duration::from_millis(if on { TICK_MS } else { TICK_MS * 2 }));
            }
        })
        .ok();
}

#[cfg(not(windows))]
pub fn start() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[(u32, u32, &str)]) -> HashMap<u32, Proc> {
        rows.iter().map(|(pid, parent, exe)| (*pid, Proc { parent: *parent, exe: exe.to_string() })).collect()
    }

    #[test]
    fn the_first_look_only_learns_what_runs_then_new_watched_starts_are_logged() {
        let mut w = Watch::default();
        let first = table(&[(1, 0, "explorer.exe"), (2, 1, "cmd.exe")]);
        let l = w.look(&first, |_| panic!("never asked on the first look"), None, |_| None);
        assert_eq!(l, vec!["watching: 1 watched process(es) already running".to_string()]);
        let second = table(&[(1, 0, "explorer.exe"), (2, 1, "cmd.exe"), (3, 1, "WINWORD.EXE"), (4, 1, "notepad.exe"), (5, 99, "WindowsTerminal.exe")]);
        let l = w.look(&second, |pid| Some(format!("cmdline-{pid}")), None, |_| None);
        assert_eq!(
            l,
            vec![
                "started WINWORD.EXE pid 3 | parent explorer.exe pid 1 | cmdline-3".to_string(),
                "started WindowsTerminal.exe pid 5 | parent (gone) pid 99 | cmdline-5".to_string(),
            ],
            "only watched names; an unwatched one (notepad) and one already seen (cmd) are not logged"
        );
        assert!(w.look(&second, |_| panic!("nothing new"), None, |_| None).is_empty());
    }

    #[test]
    fn the_window_in_front_is_logged_when_it_changes_not_every_look() {
        let mut w = Watch::default();
        let procs = table(&[(7, 1, "OUTLOOK.EXE")]);
        let l = w.look(&procs, |_| None, Some((70, 7, "Inbox - Outlook".into())), |_| None);
        assert_eq!(l.last().unwrap(), "front OUTLOOK.EXE pid 7 | \"Inbox - Outlook\"");
        assert!(w.look(&procs, |_| None, Some((70, 7, "Inbox (2) - Outlook".into())), |_| None).is_empty(), "same window: nothing");
        let l = w.look(&procs, |_| None, Some((80, 8, "x".into())), |pid| (pid == 8).then(|| "Code.exe".to_string()));
        assert_eq!(l, vec!["front Code.exe pid 8 | \"x\"".to_string()]);
        w.reset();
        assert!(w.look(&procs, |_| None, Some((80, 8, "x".into())), |_| None).iter().any(|l| l.starts_with("front")), "after a reset the front is logged again");
    }

    #[test]
    fn credentials_in_command_lines_and_titles_are_masked_and_lines_stay_one_line() {
        let token = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let m = mask(&format!("claude.exe --token hunter2hunter2 --key {token}\r\nsecond line"));
        assert!(!m.contains(token) && !m.contains("hunter2hunter2"), "{m}");
        assert!(!m.contains('\n') && !m.contains('\r'));
        let m = mask("git push https://user:pa55word@example.com/repo.git");
        assert!(!m.contains("pa55word"), "{m}");
        let long = "a".repeat(MAX_CHARS + 50);
        assert_eq!(mask(&long).chars().count(), MAX_CHARS + 1, "cut with an ellipsis");
    }

    #[test]
    fn on_only_until_its_time_and_the_state_file_round_trips() {
        assert!(!active(0, 100));
        assert!(active(200, 100));
        assert!(!active(200, 200), "off at its time");
        let dir = std::env::temp_dir().join(format!("glim-activity-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("activity-watch.json");
        assert_eq!(read_until(&p), 0, "no file = off");
        write_until(&p, 12345).unwrap();
        assert_eq!(read_until(&p), 12345);
        std::fs::write(&p, b"\xEF\xBB\xBF{\"until\":5}").unwrap();
        assert_eq!(read_until(&p), 5, "a BOM (PowerShell, Notepad) still reads");
        std::fs::write(&p, b"garbage").unwrap();
        assert_eq!(read_until(&p), 0, "unreadable = off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_never_reaches_the_network_or_the_ai() {
        let src = include_str!("activity_watch.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        // The network words are built from pieces: net::tests scans every
        // source file for them literally, this one included.
        for banned in ["net::", concat!("req", "west"), "phone::", "ai::", "claude::", concat!("Tcp", "Stream"), "keybd", "SendInput", "GetAsyncKeyState", "SetWindowsHookEx"] {
            assert!(!code.contains(banned), "activity_watch must not use {banned}");
        }
    }
}
