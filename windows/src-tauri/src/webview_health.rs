// Glim's windows keep their page through a WebView2 process failure and a sleep.
//
// Found 2026-10-26: after the PC woke, the Settings window was all white. Its
// page paints dark before any script runs (settings.html), so white means
// WebView2 had lost the page, and nothing in Glim noticed: there was no
// ProcessFailed handler and no reload after a wake. The window is created
// hidden at launch and lives as long as Glim, so one webview has to survive
// every sleep of the day.
//
// Found 2026-10-27: Settings showed white and frozen again, after a launch
// with no sleep, while the character window's render process exited; it did
// not come back in testing (each window has its own render process, and a
// killed one is reloaded fine). So Glim now checks Settings itself:
//
// - A render process that exits or hangs: that window's page is reloaded, and
//   a showing Settings is checked as well. The reload is posted to the event
//   loop, never run inside WebView2's callback: tauri runs `run_on_main_thread`
//   inline when called on the main thread, so the post goes through another
//   thread to really wait its turn.
// - The browser process exiting takes every window with it, and a reload
//   cannot bring it back: Glim starts itself again (relaunch.rs, at most
//   once per 10 minutes, else one Windows notification).
// - After a wake: the Settings page is reloaded, at once if it is showing,
//   else the next time it is shown.
// - Each time Settings is shown, its page must answer within 4 s; if it
//   doesn't, the page is reloaded, and if it still doesn't answer, the window
//   is built again. Settings showing up without Glim opening it is logged.
// - A watchdog logs when Glim's UI thread stops answering for over 5 s.
//
// Log lines carry a window label, a kind, a reason and counts, never a URL or
// page text.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub const SETTINGS: &str = "settings";

/// Pages reloaded after a process failure since launch.
static RELOADS: AtomicU32 = AtomicU32::new(0);
/// The Settings page missed a wake while hidden: reload it when shown.
static SETTINGS_STALE: AtomicBool = AtomicBool::new(false);
/// The last reload of each window, so a process that keeps hanging is not
/// reloaded in a loop.
static LAST_RELOAD: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
const RELOAD_AT_MOST_EVERY: Duration = Duration::from_secs(10);
/// Windows already watched: one handler each (the character window used to
/// get two, and every failure was logged twice).
static WATCHED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// How often the wake check looks at the clock, and how far behind it must
/// be to count as a wake: Glim's threads do not run while the PC sleeps
/// (S3 or Modern Standby), so the wall clock jumps.
const TICK: Duration = Duration::from_secs(15);
const WAKE_GAP: Duration = Duration::from_secs(120);

/// Milliseconds since launch (a clock that never jumps). 0 means "never".
fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// The Settings window's handle, read from other threads without asking the
/// UI thread (which may be the one that's stuck).
static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
/// When Glim last showed Settings, and when its page last answered.
static SHOWN_AT: AtomicU64 = AtomicU64::new(0);
static ANSWERED_AT: AtomicU64 = AtomicU64::new(0);
/// One Settings check at a time.
static CHECKING: AtomicBool = AtomicBool::new(false);
const ANSWER_WAIT: Duration = Duration::from_secs(4);
const RELOAD_WAIT: Duration = Duration::from_secs(8);

/// The UI thread's last answer to the watchdog.
static UI_PONG: AtomicU64 = AtomicU64::new(0);
const UI_PING_EVERY: Duration = Duration::from_secs(1);
const UI_STUCK_AFTER_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Response {
    Reload,
    /// Every window is gone: start Glim again (relaunch.rs).
    Relaunch,
    LogOnly,
}

/// What a WebView2 process failure (COREWEBVIEW2_PROCESS_FAILED_KIND) is
/// called in the log, and what Glim does about it.
fn respond(kind: i32) -> (&'static str, Response) {
    match kind {
        0 => ("browser process exited", Response::Relaunch),
        1 => ("render process exited", Response::Reload),
        2 => ("render process unresponsive", Response::Reload),
        3 => ("frame render process exited", Response::Reload),
        // WebView2 starts a new GPU process by itself.
        6 => ("gpu process exited", Response::LogOnly),
        _ => ("helper process exited", Response::LogOnly),
    }
}

/// COREWEBVIEW2_PROCESS_FAILED_REASON, as the log says it.
fn reason_name(reason: i32) -> &'static str {
    match reason {
        0 => "unexpected",
        1 => "unresponsive",
        2 => "terminated",
        3 => "crashed",
        4 => "launch failed",
        5 => "out of memory",
        6 => "profile deleted",
        _ => "unknown",
    }
}

fn failure_line(label: &str, what: &str, reason: Option<(i32, i32)>) -> String {
    match reason {
        Some((reason, code)) => {
            format!("webview: {label}: {what} (reason {}, exit code {code:#x})", reason_name(reason))
        }
        None => format!("webview: {label}: {what}"),
    }
}

fn reload_line(label: &str, reloaded: bool, total: u32) -> String {
    if reloaded {
        format!("webview: {label}: page reloaded ({total} reloads since launch)")
    } else {
        format!("webview: {label}: not reloaded (reloaded less than 10 s ago)")
    }
}

/// True when this window's page may be reloaded now (and notes that it is).
fn reload_due(label: &str) -> bool {
    let mut last = LAST_RELOAD.lock().unwrap_or_else(|e| e.into_inner());
    let map = last.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if map.get(label).is_some_and(|t| now.duration_since(*t) < RELOAD_AT_MOST_EVERY) {
        return false;
    }
    map.insert(label.to_string(), now);
    true
}

/// True the first time a window label is watched.
fn first_watch(label: &str) -> bool {
    let mut set = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    set.get_or_insert_with(HashSet::new).insert(label.to_string())
}

fn forget_watch(label: &str) {
    let mut set = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = set.as_mut() {
        set.remove(label);
    }
}

/// Runs `f` on the UI thread after whatever it is doing now (a WebView2
/// callback, say) has returned. See the top of this file.
fn post(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("glim-post".into()).spawn(move || {
        if let Err(err) = app.run_on_main_thread(f) {
            crate::log::line(format!("webview: could not post to the UI thread: {err}"));
        }
    });
    if let Err(err) = spawned {
        crate::log::line(format!("webview: could not post to the UI thread: {err}"));
    }
}

/// Reloads one window's page (on the UI thread, posted).
fn reload_window(app: &AppHandle, label: &str) {
    let Some(win) = app.get_webview_window(label) else { return };
    let reloaded = reload_due(label) && win.reload().is_ok();
    if reloaded {
        RELOADS.fetch_add(1, Ordering::Relaxed);
    }
    crate::log::line(reload_line(label, reloaded, RELOADS.load(Ordering::Relaxed)));
}

/// Watches one Glim window's WebView2 for process failures. Called for every
/// window Glim builds (a test checks); a window is watched once.
#[cfg(windows)]
pub fn watch(win: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_REASON,
        COREWEBVIEW2_WEB_ERROR_STATUS,
    };
    use webview2_com::{NavigationCompletedEventHandler, ProcessFailedEventHandler};
    use wv2_core::Interface;

    let label = win.label().to_string();
    if !first_watch(&label) {
        return;
    }
    if label == SETTINGS {
        if let Ok(hwnd) = win.hwnd() {
            SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        }
    }
    let app = win.app_handle().clone();
    let result = win.with_webview(move |webview| unsafe {
        let core = match webview.controller().CoreWebView2() {
            Ok(core) => core,
            Err(err) => {
                crate::log::line(format!("webview: {label}: cannot watch: {err}"));
                return;
            }
        };
        let name = label.clone();
        let failed = ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            args.ProcessFailedKind(&mut kind)?;
            let reason = args.cast::<ICoreWebView2ProcessFailedEventArgs2>().ok().and_then(|args2| {
                let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                let mut code = 0i32;
                args2.Reason(&mut reason).ok()?;
                args2.ExitCode(&mut code).ok()?;
                Some((reason.0, code))
            });
            let (what, response) = respond(kind.0);
            crate::log::line(failure_line(&name, what, reason));
            // Nothing here touches a webview: it all waits its turn.
            match response {
                Response::Reload => {
                    let (app2, name2) = (app.clone(), name.clone());
                    post(&app, move || reload_window(&app2, &name2));
                    if name != SETTINGS {
                        check_settings_if_showing(&app, "another window's page failed");
                    }
                }
                Response::Relaunch => {
                    let app2 = app.clone();
                    post(&app, move || crate::relaunch::on_browser_exited(&app2));
                }
                Response::LogOnly => {}
            }
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(err) = core.add_ProcessFailed(&failed, &mut token) {
            crate::log::line(format!("webview: {label}: cannot watch: {err}"));
        }
        // The Settings page says itself when it has drawn (settings_page);
        // a page that never arrives is logged here.
        if label == SETTINGS {
            let arrived = NavigationCompletedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut ok = wv2_core::BOOL::default();
                args.IsSuccess(&mut ok)?;
                if !ok.as_bool() {
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.WebErrorStatus(&mut status)?;
                    crate::log::line(format!("settings window: failed (navigation, web error {})", status.0));
                }
                Ok(())
            }));
            let mut token = 0i64;
            if let Err(err) = core.add_NavigationCompleted(&arrived, &mut token) {
                crate::log::line(format!("webview: {label}: cannot watch: {err}"));
            }
        }
    });
    if let Err(err) = result {
        crate::log::line(format!("webview: {}: cannot watch: {err}", win.label()));
    }
}

#[cfg(not(windows))]
pub fn watch(win: &WebviewWindow) {
    let _ = first_watch(win.label());
}

/// Every window built so far (at launch); windows already watched are skipped.
pub fn watch_all(app: &AppHandle) {
    for win in app.webview_windows().values() {
        watch(win);
    }
}

/// The Settings page's own report (src/settings/main.ts), once it has drawn
/// its sections or failed to.
pub fn settings_page(ok: bool) {
    if ok {
        ANSWERED_AT.store(now_ms(), Ordering::Relaxed);
    }
    crate::log::line(if ok { "settings window: loaded" } else { "settings window: failed (page script)" });
}

/// The Settings page heard `settings-opened` (or `settings-check`): it is alive.
pub fn settings_answered() {
    ANSWERED_AT.store(now_ms(), Ordering::Relaxed);
}

/// True when the wall clock moved on by much more than the thread slept:
/// the PC was asleep in between.
fn woke(slept: Duration, wall_before: u128, wall_after: u128) -> bool {
    wall_after.saturating_sub(wall_before) > slept.as_millis() + WAKE_GAP.as_millis()
}

fn wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Reloads the Settings page after each wake (see the top of this file).
pub fn start_wake_watch(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("glim-wake-watch".into()).spawn(move || {
        let mut before = wall_ms();
        loop {
            std::thread::sleep(TICK);
            let now = wall_ms();
            if woke(TICK, before, now) {
                after_wake(&app);
            }
            before = now;
        }
    });
    if let Err(err) = spawned {
        crate::log::line(format!("webview: no wake watch: {err}"));
    }
}

fn after_wake(app: &AppHandle) {
    if settings_visible() {
        let app2 = app.clone();
        post(app, move || {
            let Some(win) = app2.get_webview_window(SETTINGS) else { return };
            let reloaded = win.reload().is_ok();
            crate::log::line(format!("webview: woke; settings page reloaded: {reloaded}"));
        });
    } else {
        SETTINGS_STALE.store(true, Ordering::Relaxed);
        crate::log::line("webview: woke; settings page reloads when next shown");
    }
}

/// Called as the Settings window is shown: a page that missed a wake is
/// reloaded first.
pub fn before_settings_shown(win: &WebviewWindow) {
    // Before the show, so the watch never takes it for Settings showing by itself.
    SHOWN_AT.store(now_ms(), Ordering::Relaxed);
    if SETTINGS_STALE.swap(false, Ordering::Relaxed) {
        let reloaded = win.reload().is_ok();
        crate::log::line(format!("webview: settings page reloaded after a wake: {reloaded}"));
    }
}

/// Called right after Glim showed Settings (lib.rs `show_settings_window`).
pub fn settings_shown(app: &AppHandle, from: &str) {
    SHOWN_AT.store(now_ms(), Ordering::Relaxed);
    crate::log::line(format!("settings window: shown ({from})"));
    check_settings(app, "shown");
}

/// Is the Settings window on screen? Asks Windows, not the UI thread.
fn settings_visible() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        let raw = SETTINGS_HWND.load(Ordering::Relaxed);
        raw != 0 && IsWindowVisible(HWND(raw as *mut _)).as_bool()
    }
    #[cfg(not(windows))]
    false
}

fn check_settings_if_showing(app: &AppHandle, why: &'static str) {
    if settings_visible() {
        check_settings(app, why);
    }
}

/// What the Settings check does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Fine,
    Reload,
    Rebuild,
}

/// The page answered after `since` (both in `now_ms` time): fine; else the
/// first miss reloads and the second builds the window again.
fn next_step(answered_at: u64, since: u64, reloaded_already: bool) -> Step {
    match (answered_at >= since, reloaded_already) {
        (true, _) => Step::Fine,
        (false, false) => Step::Reload,
        (false, true) => Step::Rebuild,
    }
}

/// Makes sure the Settings page is alive: it must answer `settings-check`
/// (or `settings-opened`) within 4 s, else it is reloaded; if it still
/// doesn't answer within 8 s, the window is built again and shown.
fn check_settings(app: &AppHandle, why: &'static str) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("glim-settings-check".into()).spawn(move || {
        let since = now_ms();
        let _ = app.emit_to(SETTINGS, "settings-check", ());
        std::thread::sleep(ANSWER_WAIT);
        match next_step(ANSWERED_AT.load(Ordering::Relaxed), since, false) {
            Step::Fine => {}
            _ => {
                crate::log::line(format!("settings window: no answer 4 s after {why}; reloading"));
                let app2 = app.clone();
                post(&app, move || {
                    if let Some(win) = app2.get_webview_window(SETTINGS) {
                        let reloaded = win.reload().is_ok();
                        crate::log::line(format!("webview: settings: page reloaded: {reloaded}"));
                    }
                });
                let reloaded_at = now_ms();
                std::thread::sleep(RELOAD_WAIT);
                if next_step(ANSWERED_AT.load(Ordering::Relaxed), reloaded_at, true) == Step::Rebuild {
                    crate::log::line("settings window: still blank after the reload; building it again");
                    rebuild_settings(&app);
                } else {
                    crate::log::line("settings window: drawn after the reload");
                }
            }
        }
        CHECKING.store(false, Ordering::SeqCst);
    });
    if let Err(err) = spawned {
        CHECKING.store(false, Ordering::SeqCst);
        crate::log::line(format!("settings window: no check: {err}"));
    }
}

/// Throws the Settings window away and builds it again, then shows it if it
/// was showing. Runs on the check thread: tauri builds windows from any
/// thread, and building one on the UI thread from inside an event can hang.
fn rebuild_settings(app: &AppHandle) {
    let was_showing = settings_visible();
    if let Some(win) = app.get_webview_window(SETTINGS) {
        if let Err(err) = win.destroy() {
            crate::log::line(format!("settings window: could not close it: {err}"));
            return;
        }
    }
    // destroy() is carried out by the UI thread: wait for the label to free up.
    for _ in 0..30 {
        if app.get_webview_window(SETTINGS).is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if app.get_webview_window(SETTINGS).is_some() {
        crate::log::line("settings window: the old window did not close; not rebuilt");
        return;
    }
    SETTINGS_HWND.store(0, Ordering::Relaxed);
    forget_watch(SETTINGS);
    let Some(win) = crate::create_settings_window(app) else { return };
    crate::screen_capture::apply_to(app, &win);
    watch(&win);
    crate::log::line(format!("settings window: built again (showing: {was_showing})"));
    if was_showing {
        SHOWN_AT.store(now_ms(), Ordering::Relaxed);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// The UI-thread watchdog and the "Settings showed up by itself" check, once
/// a second on their own thread; neither waits on the UI thread.
pub fn start_ui_watch(app: &AppHandle) {
    let app = app.clone();
    UI_PONG.store(now_ms(), Ordering::Relaxed);
    let spawned = std::thread::Builder::new().name("glim-ui-watch".into()).spawn(move || {
        let mut stuck_since: Option<u64> = None;
        let mut was_visible = settings_visible();
        loop {
            // Answered by the UI thread when it gets to it.
            let _ = app.run_on_main_thread(|| UI_PONG.store(now_ms(), Ordering::Relaxed));
            std::thread::sleep(UI_PING_EVERY);
            let now = now_ms();
            match (ui_stuck(UI_PONG.load(Ordering::Relaxed), now), stuck_since) {
                (true, None) => {
                    stuck_since = Some(UI_PONG.load(Ordering::Relaxed));
                    crate::log::line(format!(
                        "ui thread: not responding for over 5 s (settings showing: {})",
                        settings_visible()
                    ));
                }
                (false, Some(since)) => {
                    crate::log::line(format!("ui thread: responding again after {} s", now.saturating_sub(since) / 1000));
                    stuck_since = None;
                }
                _ => {}
            }
            let visible = settings_visible();
            if visible && !was_visible && !shown_recently(SHOWN_AT.load(Ordering::Relaxed), now) {
                crate::log::line("settings window: became visible without Glim opening it");
                check_settings(&app, "showing up by itself");
            }
            was_visible = visible;
        }
    });
    if let Err(err) = spawned {
        crate::log::line(format!("ui thread: no watchdog: {err}"));
    }
}

fn ui_stuck(last_pong: u64, now: u64) -> bool {
    now.saturating_sub(last_pong) > UI_STUCK_AFTER_MS
}

/// Glim showed Settings within the last 3 s (the watch looks once a second).
fn shown_recently(shown_at: u64, now: u64) -> bool {
    shown_at != 0 && now.saturating_sub(shown_at) < 3_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_failures_reload_and_the_rest_are_logged() {
        assert_eq!(respond(0), ("browser process exited", Response::Relaunch));
        assert_eq!(respond(1).1, Response::Reload);
        assert_eq!(respond(2).1, Response::Reload);
        assert_eq!(respond(3).1, Response::Reload);
        assert_eq!(respond(6).1, Response::LogOnly);
        for other in [4, 5, 7, 8, 9, 10, 99, -1] {
            assert_eq!(respond(other).1, Response::LogOnly, "{other}");
        }
    }

    /// The kinds and reasons match WebView2's own numbers.
    #[cfg(windows)]
    #[test]
    fn kinds_and_reasons_are_webview2s_numbers() {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED.0, 0);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED.0, 1);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE.0, 2);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED.0, 3);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED.0, 6);
        for (reason, name) in [
            (COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED, "unexpected"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE, "unresponsive"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED, "terminated"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, "crashed"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED, "launch failed"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY, "out of memory"),
            (COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED, "profile deleted"),
        ] {
            assert_eq!(reason_name(reason.0), name);
        }
    }

    #[test]
    fn log_lines_carry_labels_kinds_reasons_and_counts_only() {
        assert_eq!(
            failure_line("character", "render process exited", Some((2, 1))),
            "webview: character: render process exited (reason terminated, exit code 0x1)"
        );
        assert_eq!(failure_line("island", "gpu process exited", None), "webview: island: gpu process exited");
        assert_eq!(reload_line("settings", true, 2), "webview: settings: page reloaded (2 reloads since launch)");
        assert_eq!(reload_line("settings", false, 2), "webview: settings: not reloaded (reloaded less than 10 s ago)");
    }

    #[test]
    fn a_window_is_not_reloaded_in_a_loop() {
        assert!(reload_due("test-loop"));
        assert!(!reload_due("test-loop"), "again within 10 s");
        assert!(reload_due("test-other"), "another window is its own");
    }

    #[test]
    fn a_window_is_watched_once_until_it_is_rebuilt() {
        assert!(first_watch("test-watch"));
        assert!(!first_watch("test-watch"), "a second handler would log every failure twice");
        forget_watch("test-watch");
        assert!(first_watch("test-watch"), "a rebuilt window is watched again");
    }

    #[test]
    fn a_wake_is_a_jump_of_the_wall_clock() {
        let tick = Duration::from_secs(15);
        assert!(!woke(tick, 0, 15_000));
        assert!(!woke(tick, 0, 60_000), "a busy minute is not a wake");
        assert!(woke(tick, 0, 15_000 + 120_001));
        assert!(woke(tick, 0, 510 * 60_000), "the 2026-10-26 wake (510 min)");
        assert!(!woke(tick, 50_000, 10_000), "a clock set back is not a wake");
    }

    #[test]
    fn a_silent_settings_page_is_reloaded_then_rebuilt() {
        assert_eq!(next_step(1_000, 900, false), Step::Fine, "answered after the show");
        assert_eq!(next_step(800, 900, false), Step::Reload, "last answer is older than the show");
        assert_eq!(next_step(0, 900, false), Step::Reload, "never answered");
        assert_eq!(next_step(800, 900, true), Step::Rebuild, "still silent after the reload");
        assert_eq!(next_step(950, 900, true), Step::Fine, "drawn after the reload");
    }

    #[test]
    fn the_ui_thread_counts_as_stuck_after_5_s() {
        assert!(!ui_stuck(1_000, 6_000));
        assert!(ui_stuck(1_000, 6_001));
        assert!(!ui_stuck(7_000, 6_000), "a pong newer than the check");
    }

    #[test]
    fn settings_showing_up_by_itself_is_told_apart_from_glim_opening_it() {
        assert!(shown_recently(10_000, 11_500));
        assert!(!shown_recently(10_000, 13_001));
        assert!(!shown_recently(0, 500), "never shown by Glim");
    }

    /// No webview is touched inside WebView2's own callback: the reload and
    /// the relaunch are posted (see the top of this file).
    #[test]
    fn nothing_reloads_inside_the_webview2_callback() {
        let code = include_str!("webview_health.rs").replace("\r\n", "\n");
        let watch = &code[code.find("pub fn watch(win: &WebviewWindow) {").unwrap()..];
        let handler = &watch[watch.find("ProcessFailedEventHandler::create").unwrap()..watch.find("add_ProcessFailed").unwrap()];
        assert!(!handler.contains(".Reload()"), "no direct WebView2 reload");
        let posted = "post(&app, move || reload_window(&app2, &name2));";
        assert!(handler.contains(posted));
        assert!(!handler.replace(posted, "").contains("reload_window("), "reload_window only through post");
        assert!(handler.contains("post(&app, move || crate::relaunch::on_browser_exited(&app2));"));
        let post_fn = &code[code.find("fn post(").unwrap()..];
        assert!(post_fn[..post_fn.find("\n}\n").unwrap()].contains("std::thread::Builder::new()"), "posted from another thread");
    }

    #[test]
    fn every_window_glim_builds_is_watched() {
        // CI checks the sources out with CRLF (core.autocrlf): compare as LF.
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let setup = &lib[lib.find(".setup(move |app|").unwrap()..];
        let watched = setup.find("webview_health::watch_all(&handle);").expect("watched at launch");
        for built in ["create_settings_window(&handle);", "create_capture_panel(&handle);", "desktop::setup(&handle);"] {
            assert!(setup.find(built).unwrap() < watched, "{built} comes before watch_all");
        }
        assert!(setup.contains("webview_health::start_wake_watch(&handle);"));
        assert!(setup.contains("webview_health::start_ui_watch(&handle);"));
        // The island comes from tauri.conf.json and exists before setup.
        for (file, code) in [("bubble_window.rs", include_str!("bubble_window.rs")), ("desktop.rs", include_str!("desktop.rs"))] {
            let code = code.split("#[cfg(test)]").next().unwrap();
            assert_eq!(code.matches("WebviewWindowBuilder::new(").count(), 1, "{file}");
            assert!(code.contains("crate::webview_health::watch(&win);"), "{file}");
        }
        let show = &lib[lib.find("pub fn show_settings_window").unwrap()..];
        let show = &show[..show.find("\n}\n").unwrap()];
        assert!(show.find("webview_health::before_settings_shown(&win);").unwrap() < show.find("win.show()").unwrap());
        assert!(show.find("win.show()").unwrap() < show.find("webview_health::settings_shown(app, from);").unwrap());
    }
}
