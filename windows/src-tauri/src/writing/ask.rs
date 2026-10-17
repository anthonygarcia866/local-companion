// The one-time "Check writing in [app]?" prompt (1d, docs/phase1-plan.md
// §14.2), for apps on neither of writing::apps' lists. The capture thread
// reads nothing of these apps but the exe name, the control type and the
// editable signals (no text, no labels), and offers the question after 2 s in
// an editable field, at most once a day per app. The island shows it (or
// drops it while Hidden or fullscreen) and Settings stores the answer.

use std::time::{Duration, Instant};

use serde::Serialize;

use super::prefs::Prefs;

/// How long the user must stay in an editable field first.
pub const DWELL: Duration = Duration::from_secs(2);
/// The event the island hears the question on.
pub const EVENT: &str = "writing-ask";

/// Windows' own shell: never asked about (§14.2).
pub const NEVER_ASK: [&str; 6] = [
    "explorer.exe", "SearchHost.exe", "StartMenuExperienceHost.exe", "ShellExperienceHost.exe", "LockApp.exe",
    "TextInputHost.exe",
];

/// The question, for the island: the exe (what the answer is stored under)
/// and the name to show. Never a window title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Ask {
    pub exe: String,
    pub name: String,
}

/// Today's local date, "YYYY-MM-DD": what "once a day" counts in.
pub fn today() -> String {
    let t = crate::platform::local_time();
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

/// Whether `exe` may be asked about today (`today` = local "YYYY-MM-DD"): on
/// neither built-in list, not answered (Yes or Never), not the shell, not
/// Glim, and not asked yet today. Paused apps never get this far: capture
/// reads nothing of them.
pub fn askable(exe: &str, prefs: &Prefs, today: &str) -> bool {
    let lower = exe.to_ascii_lowercase();
    valid_exe(exe)
        && !super::apps::is_known(exe)
        && !prefs.apps.contains_key(&lower)
        && !NEVER_ASK.iter().any(|n| n.eq_ignore_ascii_case(exe))
        && lower != "glim.exe"
        && prefs.asked.get(&lower).is_none_or(|d| d.as_str() < today)
}

/// A bare executable file name, as an answer may be stored under: ends in
/// ".exe", no path, nothing a file name can't hold.
pub fn valid_exe(exe: &str) -> bool {
    let stem = exe.len().checked_sub(4).and_then(|n| exe.get(..n)).unwrap_or("");
    exe.len() <= 128
        && exe.to_ascii_lowercase().ends_with(".exe")
        && !stem.trim().is_empty()
        && !exe.chars().any(|c| c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// Watches where the user is, reading by reading, and says when to ask.
#[derive(Default)]
pub struct Asker {
    /// The askable app whose editable field the user is in, and since when.
    at: Option<(String, Instant)>,
    /// Already offered for this stay in the field.
    offered: bool,
}

impl Asker {
    /// One reading. `field`: the app, when the focused element is an
    /// editable field of an askable app; None otherwise. `same`: the same
    /// element as the last reading. Returns the app to ask about, once per
    /// stay in a field.
    pub fn feed(&mut self, field: Option<&str>, same: bool, now: Instant) -> Option<String> {
        let Some(exe) = field else {
            *self = Asker::default();
            return None;
        };
        let fresh = !same || self.at.as_ref().is_none_or(|(e, _)| e != exe);
        if fresh {
            self.at = Some((exe.to_string(), now));
            self.offered = false;
            return None;
        }
        let since = self.at.as_ref().map(|(_, t)| *t)?;
        if !self.offered && now.duration_since(since) >= DWELL {
            self.offered = true;
            return Some(exe.to_string());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(apps: &[(&str, bool)], asked: &[(&str, &str)]) -> Prefs {
        Prefs {
            apps: apps.iter().map(|(e, on)| (e.to_string(), *on)).collect(),
            asked: asked.iter().map(|(e, d)| (e.to_string(), d.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn only_apps_on_neither_list_unanswered_and_not_asked_today_are_asked_about() {
        let p = prefs(&[("zed.exe", false), ("notion.exe", true)], &[("acrobat.exe", "2026-10-13"), ("wordpad.exe", "2026-10-12")]);
        let today = "2026-10-13";
        assert!(askable("SomeNewApp.exe", &p, today));
        assert!(askable("WORDPAD.EXE", &p, today), "asked yesterday: again today");
        assert!(!askable("Acrobat.exe", &p, today), "asked today already");
        assert!(!askable("Zed.exe", &p, today), "answered Never");
        assert!(!askable("Notion.exe", &p, today), "answered Yes");
        for exe in ["WINWORD.EXE", "Code.exe", "cmd.exe", "glim.exe", "explorer.exe", "searchhost.exe", "LockApp.exe", "TextInputHost.exe", "pid 1234", ""] {
            assert!(!askable(exe, &p, today), "{exe}");
        }
        assert!(askable("Acrobat.exe", &p, "2026-10-14"), "the next day");
    }

    #[test]
    fn answers_are_stored_only_under_a_bare_exe_name() {
        for ok in ["Acrobat.exe", "WORDPAD.EXE", "Grok Bot.exe", "app-1.2.exe"] {
            assert!(valid_exe(ok), "{ok}");
        }
        for bad in ["", ".exe", "  .exe", "Acrobat", "C:\\Apps\\a.exe", "a/b.exe", "a\nb.exe", "a?.exe", &format!("{}.exe", "x".repeat(130))] {
            assert!(!valid_exe(bad), "{bad:?}");
        }
    }

    /// §14.2: nothing of an app on neither list is read before Yes but its
    /// exe, its control type, the password flag and the editable signals.
    #[test]
    fn no_text_labels_title_or_url_is_read_to_decide_whether_to_ask() {
        let src = include_str!("../capture/uia.rs");
        let read = &src[src.find("fn read(").unwrap()..];
        let from = read.find("let ask =").unwrap();
        let to = read.find("SKIPPED_OFF.into()").unwrap();
        assert!(from < to);
        let decided = &read[from..to];
        for banned in ["read_text", "field_labels", "page_urls", "foreground_title", "GetText", "CurrentName", "CurrentValue"] {
            assert!(!decided.contains(banned), "{banned} before an app is on");
        }
    }

    #[test]
    fn the_question_comes_after_two_seconds_in_a_field_once_per_stay() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut a = Asker::default();
        assert_eq!(a.feed(Some("Acrobat.exe"), false, at(0)), None);
        assert_eq!(a.feed(Some("Acrobat.exe"), true, at(1500)), None);
        assert_eq!(a.feed(Some("Acrobat.exe"), true, at(2100)).as_deref(), Some("Acrobat.exe"));
        assert_eq!(a.feed(Some("Acrobat.exe"), true, at(5000)), None, "once per stay");
        // Another field starts the wait again.
        assert_eq!(a.feed(Some("Acrobat.exe"), false, at(5300)), None);
        assert_eq!(a.feed(Some("Acrobat.exe"), true, at(7400)).as_deref(), Some("Acrobat.exe"));
        // Leaving for something not askable (another app, a read-only view) resets it.
        assert_eq!(a.feed(None, false, at(7700)), None);
        assert_eq!(a.feed(Some("Acrobat.exe"), false, at(8000)), None);
        assert_eq!(a.feed(None, true, at(9000)), None);
        assert_eq!(a.feed(Some("Acrobat.exe"), true, at(10500)), None, "the wait restarted");
    }
}
