// The calendar glance's thread: a COM STA that reads Outlook's calendar
// every few minutes (sooner when Settings change), keeps what it read in
// memory, and sends the island what it shows ("calendar"). The meeting links
// stay here: the island asks to join an event by (generation, index), and
// only a checked Teams/Zoom link is opened (calendar::meeting_link).
//
// The log gets counts, codes and minutes; never a title, a location, a
// calendar's name or a link.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::{in_window, next, summary_due, today, Event, Glance, ShownEvent, WINDOW_MS};

pub const EVENT: &str = "calendar";
/// How often Outlook is read.
pub const EVERY: Duration = Duration::from_secs(5 * 60);

/// A calendar Settings can offer: Outlook's folder path and its name.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarFolder {
    pub id: String,
    pub name: String,
    pub default: bool,
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
    let mut job = Job::Refresh;
    loop {
        match job {
            Job::Refresh => {
                let p = prefs(&app);
                generation += 1;
                let g = if !p.on {
                    events.clear();
                    Glance { code: "off", generation, ..Default::default() }
                } else {
                    match super::com::Calendar::attach() {
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
                                    let line = log_line(&events, now);
                                    if line != last_log {
                                        crate::log::line(&line);
                                        last_log = line;
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
                let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, &g);
            }
            Job::List(reply) => {
                let found = match super::com::Calendar::attach() {
                    Ok(cal) => {
                        let default = cal.default_calendar();
                        cal.calendars().into_iter().map(|(id, name)| CalendarFolder { default: Some(&id) == default.as_ref(), id, name }).collect()
                    }
                    Err(_) => Vec::new(),
                };
                crate::log::line(format!("calendar: {} calendars found for Settings", found.len()));
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
        job = match rx.recv_timeout(EVERY) {
            Ok(j) => j,
            Err(RecvTimeoutError::Timeout) => Job::Refresh,
            Err(RecvTimeoutError::Disconnected) => return,
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
            location: "Room 12C".into(),
            start_ms: start,
            end_ms: start + 1_800_000,
            all_day: false,
            calendar: "\\\\me@example.com\\Calendar".into(),
            join: join.map(str::to_string),
        };
        let events = vec![e("Rent review with Gabriela", now + 42 * 60_000, Some("https://zoom.us/j/1")), e("Lunch", now + 3 * 3_600_000, None)];
        let line = log_line(&events, now);
        assert_eq!(line, "calendar: 2 events in the next 24 h (1 with a meeting link), next in 42 min");
        for secret in ["Gabriela", "Lunch", "12C", "zoom", "example.com"] {
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
}
