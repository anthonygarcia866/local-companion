// `glim.exe --selftest`: starts Glim as usual, checks it with Win32 (not
// Tauri's flags), prints PASS/FAIL per check and exits with a code:
// 0 = every check passed, 1 = one failed, 2 = another Glim is running (stop
// it first: a second launch would be handed to it), 3 = timed out.
//
// The result goes to stdout (when redirected) and to
// %LOCALAPPDATA%\Glim\selftest.txt; glim.log gets counts only. A self-test
// launch skips what writes outside Glim or reaches other apps: the autostart
// entry, the hook exe copy, the hooks pipe (a live approval must not land in
// a Glim that is about to exit), the calendar's Outlook read.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

pub const FLAG: &str = "--selftest";

/// How long the pages get to report that they loaded.
const PAGE_WAIT: Duration = Duration::from_secs(10);
/// The whole run, whatever happens.
const HARD_LIMIT: Duration = Duration::from_secs(60);

static ON: AtomicBool = AtomicBool::new(false);
static LOADED: Mutex<Vec<(String, Duration)>> = Mutex::new(Vec::new());
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);
/// Windows that were visible when Glim set their capture affinity: they may
/// keep it once hidden again (the island in front of a fullscreen app).
static APPLIED_VISIBLE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The pages that report "loaded", and what they call to say so: the island
/// `boot`, Settings `settings_page(true)` once its sections are drawn, the
/// character `desktop_mochi_info`.
const PAGES: [&str; 3] = ["island", "settings", "character"];

pub fn requested(argv: &[String]) -> bool {
    argv.iter().any(|a| a == FLAG)
}

/// This launch is a self-test. Ends it after `HARD_LIMIT` whatever happens.
pub fn begin() {
    ON.store(true, Ordering::Relaxed);
    *STARTED.lock().unwrap() = Some(Instant::now());
    std::thread::spawn(|| {
        std::thread::sleep(HARD_LIMIT);
        finish(&[Check::fail("finished", "timed out after 60 s")], 3);
    });
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// A page said it loaded (only noted during a self-test).
pub fn page_loaded(label: &str) {
    if !on() {
        return;
    }
    let since = STARTED.lock().unwrap().map(|s| s.elapsed()).unwrap_or_default();
    let mut loaded = LOADED.lock().unwrap();
    if !loaded.iter().any(|(l, _)| l == label) {
        loaded.push((label.to_string(), since));
    }
}

/// screen_capture applied the affinity to `label` while it was visible.
pub fn applied_while_visible(label: &str) {
    let mut seen = APPLIED_VISIBLE.lock().unwrap();
    if !seen.iter().any(|l| l == label) {
        seen.push(label.to_string());
    }
}

/// Another Glim holds the single instance: this launch would be handed to it.
pub fn refuse_running() -> ! {
    finish(&[Check::fail("no other Glim running", "another glim.exe is running: stop it first")], 2)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    name: &'static str,
    pass: bool,
    detail: String,
}

impl Check {
    fn new(name: &'static str, pass: bool, detail: impl Into<String>) -> Self {
        Check { name, pass, detail: detail.into() }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Check::new(name, false, detail)
    }
}

/// Once setup is done: waits for the pages, then checks.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let present: Vec<String> = PAGES.iter().filter(|l| app.get_webview_window(l).is_some()).map(|l| l.to_string()).collect();
        let deadline = Instant::now() + PAGE_WAIT;
        while Instant::now() < deadline && !all_loaded(&present) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Let what follows a load (the ui watch, a late show) settle.
        std::thread::sleep(Duration::from_secs(1));
        let checks = run_checks(&app, &present);
        let code = if checks.iter().all(|c| c.pass) { 0 } else { 1 };
        finish(&checks, code);
    });
}

fn all_loaded(present: &[String]) -> bool {
    let loaded = LOADED.lock().unwrap();
    present.iter().all(|p| loaded.iter().any(|(l, _)| l == p))
}

fn run_checks(app: &AppHandle, present: &[String]) -> Vec<Check> {
    let windows: Vec<(String, Option<(bool, u32)>)> = {
        let mut w: Vec<_> = app.webview_windows().into_iter().map(|(label, win)| (label, crate::platform::window_state(&win))).collect();
        w.sort_by(|a, b| a.0.cmp(&b.0));
        w
    };
    let loaded = LOADED.lock().unwrap().clone();
    let applied = APPLIED_VISIBLE.lock().unwrap().clone();
    // Glim's own rule: no island in front of a fullscreen app, or when the
    // user picked Hidden.
    let island_shown = crate::presence::current(app) != crate::presence::Presence::Hidden;
    let shortcuts = crate::shortcuts::status(app);
    let statuses: Vec<_> = shortcuts.actions.iter().map(|a| (a.id, a.status)).collect();
    let found = crate::hooks::config_dirs().len();
    let expected = expected_hook_dirs(&crate::platform::home_dir(), std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from));
    vec![
        windows_check(&windows, island_shown),
        pages_check(present, &loaded),
        hotkeys_check(&statuses, shortcuts.blocked.is_some()),
        hooks_check(found, expected),
        settings_check(crate::settings::file_check()),
        capture_check(&windows, &applied, crate::screen_capture::wanted_now(app)),
    ]
}

/// Whether Glim means `label` to be on screen at launch: the island unless
/// presence hides it, and the character on Windows (always shown, parked off
/// screen while he is in the island). None = not checked (the dev-only
/// capture panel).
fn expected_visible(label: &str, island_shown: bool) -> Option<bool> {
    match label {
        "island" => Some(island_shown),
        "character" => Some(cfg!(windows)),
        "capture" => None,
        _ => Some(false),
    }
}

fn windows_check(windows: &[(String, Option<(bool, u32)>)], island_shown: bool) -> Check {
    let mut wrong = Vec::new();
    let (mut hidden, mut shown) = (0, 0);
    for (label, state) in windows {
        let Some(want) = expected_visible(label, island_shown) else { continue };
        match state {
            None => wrong.push(format!("{label} (no window handle)")),
            Some((visible, _)) if *visible != want => {
                wrong.push(format!("{label} {}", if *visible { "visible, should be hidden" } else { "hidden, should be visible" }))
            }
            Some(_) if want => shown += 1,
            Some(_) => hidden += 1,
        }
    }
    if wrong.is_empty() {
        Check::new("windows hidden/shown", true, format!("{hidden} hidden, {shown} shown, as meant"))
    } else {
        Check::fail("windows hidden/shown", wrong.join("; "))
    }
}

fn pages_check(present: &[String], loaded: &[(String, Duration)]) -> Check {
    let missing: Vec<&str> = present.iter().filter(|p| !loaded.iter().any(|(l, _)| l == *p)).map(String::as_str).collect();
    let slowest = loaded.iter().map(|(_, d)| *d).max().unwrap_or_default();
    if present.is_empty() {
        Check::fail("pages loaded", "no page windows")
    } else if missing.is_empty() {
        Check::new("pages loaded", true, format!("{} of {} within {:.1} s", present.len(), present.len(), slowest.as_secs_f32()))
    } else {
        Check::fail("pages loaded", format!("not loaded within 10 s: {}", missing.join(", ")))
    }
}

fn hotkeys_check(statuses: &[(&'static str, crate::shortcuts::Status)], blocked: bool) -> Check {
    use crate::shortcuts::Status;
    if blocked {
        return Check::fail("hotkeys", "global shortcuts are blocked on this system");
    }
    let active = statuses.iter().filter(|(_, s)| *s == Status::Active).count();
    let off = statuses.iter().filter(|(_, s)| matches!(s, Status::Off | Status::NotPorted)).count();
    let failed: Vec<String> = statuses
        .iter()
        .filter(|(_, s)| !matches!(s, Status::Active | Status::Off | Status::NotPorted))
        .map(|(id, s)| format!("{id} ({s:?})"))
        .collect();
    if active == 0 {
        Check::fail("hotkeys", "none registered")
    } else if failed.is_empty() {
        Check::new("hotkeys", true, format!("{active} registered, {off} off"))
    } else {
        Check::fail("hotkeys", format!("{active} registered; not registered: {}", failed.join(", ")))
    }
}

/// The Claude config folders the hooks rule should find, worked out
/// separately from it: ~/.claude and ~/.claude<digits> (exactly that name:
/// not ".claude2 ", not ".claude-mem"), and CLAUDE_CONFIG_DIR, each holding
/// the projects/ folder Claude Code keeps, each once.
fn expected_hook_dirs(home: &Path, env: Option<PathBuf>) -> usize {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(home).into_iter().flatten().filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        let digits = name.get(7..).unwrap_or("");
        let named = name.get(..7).is_some_and(|h| h.eq_ignore_ascii_case(".claude")) && digits.bytes().all(|b| b.is_ascii_digit());
        if named && e.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push(e.path());
        }
    }
    if let Some(dir) = env.filter(|d| !d.as_os_str().is_empty()) {
        dirs.push(if dir.is_absolute() { dir } else { home.join(dir) });
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for d in dirs.into_iter().filter(|d| d.join("projects").is_dir()) {
        let key = std::fs::canonicalize(&d).unwrap_or(d);
        if !seen.contains(&key) {
            seen.push(key);
        }
    }
    seen.len()
}

fn hooks_check(found: usize, expected: usize) -> Check {
    Check::new("hooks folders", found == expected, format!("found {found}, expected {expected}"))
}

fn settings_check(file: Result<bool, ()>) -> Check {
    match file {
        Ok(true) => Check::new("settings.json", true, "parses"),
        Ok(false) => Check::new("settings.json", true, "not there yet (defaults)"),
        Err(()) => Check::fail("settings.json", "can't be read or doesn't parse"),
    }
}

/// A capture affinity is only ever set on a visible window: a hidden window
/// that has one and was never visible when Glim set it is the 2026-10-27
/// white ghost. Visible windows have the one the setting asks for.
fn capture_check(windows: &[(String, Option<(bool, u32)>)], applied_visible: &[String], want: bool) -> Check {
    let excluded = crate::platform::CAPTURE_EXCLUDED;
    let mut wrong = Vec::new();
    let (mut set, mut untouched) = (0, 0);
    for (label, state) in windows {
        let Some((visible, affinity)) = state else { continue };
        if !visible {
            if *affinity == 0 || applied_visible.contains(label) {
                untouched += 1;
            } else {
                wrong.push(format!("{label} hidden with affinity {affinity:#x}"));
            }
        } else if *affinity == if want { excluded } else { 0 } {
            set += 1;
        } else {
            wrong.push(format!("{label} visible with affinity {affinity:#x}"));
        }
    }
    let mode = if want { "on" } else { "off" };
    if wrong.is_empty() {
        Check::new("capture exclusion", true, format!("{mode}: {set} visible as set, {untouched} hidden ok"))
    } else {
        Check::fail("capture exclusion", format!("{mode}: {}", wrong.join("; ")))
    }
}

fn report(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        out.push_str(&format!("{}  {}: {}\n", if c.pass { "PASS" } else { "FAIL" }, c.name, c.detail));
    }
    let passed = checks.iter().filter(|c| c.pass).count();
    let all = passed == checks.len();
    out.push_str(&format!("SELFTEST {} ({passed}/{})\n", if all { "PASS" } else { "FAIL" }, checks.len()));
    out
}

/// Where the result goes besides stdout.
pub fn result_path() -> PathBuf {
    crate::platform::local_dir().join("selftest.txt")
}

fn finish(checks: &[Check], code: i32) -> ! {
    let text = report(checks);
    print!("{text}");
    let path = result_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &text);
    let passed = checks.iter().filter(|c| c.pass).count();
    let failed: Vec<&str> = checks.iter().filter(|c| !c.pass).map(|c| c.name).collect();
    crate::log::line(format!(
        "selftest: {passed} of {} passed{} (exit {code})",
        checks.len(),
        if failed.is_empty() { String::new() } else { format!("; failed: {}", failed.join(", ")) }
    ));
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::Status;

    fn win(label: &str, visible: bool, affinity: u32) -> (String, Option<(bool, u32)>) {
        (label.to_string(), Some((visible, affinity)))
    }

    #[test]
    fn only_the_flag_starts_a_self_test() {
        assert!(requested(&["glim.exe".into(), "--selftest".into()]));
        assert!(!requested(&["glim.exe".into(), "--open-settings".into()]));
        assert!(!requested(&["glim.exe".into(), "selftest".into()]));
    }

    #[test]
    fn a_visible_settings_window_fails_and_a_hidden_island_fails() {
        let parked = cfg!(windows);
        let ok = [win("island", true, 0), win("settings", false, 0), win("character", parked, 0), win("capture", true, 0)];
        assert!(windows_check(&ok, true).pass, "{:?}", windows_check(&ok, true));
        let shown = [win("island", true, 0), win("settings", true, 0)];
        assert_eq!(windows_check(&shown, true).detail, "settings visible, should be hidden");
        let island_hidden = [win("island", false, 0), win("settings", false, 0)];
        assert!(!windows_check(&island_hidden, true).pass);
        // Presence hid it (a fullscreen app in front, seen 2026-10-28): meant.
        assert!(windows_check(&island_hidden, false).pass);
        assert!(!windows_check(&[win("island", true, 0)], false).pass);
        assert!(!windows_check(&[("settings".into(), None)], true).pass, "no handle is a failure");
        // A window Glim builds later (the bubble) must be hidden too.
        assert!(!windows_check(&[win("bubble", true, 0)], true).pass);
    }

    #[test]
    fn a_page_that_never_reported_fails() {
        let present = vec!["island".to_string(), "settings".to_string()];
        let one = vec![("island".to_string(), Duration::from_millis(900))];
        let c = pages_check(&present, &one);
        assert!(!c.pass);
        assert_eq!(c.detail, "not loaded within 10 s: settings");
        let both = vec![("island".to_string(), Duration::from_millis(900)), ("settings".to_string(), Duration::from_millis(2400))];
        assert_eq!(pages_check(&present, &both).detail, "2 of 2 within 2.4 s");
        assert!(!pages_check(&[], &both).pass);
    }

    #[test]
    fn a_hotkey_another_app_holds_fails() {
        let ok = [("a", Status::Active), ("b", Status::Off), ("c", Status::Active)];
        assert_eq!(hotkeys_check(&ok, false).detail, "2 registered, 1 off");
        let taken = [("a", Status::Active), ("pause", Status::InUse)];
        let c = hotkeys_check(&taken, false);
        assert!(!c.pass);
        assert_eq!(c.detail, "1 registered; not registered: pause (InUse)");
        assert!(!hotkeys_check(&ok, true).pass, "blocked");
        assert!(!hotkeys_check(&[("a", Status::Off)], false).pass, "none registered");
    }

    #[test]
    fn hidden_windows_keep_no_affinity_and_visible_ones_follow_the_setting() {
        let x = crate::platform::CAPTURE_EXCLUDED;
        let island = vec!["island".to_string()];
        let on = [win("island", true, x), win("settings", false, 0)];
        assert!(capture_check(&on, &island, true).pass);
        // The 2026-10-27 ghost: affinity on the hidden Settings window, which
        // was never visible when it was set.
        let ghost = [win("island", true, x), win("settings", false, 0x11)];
        let c = capture_check(&ghost, &island, true);
        assert!(!c.pass);
        assert_eq!(c.detail, "on: settings hidden with affinity 0x11");
        // Set while visible, hidden since (presence): fine.
        let hidden_later = [win("island", false, 0x11), win("settings", false, 0)];
        assert!(capture_check(&hidden_later, &island, true).pass);
        assert!(!capture_check(&hidden_later, &[], true).pass);
        let off = [win("island", true, 0), win("settings", false, 0)];
        assert_eq!(capture_check(&off, &island, false).detail, "off: 1 visible as set, 1 hidden ok");
        if x != 0 {
            assert!(!capture_check(&off, &island, true).pass, "visible but not excluded");
        }
    }

    #[test]
    fn the_settings_check_passes_a_missing_file_and_fails_a_broken_one() {
        assert!(settings_check(Ok(true)).pass);
        assert!(settings_check(Ok(false)).pass);
        assert!(!settings_check(Err(())).pass);
    }

    #[test]
    fn the_expected_hook_folders_are_named_ones_with_projects() {
        let home = std::env::temp_dir().join(format!("glim-selftest-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in [".claude/projects", ".claude2/projects", ".claude-mem/projects", ".claude3", "other/projects"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        assert_eq!(expected_hook_dirs(&home, None), 2);
        // CLAUDE_CONFIG_DIR adds one; naming one already counted adds none.
        assert_eq!(expected_hook_dirs(&home, Some(home.join("other"))), 3);
        assert_eq!(expected_hook_dirs(&home, Some(home.join(".claude2"))), 2);
        assert!(hooks_check(2, 2).pass);
        assert!(!hooks_check(1, 2).pass);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_report_ends_with_the_verdict() {
        let r = report(&[Check::new("a", true, "fine"), Check::fail("b", "broken")]);
        assert_eq!(r, "PASS  a: fine\nFAIL  b: broken\nSELFTEST FAIL (1/2)\n");
        assert!(report(&[Check::new("a", true, "fine")]).ends_with("SELFTEST PASS (1/1)\n"));
    }

    /// The launch hooks: the flag is read before the single-instance plugin,
    /// a running Glim ignores it, the risky steps are skipped, and each page
    /// reports through the command it already calls.
    #[test]
    fn the_launch_is_wired_for_the_self_test() {
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let run = &lib[lib.find("pub fn run()").unwrap()..];
        let begin = run.find("selftest::begin();").expect("begin");
        assert!(begin < run.find("tauri_plugin_single_instance").unwrap());
        assert!(run[begin..].contains("if platform::other_glim_running() {"));
        assert!(run.contains("argv.iter().any(|a| a == selftest::FLAG)"), "a running Glim ignores it");
        let setup = &run[run.find(".setup(move |app|").unwrap()..];
        for skipped in ["autostart::sync(", "hooks::ensure_hook_exe(", "pipe::start(", "calendar::worker::start("] {
            let at = setup.find(skipped).unwrap_or_else(|| panic!("{skipped}"));
            assert!(setup[..at].trim_end().ends_with("if !selftest {"), "{skipped} is skipped in a self-test");
        }
        assert!(setup.contains("selftest::start(handle.clone());"));
        let boot = &lib[lib.find("fn boot(").unwrap()..];
        assert!(boot[..boot.find("\n}\n").unwrap()].contains("selftest::page_loaded(win.label())"));
        let page = &lib[lib.find("fn settings_page(ok: bool)").unwrap()..];
        assert!(page[..page.find("\n}\n").unwrap()].contains("selftest::page_loaded(\"settings\")"));
        let desktop = include_str!("desktop.rs").replace("\r\n", "\n");
        let info = &desktop[desktop.find("fn desktop_mochi_info").unwrap()..];
        assert!(info[..info.find("\n}\n").unwrap()].contains("crate::selftest::page_loaded(LABEL)"));
    }
}
