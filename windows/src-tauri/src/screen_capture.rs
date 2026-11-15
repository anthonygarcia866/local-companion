// Keep Glim's windows out of screen shares, recordings and screenshots
// (docs/glim-learns-plan.md §5; decided 2026-10-24, §9: a Settings switch,
// on by default). Every Glim window gets WDA_EXCLUDEFROMCAPTURE, so a Teams
// share or a recording shows what's behind the island's cards (transcripts,
// drafts), not the cards.
//
// `GLIM_CAPTURABLE=1` in Glim's environment turns it off whatever the
// setting says: a test-only override for the PrintWindow verify scripts
// (scripts/verify-*.ps1). Checked 2026-10-25 on Windows 11 with a window of
// the probe's own: once excluded, PrintWindow with PW_RENDERFULLCONTENT (what
// the scripts use, WebView2 needs it) comes back black; PrintWindow without
// it still drew that plain window, and a screen copy was black.
//
// "Allow screenshots for 10 minutes" (owner 2026-11-14; tray + Settings →
// Privacy): lets Glim's windows be captured for ALLOW_FOR, with a countdown
// on the island, then hides them again by itself. It lives only in memory
// (never in Glim's saved settings), so a restart, a crash or a crash recovery always
// comes back hidden; the setting itself never changes.
//
// The log says on/off and counts only.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// The test-only override: "1" lets every Glim window be captured.
pub const OVERRIDE_VAR: &str = "GLIM_CAPTURABLE";

/// How long "Allow screenshots" lasts.
pub const ALLOW_FOR: Duration = Duration::from_secs(10 * 60);

/// The event every window hears: the time (Unix ms) screenshots are allowed
/// until, or 0 once Glim hides again.
pub const EVENT: &str = "capture-allowed";

/// Whether Glim's windows are kept out of capture: the setting, unless the
/// override is set to 1 or screenshots are allowed for a while.
pub fn wanted(setting: bool, override_var: Option<&str>, allowed: bool) -> bool {
    setting && !allowed && override_var.map(str::trim) != Some("1")
}

/// Why, in the log: "on", "off (setting)", "off (GLIM_CAPTURABLE)",
/// "off (allowed for 10 minutes)".
fn reason(setting: bool, override_var: Option<&str>, allowed: bool) -> &'static str {
    if wanted(setting, override_var, allowed) {
        "on"
    } else if !setting {
        "off (setting)"
    } else if override_var.map(str::trim) == Some("1") {
        "off (GLIM_CAPTURABLE)"
    } else {
        "off (allowed for 10 minutes)"
    }
}

/// The allow window: (until, Unix ms; generation). Memory only, on purpose.
static ALLOWED: Mutex<(u64, u64)> = Mutex::new((0, 0));

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The allow window's end, if it is still open at `now`.
fn open_until(until: u64, now: u64) -> u64 {
    if until > now { until } else { 0 }
}

/// Screenshots allowed until (Unix ms), or 0.
pub fn allowed_until() -> u64 {
    open_until(ALLOWED.lock().unwrap().0, now_ms())
}

fn allowed() -> bool {
    allowed_until() > 0
}

/// Opens (or restarts) the allow window: every window capturable now, hidden
/// again by itself after ALLOW_FOR, checked by the wall clock each second so
/// a PC that slept through the end hides at once when it wakes.
pub fn allow_for_a_while(app: &AppHandle) -> u64 {
    let until = now_ms() + ALLOW_FOR.as_millis() as u64;
    let generation = {
        let mut a = ALLOWED.lock().unwrap();
        a.1 += 1;
        a.0 = until;
        a.1
    };
    crate::log::line("capture-exclude: screenshots allowed for 10 minutes");
    changed(app);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("capture-allow".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let (until, current) = *ALLOWED.lock().unwrap();
        if current != generation {
            return;
        }
        if now_ms() >= until {
            end(&app, "time's up");
            return;
        }
    });
    until
}

/// Hides again now ("Hide again" in the tray or Settings, or time's up).
pub fn end(app: &AppHandle, why: &str) {
    {
        let mut a = ALLOWED.lock().unwrap();
        if a.0 == 0 {
            return;
        }
        a.0 = 0;
        a.1 += 1;
    }
    crate::log::line(format!("capture-exclude: hidden again ({why})"));
    changed(app);
}

fn changed(app: &AppHandle) {
    apply(app);
    let _ = app.emit(EVENT, allowed_until());
    crate::tray::retitle(app);
}

/// What was logged last, so a save that changes nothing logs nothing, and
/// "unsupported" is said once.
static LOGGED: Mutex<(Option<&'static str>, bool)> = Mutex::new((None, false));

fn setting(app: &AppHandle) -> bool {
    app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().hide_from_capture).unwrap_or(true)
}

fn override_var() -> Option<String> {
    std::env::var(OVERRIDE_VAR).ok()
}

/// Whether Glim's windows should be kept out of capture right now.
pub fn wanted_now(app: &AppHandle) -> bool {
    wanted(setting(app), override_var().as_deref(), allowed())
}

/// Every Glim window as the setting says: at launch, once its windows
/// exist, after each settings save, and when the allow window opens or ends.
pub fn apply(app: &AppHandle) {
    let (setting, var, allowed) = (setting(app), override_var(), allowed());
    let want = wanted(setting, var.as_deref(), allowed);
    let windows = app.webview_windows();
    let refused = windows.values().filter(|win| !crate::platform::set_capture_excluded(win, want)).count();
    windows.values().for_each(note_visible);
    note(reason(setting, var.as_deref(), allowed), windows.len(), want && refused > 0);
}

/// Right after Glim shows a window. A hidden window is never touched
/// (platform::set_capture_excluded: on hidden Settings it left a white copy
/// on screen, 2026-10-27), so the window gets the setting here.
pub fn after_show(app: &AppHandle, win: &WebviewWindow) {
    apply_to(app, win);
}

/// A window built after launch (the mini lantern, the desktop lantern).
pub fn apply_to(app: &AppHandle, win: &WebviewWindow) {
    let want = wanted_now(app);
    if !crate::platform::set_capture_excluded(win, want) && want {
        note("on", 1, true);
    }
    note_visible(win);
}

/// For glim.exe --selftest: which windows were visible when the affinity was
/// applied (only those may keep one once hidden).
fn note_visible(win: &WebviewWindow) {
    if crate::selftest::on() && crate::platform::window_state(win).is_some_and(|(visible, _)| visible) {
        crate::selftest::applied_while_visible(win.label());
    }
}

fn note(reason: &'static str, windows: usize, refused: bool) {
    let mut logged = LOGGED.lock().unwrap();
    if logged.0 != Some(reason) {
        logged.0 = Some(reason);
        crate::log::line(format!("capture-exclude: {reason} ({windows} windows)"));
    }
    if refused && !logged.1 {
        logged.1 = true;
        crate::log::line("capture-exclude: unsupported");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_decides_unless_the_test_override_is_one() {
        assert!(wanted(true, None, false));
        assert!(!wanted(false, None, false));
        assert!(!wanted(true, Some("1"), false));
        assert!(!wanted(true, Some(" 1 "), false));
        assert!(!wanted(false, Some("1"), false));
        // Anything but 1 leaves the setting in charge.
        for other in ["", "0", "true", "yes", "2"] {
            assert!(wanted(true, Some(other), false), "{other:?}");
        }
    }

    #[test]
    fn allowing_screenshots_lets_capture_through_only_while_the_window_is_open() {
        assert!(!wanted(true, None, true));
        assert!(!wanted(false, None, true));
        assert_eq!(open_until(1_000, 999), 1_000);
        assert_eq!(open_until(1_000, 1_000), 0, "ends exactly on time");
        assert_eq!(open_until(1_000, 5_000), 0, "a PC that slept past the end is hidden at once");
        assert_eq!(open_until(0, 0), 0);
        assert_eq!(ALLOW_FOR, Duration::from_secs(600));
    }

    /// A restart or crash must come back hidden: the allow window is never
    /// written anywhere (no settings field, no file), and nothing at launch
    /// opens it.
    #[test]
    fn the_allow_window_is_memory_only() {
        let code = include_str!("screen_capture.rs").replace("\r\n", "\n");
        let code = code.split("#[cfg(test)]").next().unwrap();
        for disk in ["std::fs", "fs::write", "settings.json", "update_settings", "hide_from_capture ="] {
            assert!(!code.contains(disk), "{disk}");
        }
        let settings = include_str!("settings.rs");
        assert!(!settings.contains("capture_allowed") && !settings.contains("captureAllowed"));
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let setup = &lib[lib.find(".setup(move |app|").unwrap()..];
        let setup = &setup[..setup.find("\n        })\n").unwrap_or(setup.len())];
        assert!(!setup.contains("allow_for_a_while"), "never opened at launch");
        assert_eq!(*ALLOWED.lock().unwrap(), (0, 0), "starts closed");
    }

    #[test]
    fn the_log_says_why_and_never_more() {
        assert_eq!(reason(true, None, false), "on");
        assert_eq!(reason(true, Some("1"), false), "off (GLIM_CAPTURABLE)");
        assert_eq!(reason(false, None, false), "off (setting)");
        assert_eq!(reason(false, Some("1"), false), "off (setting)");
        assert_eq!(reason(true, None, true), "off (allowed for 10 minutes)");
    }

    /// 2026-10-27: SetWindowDisplayAffinity on the hidden Settings window left a
    /// white, click-through copy of it on screen at every launch (#56's launch
    /// apply). Hidden windows are skipped, and each show applies it after.
    #[test]
    fn a_hidden_window_is_never_touched_and_each_show_applies_it_after() {
        let platform = include_str!("platform/windows.rs").replace("\r\n", "\n");
        let set = &platform[platform.find("fn set_hwnd_capture_excluded").unwrap()..];
        let set = &set[..set.find("\n}\n").unwrap()];
        let hidden = set.find("if !IsWindowVisible(hwnd).as_bool() {").expect("hidden windows skipped");
        assert!(hidden < set.find("SetWindowDisplayAffinity(hwnd").unwrap());
        assert!(set.find("GetWindowDisplayAffinity").unwrap() < set.find("SetWindowDisplayAffinity(hwnd").unwrap(), "no call when nothing changes");
        // Every place Glim shows a window applies it right after the show.
        for (file, code, show) in [
            ("lib.rs settings", include_str!("lib.rs"), "    let _ = win.show();\n"),
            ("lib.rs island", include_str!("lib.rs"), "                let _ = win.show();\n"),
            ("island.rs", include_str!("island.rs"), "        let _ = win.show();\n"),
            ("desktop.rs", include_str!("desktop.rs"), "            let _ = win.show();\n"),
            ("webview_health.rs", include_str!("webview_health.rs"), "        let _ = win.show();\n"),
        ] {
            let code = code.replace("\r\n", "\n");
            let at = code.find(show).unwrap_or_else(|| panic!("{file}: show"));
            let next = &code[at + show.len()..];
            let next = next.lines().find(|l| !l.trim().starts_with("//")).unwrap();
            assert!(next.contains("screen_capture::after_show("), "{file}: {next}");
        }
    }

    #[test]
    fn every_window_glim_builds_is_covered() {
        // CI checks the sources out with CRLF (core.autocrlf): compare as LF.
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let setup = &lib[lib.find(".setup(move |app|").unwrap()..];
        let applied = setup.find("screen_capture::apply(&handle);").expect("applied at launch");
        for built in ["create_settings_window(&handle);", "create_capture_panel(&handle);", "desktop::setup(&handle);"] {
            assert!(setup.find(built).unwrap() < applied, "{built} comes before the launch apply");
        }
        assert!(applied < setup.find("let _ = win.show();").unwrap(), "applied before the island is first shown");
        let save = &lib[lib.find("fn save_settings").unwrap()..];
        assert!(save[..save.find("\n}\n").unwrap()].contains("screen_capture::apply(&app);"), "re-applied on save");
        // The windows built later apply it themselves.
        for (file, code) in [("bubble_window.rs", include_str!("bubble_window.rs")), ("live_window.rs", include_str!("live_window.rs")), ("desktop.rs", include_str!("desktop.rs"))] {
            let code = code.split("#[cfg(test)]").next().unwrap();
            assert_eq!(code.matches("WebviewWindowBuilder::new(").count(), 1, "{file}");
            assert!(code.contains("crate::screen_capture::apply_to("), "{file}");
        }
        // lib.rs builds two (Settings, the dev capture panel), both before
        // the launch apply; the island comes from tauri.conf.json.
        let code = lib.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches("WebviewWindowBuilder::new(").count(), 2);
    }
}
