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
