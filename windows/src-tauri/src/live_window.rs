// The live revision's preview (owner, 2026-11-03): one small webview just
// above the paragraph being written (writing::live::preview_rect), else
// under the island's top (island_rect). It shows the word-level diff, three
// lines at most until "…" is clicked, and the accept key's hint.
//
// Focus: never takes it, like the mini lantern's window (bubble_window.rs):
// created once, visible, at a parked spot far off screen, then only ever
// moved, never shown or hidden; a tool window, topmost, WS_EX_NOACTIVATE
// through tao's FOCUSABLE flag, with BROWSER_ARGS. It takes no keys: the
// accept and dismiss keys are global shortcuts handled in Rust.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::writing::bubble::{Monitor, Rect};
use crate::writing::live::{self, LiveEvent};

pub const LABEL: &str = "live";
/// What the page hears: the revision to draw ("show"), that it went away
/// ("hide"), or a note after the accept key ("note", with its code).
pub const EVENT: &str = "live-preview";
/// Far off every monitor: where the window waits while nothing is shown.
const PARKED: (i32, i32) = (-32000, -32000);

static CREATING: AtomicBool = AtomicBool::new(false);
/// The revision that came while the window was still being made (the first
/// of a run): shown once its page says it listens (page_ready). Stage 3 on
/// CI, 2026-11-04: without it, the first revision after launch was dropped.
static PENDING: Mutex<Option<(LiveEvent, Option<Rect>)>> = Mutex::new(None);
/// "…" was clicked: the preview is taller until the next revision.
static EXPANDED: AtomicBool = AtomicBool::new(false);

/// Never silent (owner, 2026-11-07): the page says it drew each revision
/// (live_page_ready, after every "show"). No answer 3 s after a show reloads
/// the page; still none 6 s after that, the window is built again and the
/// revision shown in it (as webview_health does for Settings, #66).
const ANSWER_WAIT: Duration = Duration::from_secs(3);
const RELOAD_WAIT: Duration = Duration::from_secs(6);
/// When (ms since launch, 0 = never) the last revision was sent to the page,
/// and when the page last answered.
static SHOWN_AT: AtomicU64 = AtomicU64::new(0);
static ANSWERED_AT: AtomicU64 = AtomicU64::new(0);
static CHECKING: AtomicBool = AtomicBool::new(false);
/// The last revision shown, to show again in a rebuilt window.
static LAST_SHOW: Mutex<Option<(LiveEvent, Option<Rect>)>> = Mutex::new(None);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// What the check does after a wait: fine (answered since `since`), else
/// reload first, then rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Fine,
    Reload,
    Rebuild,
}

fn next_step(answered_at: u64, since: u64, reloaded_already: bool) -> Step {
    match (answered_at >= since, reloaded_already) {
        (true, _) => Step::Fine,
        (false, false) => Step::Reload,
        (false, true) => Step::Rebuild,
    }
}

pub fn set_expanded(on: bool) {
    EXPANDED.store(on, Ordering::SeqCst);
}

pub fn expanded() -> bool {
    EXPANDED.load(Ordering::SeqCst)
}

/// Where the preview goes now: above the paragraph, else under the island.
pub fn place(app: &AppHandle, paragraph: Option<Rect>, line: Option<Rect>, field: Option<Rect>, monitors: &[Monitor]) -> Option<Rect> {
    let open = expanded();
    live::preview_rect(paragraph, line, field, monitors, open).or_else(|| {
        let island = app.get_webview_window(crate::island::WINDOW_LABEL)?;
        let (pos, size) = (island.outer_position().ok()?, island.outer_size().ok()?);
        let r = Rect::new(pos.x, pos.y, pos.x + size.width as i32, pos.y + size.height as i32);
        live::island_rect(r, monitors, open)
    })
}

/// Shows `event` at `at` (a "show"), or parks the window (a "hide").
pub fn apply(app: &AppHandle, event: &LiveEvent, at: Option<Rect>) {
    if event.phase == "show" {
        set_expanded(false);
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        match event.phase {
            "show" => {
                *pending = Some((event.clone(), at));
                create(app);
            }
            "hide" => *pending = None,
            _ => {}
        }
        return;
    };
    match (event.phase, at) {
        ("hide", _) | (_, None) => {
            let _ = win.set_position(PhysicalPosition::new(PARKED.0, PARKED.1));
        }
        (_, Some(r)) => move_to(&win, r),
    }
    if event.phase == "show" {
        *LAST_SHOW.lock().unwrap_or_else(|e| e.into_inner()) = Some((event.clone(), at));
        SHOWN_AT.store(now_ms(), Ordering::SeqCst);
    }
    let _ = app.emit_to(LABEL, EVENT, event);
    if event.phase == "show" {
        check(app);
    }
}

/// Makes sure the page drew the revision just sent (see ANSWER_WAIT).
fn check(app: &AppHandle) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("glim-live-check".into()).spawn(move || {
        let since = SHOWN_AT.load(Ordering::SeqCst);
        std::thread::sleep(ANSWER_WAIT);
        if next_step(ANSWERED_AT.load(Ordering::SeqCst), since, false) != Step::Fine {
            crate::log::line("live preview: no answer 3 s after a revision; reloading");
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(win) = app2.get_webview_window(LABEL) {
                    let reloaded = win.reload().is_ok();
                    crate::log::line(format!("live preview: page reloaded: {reloaded}"));
                }
            });
            let reloaded_at = now_ms();
            std::thread::sleep(RELOAD_WAIT);
            if next_step(ANSWERED_AT.load(Ordering::SeqCst), reloaded_at, true) == Step::Rebuild {
                crate::log::line("live preview: still no answer after the reload; building it again");
                rebuild(&app);
            } else {
                crate::log::line("live preview: answered after the reload");
            }
        }
        CHECKING.store(false, Ordering::SeqCst);
    });
    if spawned.is_err() {
        CHECKING.store(false, Ordering::SeqCst);
    }
}

/// Throws the window away and makes it again; the last revision waits for
/// the new page (PENDING, page_ready). Runs on the check thread: destroy()
/// is carried out by the UI thread, so the label is waited for.
fn rebuild(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Err(err) = win.destroy() {
            crate::log::line(format!("live preview: could not close it: {err}"));
            return;
        }
    }
    for _ in 0..30 {
        if app.get_webview_window(LABEL).is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if app.get_webview_window(LABEL).is_some() {
        crate::log::line("live preview: the old window did not close; not rebuilt");
        return;
    }
    crate::webview_health::forget_watch(LABEL);
    CREATING.store(false, Ordering::SeqCst);
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = LAST_SHOW.lock().unwrap_or_else(|e| e.into_inner()).clone();
    create(app);
    crate::log::line("live preview: built again");
}

/// The page has loaded and listens (lib.rs live_page_ready): the revision
/// that came before it, if it's still the one to show.
pub fn page_ready(app: &AppHandle) {
    ANSWERED_AT.store(now_ms(), Ordering::SeqCst);
    let pending = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((event, at)) = pending {
        if app.get_webview_window(LABEL).is_some() {
            apply(app, &event, at);
        }
    }
}

/// Follows the paragraph while the revision is shown.
pub fn follow(app: &AppHandle, at: Option<Rect>) {
    if let (Some(win), Some(r)) = (app.get_webview_window(LABEL), at) {
        move_to(&win, r);
    }
}

fn move_to(win: &tauri::WebviewWindow, r: Rect) {
    let _ = win.set_size(PhysicalSize::new(r.width().max(1) as u32, r.height().max(1) as u32));
    let _ = win.set_position(PhysicalPosition::new(r.left, r.top));
}

fn page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/live.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("live.html".into())
}

/// Creates the window once, on the main thread: visible, parked, never
/// focusable. The next revision moves it into place.
fn create(app: &AppHandle) {
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let built = WebviewWindowBuilder::new(&handle, LABEL, page_url(&handle))
            .additional_browser_args(&crate::browser_args())
            .title("Glim")
            .inner_size(live::PREVIEW_W, live::PREVIEW_H)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .maximizable(false)
            .minimizable(false)
            .closable(false)
            .disable_drag_drop_handler()
            // tao keeps WS_EX_NOACTIVATE only through its FOCUSABLE flag.
            .focusable(false)
            .position(PARKED.0 as f64, PARKED.1 as f64)
            .visible(true)
            .build();
        match built {
            Ok(win) => {
                crate::platform::make_non_activating(&win);
                crate::screen_capture::apply_to(&handle, &win);
                crate::webview_health::watch(&win);
            }
            Err(err) => {
                crate::log::line(format!("live preview window failed: {err}"));
                CREATING.store(false, Ordering::SeqCst);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{next_step, Step};

    #[test]
    fn a_page_that_doesnt_answer_is_reloaded_then_built_again() {
        assert_eq!(next_step(10, 5, false), Step::Fine);
        assert_eq!(next_step(4, 5, false), Step::Reload);
        assert_eq!(next_step(4, 5, true), Step::Rebuild);
        assert_eq!(next_step(9, 5, true), Step::Fine);
        let code = include_str!("live_window.rs");
        let code = &code[..code.find("#[cfg(test)]").unwrap()];
        let ready = &code[code.find("pub fn page_ready").unwrap()..];
        assert!(ready.find("ANSWERED_AT.store").unwrap() < ready.find("PENDING").unwrap(), "every page_ready is an answer");
        assert!(code.contains("forget_watch(LABEL)") && code.contains("CREATING.store(false"), "a rebuilt window is watched and made again");
    }

    /// Like the mini lantern's: nothing here hands the window the keyboard.
    #[test]
    fn the_preview_never_takes_focus() {
        let code = include_str!("live_window.rs");
        let code = &code[..code.find("#[cfg(test)]").unwrap()];
        for banned in [".show()", ".set_focus(", ".focused(true)", "SetForegroundWindow", "SetFocus("] {
            assert!(!code.contains(banned), "{banned}");
        }
        assert!(code.contains(".focusable(false)") && code.contains(".focused(false)") && code.contains("make_non_activating"));
        assert!(code.contains("additional_browser_args(&crate::browser_args())"));
    }
}
