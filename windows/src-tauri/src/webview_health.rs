// Glim's windows keep their page through a WebView2 process failure and a sleep.
//
// Found 2026-10-26: after the PC woke, the Settings window was all white. Its
// page paints dark before any script runs (settings.html), so white means
// WebView2 had lost the page, and nothing in Glim noticed: there was no
// ProcessFailed handler and no reload after a wake. The window is created
// hidden at launch and lives as long as Glim, so one webview has to survive
// every sleep of the day.
//
// - A render process that exits or hangs: that window's page is reloaded.
// - The browser process exiting takes every window with it, and a reload
//   cannot bring it back: Glim starts itself again (relaunch.rs, at most
//   once per 10 minutes, else one Windows notification).
// - After a wake: the Settings page is reloaded, at once if it is showing,
//   else the next time it is shown.
//
// Log lines carry a window label, a kind and counts, never a URL or page text.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, WebviewWindow};

pub const SETTINGS: &str = "settings";

/// Pages reloaded after a process failure since launch.
static RELOADS: AtomicU32 = AtomicU32::new(0);
/// The Settings page missed a wake while hidden: reload it when shown.
static SETTINGS_STALE: AtomicBool = AtomicBool::new(false);
/// The last reload of each window, so a process that keeps hanging is not
/// reloaded in a loop.
static LAST_RELOAD: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
const RELOAD_AT_MOST_EVERY: Duration = Duration::from_secs(10);

/// How often the wake check looks at the clock, and how far behind it must
/// be to count as a wake: Glim's threads do not run while the PC sleeps
/// (S3 or Modern Standby), so the wall clock jumps.
const TICK: Duration = Duration::from_secs(15);
const WAKE_GAP: Duration = Duration::from_secs(120);

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

fn failure_line(label: &str, what: &str, reloaded: bool, total: u32) -> String {
    match (reloaded, what) {
        (true, _) => format!("webview: {label}: {what}; page reloaded ({total} reloads since launch)"),
        (false, _) => format!("webview: {label}: {what}"),
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

/// Watches one Glim window's WebView2 for process failures. Called for every
/// window Glim builds (a test checks).
#[cfg(windows)]
pub fn watch(win: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_WEB_ERROR_STATUS,
    };
    use webview2_com::{NavigationCompletedEventHandler, ProcessFailedEventHandler};

    let label = win.label().to_string();
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
        let failed = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
            let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            args.ProcessFailedKind(&mut kind)?;
            let (what, response) = respond(kind.0);
            let reloaded = response == Response::Reload && reload_due(&name) && sender.Reload().is_ok();
            if reloaded {
                RELOADS.fetch_add(1, Ordering::Relaxed);
            }
            crate::log::line(failure_line(&name, what, reloaded, RELOADS.load(Ordering::Relaxed)));
            if response == Response::Relaunch {
                crate::relaunch::on_browser_exited(&app);
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
pub fn watch(_win: &WebviewWindow) {}

/// Every window built so far (at launch).
pub fn watch_all(app: &AppHandle) {
    for win in app.webview_windows().values() {
        watch(win);
    }
}

/// The Settings page's own report (src/settings/main.ts), once it has drawn
/// its sections or failed to.
pub fn settings_page(ok: bool) {
    crate::log::line(if ok { "settings window: loaded" } else { "settings window: failed (page script)" });
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
    let Some(win) = app.get_webview_window(SETTINGS) else { return };
    if win.is_visible().unwrap_or(false) {
        let reloaded = win.reload().is_ok();
        crate::log::line(format!("webview: woke; settings page reloaded: {reloaded}"));
    } else {
        SETTINGS_STALE.store(true, Ordering::Relaxed);
        crate::log::line("webview: woke; settings page reloads when next shown");
    }
}

/// Called as the Settings window is shown: a page that missed a wake is
/// reloaded first.
pub fn before_settings_shown(win: &WebviewWindow) {
    if SETTINGS_STALE.swap(false, Ordering::Relaxed) {
        let reloaded = win.reload().is_ok();
        crate::log::line(format!("webview: settings page reloaded after a wake: {reloaded}"));
    }
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

    /// The kinds match WebView2's own numbers, so `respond` reads them right.
    #[cfg(windows)]
    #[test]
    fn kinds_are_webview2s_numbers() {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED.0, 0);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED.0, 1);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE.0, 2);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED.0, 3);
        assert_eq!(COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED.0, 6);
    }

    #[test]
    fn log_lines_carry_labels_kinds_and_counts_only() {
        assert_eq!(
            failure_line("settings", "render process exited", true, 2),
            "webview: settings: render process exited; page reloaded (2 reloads since launch)"
        );
        assert_eq!(failure_line("island", "gpu process exited", false, 0), "webview: island: gpu process exited");
        assert_eq!(failure_line("island", "browser process exited", false, 0), "webview: island: browser process exited");
    }

    #[test]
    fn a_window_is_not_reloaded_in_a_loop() {
        assert!(reload_due("test-loop"));
        assert!(!reload_due("test-loop"), "again within 10 s");
        assert!(reload_due("test-other"), "another window is its own");
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
    fn every_window_glim_builds_is_watched() {
        // CI checks the sources out with CRLF (core.autocrlf): compare as LF.
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let setup = &lib[lib.find(".setup(move |app|").unwrap()..];
        let watched = setup.find("webview_health::watch_all(&handle);").expect("watched at launch");
        for built in ["create_settings_window(&handle);", "create_capture_panel(&handle);", "desktop::setup(&handle);"] {
            assert!(setup.find(built).unwrap() < watched, "{built} comes before watch_all");
        }
        assert!(setup.contains("webview_health::start_wake_watch(&handle);"));
        // The island comes from tauri.conf.json and exists before setup.
        for (file, code) in [("bubble_window.rs", include_str!("bubble_window.rs")), ("desktop.rs", include_str!("desktop.rs"))] {
            let code = code.split("#[cfg(test)]").next().unwrap();
            assert_eq!(code.matches("WebviewWindowBuilder::new(").count(), 1, "{file}");
            assert!(code.contains("crate::webview_health::watch(&win);"), "{file}");
        }
        let show = &lib[lib.find("pub fn show_settings_window").unwrap()..];
        let show = &show[..show.find("\n}\n").unwrap()];
        assert!(show.find("webview_health::before_settings_shown(&win);").unwrap() < show.find("win.show()").unwrap());
    }
}
