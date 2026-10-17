// Calendar glance (roadmap, owner 2026-10-17): the island shows the next
// event from classic Outlook's calendar, the lantern glows a few minutes
// before it, a Join button appears for a Teams or Zoom link found in the
// event, and an optional morning summary lists the day.
//
// Read only: Outlook's calendar through local COM (com.rs), today and the
// next 24 h. Nothing is created, edited or answered, no window opens, and
// Outlook is never started for this: if it isn't running, there is nothing
// to show. Glim makes no network call; Join opens the meeting link in the
// user's browser or Teams/Zoom app, on a click, and only for a link whose
// host is Teams' or Zoom's (meeting_link).
//
// Titles and locations go to the island (on this PC) and nowhere else: never
// to the log (counts and times only), never to the API.
//
// This file is the pure part: times, the window, the next event, the Join
// link, the morning summary. com.rs reads Outlook, worker.rs runs the polling
// thread and talks to the island.

pub mod com;
pub mod worker;

use serde::Serialize;

/// How far ahead Glim looks: today and the next 24 hours.
pub const WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// An event that started less than this ago is still "now" (late joiners).
pub const STILL_NOW_MS: i64 = 10 * 60 * 1000;
/// The morning summary waits for this local hour.
pub const SUMMARY_HOUR: u32 = 6;

/// One event as Glim keeps it. `join` is a checked Teams/Zoom link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub title: String,
    pub location: String,
    /// UTC, milliseconds since 1970.
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    /// Which calendar it came from (Outlook's folder path).
    pub calendar: String,
    pub join: Option<String>,
}

/// An OLE automation date (days since 1899-12-30, what COM's VT_DATE holds;
/// Outlook's StartUTC/EndUTC are in UTC) → milliseconds since 1970, UTC.
pub fn ole_to_unix_ms(days: f64) -> i64 {
    ((days - 25_569.0) * 86_400_000.0).round() as i64
}

/// Hosts a Join link may point at (exact, or a subdomain of the Zoom ones).
const TEAMS_HOSTS: [&str; 3] = ["teams.microsoft.com", "teams.live.com", "teams.microsoft.us"];
const ZOOM_HOSTS: [&str; 2] = ["zoom.us", "zoomgov.com"];

/// A meeting link Glim will open: https, a Teams or Zoom host, a meeting
/// path, no credentials in it. Anything else is no Join button.
pub fn meeting_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    // The host is only letters, digits, dots and hyphens: "evil.com#.zoom.us",
    // "evil.com\.zoom.us" or "evil.com?.zoom.us" would reach evil.com in a
    // browser (security review, 2026-10-17). No user, no port.
    if authority.is_empty() || !authority.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') || authority.starts_with('.') || authority.contains("..") {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '<' || c == '>') {
        return false;
    }
    let teams = TEAMS_HOSTS.contains(&host.as_str()) && (path.starts_with("l/meetup-join/") || path.starts_with("meet/") || path.starts_with("l/meet/"));
    let zoom = ZOOM_HOSTS.iter().any(|z| host == *z || host.ends_with(&format!(".{z}"))) && (path.starts_with("j/") || path.starts_with("w/") || path.starts_with("my/") || path.starts_with("s/"));
    teams || zoom
}

/// The first Teams or Zoom meeting link in the event's own text (its
/// location, then its body), or None. Only https links are looked at.
pub fn join_link(texts: &[&str]) -> Option<String> {
    for text in texts {
        let mut from = 0;
        while let Some(i) = text[from..].find("https://") {
            let start = from + i;
            let end = text[start..].find(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>' | ')' | ']' | '|')).map_or(text.len(), |e| start + e);
            let url = text[start..end].trim_end_matches(['.', ',', ';']);
            if meeting_link(url) {
                return Some(url.to_string());
            }
            from = end.max(start + 1);
        }
    }
    None
}

/// The events Glim shows: overlapping now … now + 24 h, all-day ones only
/// when not hidden, ended ones out, sorted by start (then end, then title),
/// the same event from two calendars once.
pub fn in_window(mut events: Vec<Event>, now_ms: i64, hide_all_day: bool) -> Vec<Event> {
    events.retain(|e| e.end_ms > now_ms && e.start_ms < now_ms + WINDOW_MS && !(hide_all_day && e.all_day));
    events.sort_by(|a, b| (a.start_ms, a.end_ms, &a.title).cmp(&(b.start_ms, b.end_ms, &b.title)));
    events.dedup_by(|a, b| a.start_ms == b.start_ms && a.end_ms == b.end_ms && a.title == b.title && a.all_day == b.all_day);
    events
}

/// The event the island shows: the first timed one that hasn't started, or
/// started less than STILL_NOW_MS ago; and how many others start at the
/// same time. All-day events never are "next" (they're on the list only).
pub fn next(events: &[Event], now_ms: i64) -> Option<(usize, usize)> {
    let i = events.iter().position(|e| !e.all_day && e.end_ms > now_ms && e.start_ms + STILL_NOW_MS > now_ms)?;
    let same = events.iter().enumerate().filter(|(j, e)| *j != i && !e.all_day && e.start_ms == events[i].start_ms).count();
    Some((i, same))
}

/// (year, month, day) for days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Days since 1970-01-01 for a civil date (the inverse of civil_from_days).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The local date ("YYYY-MM-DD") at `ms` for a UTC offset in minutes.
pub fn local_date(ms: i64, offset_min: i32) -> String {
    let (y, m, d) = civil_from_days((ms + i64::from(offset_min) * 60_000).div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// A time as Outlook's Items.Restrict wants it: local, "10/17/2026 03:05 PM"
/// (the US short format; com.rs falls back to reading in order if the PC's
/// format refuses it).
pub fn restrict_date(ms: i64, offset_min: i32) -> String {
    let local = ms + i64::from(offset_min) * 60_000;
    let (y, m, d) = civil_from_days(local.div_euclid(86_400_000));
    let mins = local.rem_euclid(86_400_000) / 60_000;
    let (h, mi) = (mins / 60, mins % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{m:02}/{d:02}/{y:04} {h12:02}:{mi:02} {ampm}")
}

/// The local hour at `ms`.
pub fn local_hour(ms: i64, offset_min: i32) -> u32 {
    ((ms + i64::from(offset_min) * 60_000).rem_euclid(86_400_000) / 3_600_000) as u32
}

/// The morning summary is due: switched on, past SUMMARY_HOUR, and not yet
/// shown today (local date).
pub fn summary_due(on: bool, last_shown: &str, now_ms: i64, offset_min: i32) -> bool {
    on && local_hour(now_ms, offset_min) >= SUMMARY_HOUR && last_shown != local_date(now_ms, offset_min)
}

/// Today's events (local date of their start), for the summary.
pub fn today(events: &[Event], now_ms: i64, offset_min: i32) -> Vec<Event> {
    let d = local_date(now_ms, offset_min);
    events.iter().filter(|e| local_date(e.start_ms.max(now_ms), offset_min) == d).cloned().collect()
}

/// What the island gets: the events in view (with `hasJoin`, never the link
/// itself), whether Outlook was reachable, and the summary when it's due.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Glance {
    /// The setting is on and classic Outlook is running.
    pub available: bool,
    /// Why not, when it isn't: "off", "no-outlook", "failed", or "".
    pub code: &'static str,
    pub events: Vec<ShownEvent>,
    /// Bumped on every read; Join names an event by (generation, index).
    pub generation: u64,
    /// Show the morning summary now (once a day).
    pub summary: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShownEvent {
    pub title: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub has_join: bool,
}

impl ShownEvent {
    pub fn of(e: &Event) -> ShownEvent {
        ShownEvent { title: e.title.clone(), location: e.location.clone(), start: e.start_ms, end: e.end_ms, all_day: e.all_day, has_join: e.join.is_some() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;
    const M: i64 = 60_000;
    /// 2026-10-17 15:00 UTC (08:00 in Los Angeles, PDT = UTC−7).
    const NOW: i64 = 1_792_249_200_000;

    fn ev(title: &str, start: i64, mins: i64) -> Event {
        Event { title: title.into(), location: String::new(), start_ms: start, end_ms: start + mins * M, all_day: false, calendar: "Calendar".into(), join: None }
    }

    #[test]
    fn outlook_dates_become_utc_milliseconds() {
        assert_eq!(local_date(NOW, 0), "2026-10-17");
        // 2026-10-17 15:00 UTC as an OLE date: 46312.625.
        assert_eq!(ole_to_unix_ms(46_312.625), NOW);
        assert_eq!(ole_to_unix_ms(25_569.0), 0);
        assert_eq!(local_hour(NOW, -7 * 60), 8);
        assert_eq!(local_hour(NOW, 9 * 60), 0);
        assert_eq!(local_date(NOW, 9 * 60), "2026-10-18", "Tokyo is already tomorrow");
        assert_eq!(local_date(NOW - 16 * H, -7 * 60), "2026-10-16");
        assert_eq!(local_date(0, 0), "1970-01-01");
        assert_eq!(local_date(951_782_400_000, 0), "2000-02-29");
        assert_eq!(days_from_civil(2026, 10, 17), NOW.div_euclid(86_400_000));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 2, 29) * 86_400_000, 951_782_400_000);
        // Outlook's Restrict wants local time.
        assert_eq!(restrict_date(NOW, -7 * 60), "10/17/2026 08:00 AM");
        assert_eq!(restrict_date(NOW + 5 * M, 0), "10/17/2026 03:05 PM");
        assert_eq!(restrict_date(NOW + 9 * H, 0), "10/18/2026 12:00 AM");
        assert_eq!(restrict_date(NOW - 3 * H, 0), "10/17/2026 12:00 PM");
    }

    #[test]
    fn only_https_teams_and_zoom_meeting_links_get_a_join_button() {
        for ok in [
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%7d",
            "https://teams.live.com/meet/9876543210",
            "https://us02web.zoom.us/j/81234567890?pwd=abc",
            "https://zoom.us/j/123",
            "https://acme.zoom.us/my/anthony",
        ] {
            assert!(meeting_link(ok), "{ok}");
        }
        for bad in [
            "http://zoom.us/j/123",
            "https://zoom.us.evil.example/j/1",
            "https://evilzoom.us/j/1",
            "https://teams.microsoft.com.evil.example/l/meetup-join/x",
            "https://user:pw@zoom.us/j/1",
            "https://zoom.us:8443/j/1",
            "https://zoom.us/signin",
            "https://teams.microsoft.com/",
            "https://example.com/j/1",
            "javascript:alert(1)",
            // Host-check bypasses: the browser would go to evil.com.
            "https://evil.com#.zoom.us/j/1",
            "https://evil.com?.zoom.us/j/1",
            "https://evil.com\\.zoom.us/j/1",
            "https://evil.com%2F.zoom.us/j/1",
            "https://.zoom.us/j/1",
            "https://teams.microsoft.com\\@evil.com/l/meetup-join/x",
        ] {
            assert!(!meeting_link(bad), "{bad}");
        }
        // Found in the location first, then the body; text around it ignored.
        let body = "Join on your computer\r\n<https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=a>\r\nMeeting ID: 123";
        assert_eq!(join_link(&["Room 4B", body]).as_deref(), Some("https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=a"));
        assert_eq!(join_link(&["https://us02web.zoom.us/j/811?pwd=x.", body]).as_deref(), Some("https://us02web.zoom.us/j/811?pwd=x"));
        assert_eq!(join_link(&["see https://example.com/agenda and http://zoom.us/j/1"]), None);
        assert_eq!(join_link(&[""]), None);
    }

    #[test]
    fn the_window_is_now_to_24_hours_sorted_with_all_day_hidden_by_default() {
        let mut all_day = ev("Holiday", NOW - 8 * H, 24 * 60);
        all_day.all_day = true;
        let events = vec![
            ev("Later", NOW + 3 * H, 30),
            ev("Ended", NOW - 2 * H, 30),
            ev("Tomorrow evening", NOW + 30 * H, 30),
            ev("Soon", NOW + 20 * M, 30),
            all_day.clone(),
            // The same meeting in two calendars: once.
            ev("Soon", NOW + 20 * M, 30),
        ];
        let shown = in_window(events.clone(), NOW, true);
        assert_eq!(shown.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Soon", "Later"]);
        let with_all_day = in_window(events, NOW, false);
        assert_eq!(with_all_day.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Holiday", "Soon", "Later"]);
        // An all-day event is never "next".
        assert_eq!(next(&with_all_day, NOW).map(|(i, _)| with_all_day[i].title.as_str()), Some("Soon"));
    }

    #[test]
    fn recurring_occurrences_overlaps_and_late_joining() {
        // A daily stand-up: Outlook hands each occurrence over separately.
        let standup = |day: i64| ev("Stand-up", NOW + day * 24 * H - H, 15);
        let events = in_window(vec![standup(0), standup(1), standup(2)], NOW, true);
        assert_eq!(events.len(), 1, "today's has ended, tomorrow's (+23 h) is in the window, the day after's isn't");
        assert_eq!(events[0].start_ms, NOW + 23 * H);
        let events = in_window(vec![standup(1), ev("1:1", NOW + 2 * H, 30)], NOW, true);
        assert_eq!(events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["1:1", "Stand-up"]);
        // Two at the same time: the first by end, then title; the other counted.
        let events = in_window(vec![ev("Board", NOW + H, 60), ev("Audit", NOW + H, 60), ev("Lunch", NOW + 4 * H, 60)], NOW, true);
        assert_eq!(next(&events, NOW), Some((0, 1)));
        assert_eq!(events[0].title, "Audit");
        // Overlapping but not the same start: the earlier one is next.
        let events = in_window(vec![ev("Long", NOW + H, 120), ev("Inside", NOW + H + 30 * M, 15)], NOW, true);
        assert_eq!(next(&events, NOW), Some((0, 0)));
        // Started 5 minutes ago: still "now" (join late); 15 minutes ago: the next one.
        let events = in_window(vec![ev("Started", NOW - 5 * M, 60), ev("After", NOW + H, 30)], NOW, true);
        assert_eq!(next(&events, NOW), Some((0, 0)));
        let events = in_window(vec![ev("Started", NOW - 15 * M, 60), ev("After", NOW + H, 30)], NOW, true);
        assert_eq!(next(&events, NOW).map(|(i, _)| events[i].title.as_str()), Some("After"));
        assert_eq!(next(&[], NOW), None);
    }

    #[test]
    fn the_morning_summary_once_a_day_after_six_in_the_users_time_zone() {
        let la = -7 * 60;
        // 08:00 in LA, not shown today yet.
        assert!(summary_due(true, "2026-10-16", NOW, la));
        assert!(!summary_due(true, "2026-10-17", NOW, la), "once a day");
        assert!(!summary_due(false, "", NOW, la), "off by default");
        // 05:30 in LA: too early.
        assert!(!summary_due(true, "", NOW - 150 * M, la));
        // Tokyo: it's midnight there, so not yet.
        assert!(!summary_due(true, "", NOW, 9 * 60));
        // Today = the local date: an event at 23:30 LA is today, one at 00:30 is tomorrow.
        let events = vec![ev("Late", NOW + 15 * H + 30 * M, 30), ev("Next day", NOW + 16 * H + 30 * M, 30)];
        assert_eq!(today(&events, NOW, la).iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Late"]);
        // In UTC the same 23:30-LA event is already tomorrow.
        assert!(today(&events, NOW, 0).is_empty());
    }

    #[test]
    fn the_island_gets_has_join_never_the_link() {
        let mut e = ev("Standup", NOW + H, 15);
        e.join = Some("https://zoom.us/j/1".into());
        let shown = serde_json::to_value(ShownEvent::of(&e)).unwrap();
        assert_eq!(shown["hasJoin"], true);
        assert!(!shown.to_string().contains("zoom.us"), "the link stays in Rust");
    }
}
