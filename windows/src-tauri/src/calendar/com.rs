// Classic Outlook's calendar, read only, through local COM (calendar glance).
//
// - Attaches only to an Outlook that is already running (GetActiveObject):
//   never starts it, so no window and no background Outlook appears for this.
// - Reads appointments in a time window (Items.Restrict with recurrences
//   expanded by Outlook): subject, location, start/end in UTC, all day, and
//   the body only to look for a Teams/Zoom link. Cancelled and declined
//   meetings are left out.
// - Never saves, sends, answers or opens an item: there is no Save, Send,
//   Respond, Display or Delete call here (a test checks).
//
// Late-bound (IDispatch) on the calling thread, a COM STA (calendar::worker).
// Nothing read here is logged.

use windows::core::{Interface, IUnknown};
use windows::Win32::System::Com::{CLSIDFromProgID, IDispatch};
use windows::Win32::System::Ole::GetActiveObject;
use windows::Win32::System::Variant::{VARIANT, VT_DATE};

use super::{join_link, ole_to_unix_ms, restrict_date, Event};
use crate::outlook::{boolean, Disp};

/// olFolderCalendar.
const CALENDAR: i32 = 9;
/// A folder's DefaultItemType for appointments (olAppointmentItem).
const APPOINTMENTS: i32 = 1;
/// An item's Class for an appointment (olAppointment).
const APPOINTMENT_CLASS: i32 = 26;
/// MeetingStatus: olMeetingCanceled, olMeetingReceivedAndCanceled.
const CANCELED: [i32; 2] = [5, 7];
/// ResponseStatus: olResponseDeclined.
const DECLINED: i32 = 4;
/// Items read per calendar, at most.
const MAX_ITEMS: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComError {
    /// Classic Outlook isn't running (or isn't installed).
    NotRunning,
    /// Outlook refused or failed.
    Failed,
}

impl ComError {
    pub fn code(self) -> &'static str {
        match self {
            ComError::NotRunning => "no-outlook",
            ComError::Failed => "failed",
        }
    }
}

pub struct Calendar {
    ns: Disp,
}

impl Calendar {
    /// The running Outlook's MAPI namespace; Err(NotRunning) if it isn't up.
    pub fn attach() -> Result<Calendar, ComError> {
        let clsid = unsafe { CLSIDFromProgID(windows::core::w!("Outlook.Application")) }.map_err(|_| ComError::NotRunning)?;
        let mut unknown: Option<IUnknown> = None;
        unsafe { GetActiveObject(&clsid, None, &mut unknown) }.map_err(|_| ComError::NotRunning)?;
        let app: IDispatch = unknown.ok_or(ComError::NotRunning)?.cast().map_err(|_| ComError::Failed)?;
        let ns = Disp::from(app).call_obj("GetNamespace", &[VARIANT::from("MAPI")]).map_err(|_| ComError::Failed)?;
        Ok(Calendar { ns })
    }

    /// The default calendar's folder path.
    pub fn default_calendar(&self) -> Option<String> {
        self.ns.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]).ok().map(|f| f.text("FolderPath")).filter(|p| !p.is_empty())
    }

    /// Every calendar folder Outlook has open: (folder path, name), the
    /// default one first. Each store's calendar and the calendars under it.
    pub fn calendars(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add = |f: &Disp| {
            let path = f.text("FolderPath");
            if !path.is_empty() && !out.iter().any(|(p, _)| *p == path) {
                out.push((path, f.text("Name")));
            }
        };
        if let Ok(f) = self.ns.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]) {
            add(&f);
        }
        let Ok(stores) = self.ns.obj("Stores") else { return out };
        for i in 1..=stores.int("Count") {
            let Ok(store) = stores.item(i) else { continue };
            let Ok(cal) = store.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]) else { continue };
            add(&cal);
            for sub in subfolders(&cal, 2) {
                add(&sub);
            }
        }
        out
    }

    /// The events overlapping [from, to) in the calendars `ids` (folder
    /// paths; empty = the default calendar). `offset_min`: the PC's UTC
    /// offset, for Restrict's local times.
    pub fn events(&self, ids: &[String], from_ms: i64, to_ms: i64, offset_min: i32) -> Result<Vec<Event>, ComError> {
        let mut folders: Vec<Disp> = Vec::new();
        if ids.is_empty() {
            folders.push(self.ns.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]).map_err(|_| ComError::Failed)?);
        } else {
            let wanted = |f: &Disp| ids.contains(&f.text("FolderPath"));
            if let Ok(f) = self.ns.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]) {
                if wanted(&f) {
                    folders.push(f);
                }
            }
            if let Ok(stores) = self.ns.obj("Stores") {
                for i in 1..=stores.int("Count") {
                    let Ok(store) = stores.item(i) else { continue };
                    let Ok(cal) = store.call_obj("GetDefaultFolder", &[crate::outlook::int(CALENDAR)]) else { continue };
                    let mut all = vec![cal.clone()];
                    all.extend(subfolders(&cal, 2));
                    for f in all {
                        let path = f.text("FolderPath");
                        if wanted(&f) && !folders.iter().any(|g| g.text("FolderPath") == path) {
                            folders.push(f);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for f in folders {
            out.extend(read_folder(&f, from_ms, to_ms, offset_min));
        }
        Ok(out)
    }
}

/// Calendar folders under `f`, `depth` levels down.
fn subfolders(f: &Disp, depth: u32) -> Vec<Disp> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    let Ok(children) = f.obj("Folders") else { return out };
    for j in 1..=children.int("Count") {
        let Ok(c) = children.item(j) else { continue };
        if c.int("DefaultItemType") == APPOINTMENTS {
            out.extend(subfolders(&c, depth - 1));
            out.push(c);
        }
    }
    out
}

fn date(d: &Disp, name: &str) -> Option<f64> {
    let v = d.get(name).ok()?;
    // Checked by its type first, like Disp::text.
    (v.vt() == VT_DATE).then(|| unsafe { v.Anonymous.Anonymous.Anonymous.date })
}

fn flag(d: &Disp, name: &str) -> bool {
    d.get(name).ok().and_then(|v| bool::try_from(&v).ok()).unwrap_or(false)
}

/// One calendar's appointments overlapping the window, recurrences expanded.
fn read_folder(f: &Disp, from_ms: i64, to_ms: i64, offset_min: i32) -> Vec<Event> {
    let calendar = f.text("FolderPath");
    let Ok(items) = f.obj("Items") else { return Vec::new() };
    // Outlook expands recurring meetings only for a list sorted by Start
    // with IncludeRecurrences on, then restricted to a window.
    let _ = items.call("Sort", &[VARIANT::from("[Start]")]);
    let _ = items.put("IncludeRecurrences", boolean(true));
    let filter = format!("[Start] < '{}' AND [End] > '{}'", restrict_date(to_ms, offset_min), restrict_date(from_ms, offset_min));
    let list = items.call_obj("Restrict", &[VARIANT::from(filter.as_str())]).unwrap_or(items);
    let mut out = Vec::new();
    let mut next = list.call_obj("GetFirst", &[]);
    let mut n = 0;
    while let Ok(item) = next {
        n += 1;
        if n > MAX_ITEMS {
            break;
        }
        if let Some(e) = event_of(&item, &calendar) {
            // Restrict refused (another date format): sorted by Start, so
            // stop at the first one past the window.
            if e.start_ms >= to_ms {
                break;
            }
            if e.end_ms > from_ms {
                out.push(e);
            }
        }
        next = list.call_obj("GetNext", &[]);
    }
    out
}

/// An appointment as an Event; None for anything else, cancelled or declined.
fn event_of(item: &Disp, calendar: &str) -> Option<Event> {
    if item.int("Class") != APPOINTMENT_CLASS || CANCELED.contains(&item.int("MeetingStatus")) || item.int("ResponseStatus") == DECLINED {
        return None;
    }
    let start_ms = ole_to_unix_ms(date(item, "StartUTC")?);
    let end_ms = ole_to_unix_ms(date(item, "EndUTC")?);
    let location = item.text("Location");
    // The body only to find a meeting link; it is dropped right here.
    let join = join_link(&[location.as_str(), item.text("Body").as_str()]);
    Some(Event { title: item.text("Subject"), location, start_ms, end_ms: end_ms.max(start_ms), all_day: flag(item, "AllDayEvent"), calendar: calendar.to_string(), join })
}

/// The PC's UTC offset in minutes (local − UTC), from Windows' own clocks.
pub fn utc_offset_minutes() -> i32 {
    use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    let (l, u) = unsafe { (GetLocalTime(), GetSystemTime()) };
    let mins = |t: &windows::Win32::Foundation::SYSTEMTIME| {
        super::days_from_civil(i64::from(t.wYear), i64::from(t.wMonth), i64::from(t.wDay)) * 1_440 + i64::from(t.wHour) * 60 + i64::from(t.wMinute)
    };
    (mins(&l) - mins(&u)) as i32
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_only_never_starts_outlook_never_changes_an_item() {
        let src = include_str!("com.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for banned in ["\"Save\"", "\"Send\"", "\"Respond\"", "\"Display\"", "\"Delete\"", "\"Move\"", "CoCreateInstance", "\"GetInspector\""] {
            assert!(!code.contains(banned), "{banned}");
        }
        assert!(code.contains("GetActiveObject(&clsid"), "attaches to a running Outlook only");
        assert!(!code.contains("log::line"), "nothing read here is logged");
    }

    #[test]
    fn the_utc_offset_is_whole_minutes_within_a_day() {
        let o = super::utc_offset_minutes();
        assert!((-14 * 60..=14 * 60).contains(&o), "{o}");
    }
}

/// The read-only probe against the real calendar (owner's OK, 2026-10-17),
/// run by hand, never in CI:
///   GLIM_LIVE_CALENDAR=1 cargo test --lib calendar::com::live -- --ignored --nocapture
/// Prints counts and times only: never a title, location, calendar name or link.
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore = "live: reads the running Outlook's calendar (read only); needs GLIM_LIVE_CALENDAR=1"]
    fn probe() {
        assert_eq!(std::env::var("GLIM_LIVE_CALENDAR").as_deref(), Ok("1"));
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let t0 = std::time::Instant::now();
        let cal = match Calendar::attach() {
            Ok(c) => c,
            Err(e) => panic!("not attached: {}", e.code()),
        };
        let offset = utc_offset_minutes();
        println!("attached in {:.2}s; UTC offset {offset} min", t0.elapsed().as_secs_f64());
        let folders = cal.calendars();
        println!("calendars found: {} (default first: {})", folders.len(), cal.default_calendar().is_some_and(|d| folders.first().is_some_and(|(p, _)| *p == d)));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let t1 = std::time::Instant::now();
        let read = cal.events(&[], now, now + crate::calendar::WINDOW_MS, offset).expect("read");
        println!("default calendar: {} events overlapping the next 24 h, read in {:.2}s", read.len(), t1.elapsed().as_secs_f64());
        let hm = |ms: i64| {
            let local = ms + i64::from(offset) * 60_000;
            let m = local.rem_euclid(86_400_000) / 60_000;
            format!("{} {:02}:{:02}", crate::calendar::local_date(ms, offset), m / 60, m % 60)
        };
        for e in &read {
            println!("  {} → {} ({} min){}{}", hm(e.start_ms), hm(e.end_ms), (e.end_ms - e.start_ms) / 60_000, if e.all_day { ", all day" } else { "" }, if e.join.is_some() { ", meeting link" } else { "" });
        }
        let shown = crate::calendar::in_window(read.clone(), now, true);
        println!("shown (all-day hidden): {}; next: {}", shown.len(), crate::calendar::next(&shown, now).map(|(i, same)| format!("in {} min (+{same} at the same time)", (shown[i].start_ms - now) / 60_000)).unwrap_or_else(|| "none".into()));
        // Every calendar, counts only.
        let ids: Vec<String> = folders.iter().map(|(p, _)| p.clone()).collect();
        let all = cal.events(&ids, now, now + crate::calendar::WINDOW_MS, offset).expect("read all");
        println!("all {} calendars: {} events", ids.len(), all.len());
        // A week ahead: occurrences of a recurring meeting come one by one
        // (counted by repeated titles; the titles aren't printed).
        let week = cal.events(&[], now, now + 7 * crate::calendar::WINDOW_MS, offset).expect("read week");
        let mut titles: Vec<&str> = week.iter().map(|e| e.title.as_str()).collect();
        titles.sort_unstable();
        let total = titles.len();
        titles.dedup();
        println!("next 7 days: {total} events, {} distinct titles ({} repeated occurrences), {} all-day, {} with a meeting link", titles.len(), total - titles.len(), week.iter().filter(|e| e.all_day).count(), week.iter().filter(|e| e.join.is_some()).count());
    }
}
