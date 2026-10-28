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
// The log says on/off and counts only.

use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewWindow};

/// The test-only override: "1" lets every Glim window be captured.
pub const OVERRIDE_VAR: &str = "GLIM_CAPTURABLE";

/// Whether Glim's windows are kept out of capture: the setting, unless the
/// override is set to 1.
pub fn wanted(setting: bool, override_var: Option<&str>) -> bool {
    setting && override_var.map(str::trim) != Some("1")
}

/// Why, in the log: "on", "off (setting)", "off (GLIM_CAPTURABLE)".
fn reason(setting: bool, override_var: Option<&str>) -> &'static str {
    if wanted(setting, override_var) {
        "on"
    } else if setting {
        "off (GLIM_CAPTURABLE)"
    } else {
        "off (setting)"
    }
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
    wanted(setting(app), override_var().as_deref())
}

/// Every Glim window as the setting says: at launch, once its windows
/// exist, and after each settings save.
pub fn apply(app: &AppHandle) {
    let (setting, var) = (setting(app), override_var());
    let want = wanted(setting, var.as_deref());
    let windows = app.webview_windows();
    let refused = windows.values().filter(|win| !crate::platform::set_capture_excluded(win, want)).count();
    note(reason(setting, var.as_deref()), windows.len(), want && refused > 0);
}

/// Right after Glim shows a window. A hidden window is never touched
/// (platform::set_capture_excluded: on hidden Settings it left a white copy
/// on screen, 2026-10-27), so the window gets the setting here.
pub fn after_show(app: &AppHandle, win: &WebviewWindow) {
    apply_to(app, win);
}

/// A window built after launch (the mini lantern, the desktop lantern).
pub fn apply_to(app: &AppHandle, win: &WebviewWindow) {
    let want = wanted(setting(app), override_var().as_deref());
    if !crate::platform::set_capture_excluded(win, want) && want {
        note("on", 1, true);
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
        assert!(wanted(true, None));
        assert!(!wanted(false, None));
        assert!(!wanted(true, Some("1")));
        assert!(!wanted(true, Some(" 1 ")));
        assert!(!wanted(false, Some("1")));
        // Anything but 1 leaves the setting in charge.
        for other in ["", "0", "true", "yes", "2"] {
            assert!(wanted(true, Some(other)), "{other:?}");
        }
    }

    #[test]
    fn the_log_says_why_and_never_more() {
        assert_eq!(reason(true, None), "on");
        assert_eq!(reason(true, Some("1")), "off (GLIM_CAPTURABLE)");
        assert_eq!(reason(false, None), "off (setting)");
        assert_eq!(reason(false, Some("1")), "off (setting)");
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
        for (file, code) in [("bubble_window.rs", include_str!("bubble_window.rs")), ("desktop.rs", include_str!("desktop.rs"))] {
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
