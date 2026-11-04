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

use std::sync::atomic::{AtomicBool, Ordering};

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
/// "…" was clicked: the preview is taller until the next revision.
static EXPANDED: AtomicBool = AtomicBool::new(false);

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
        if event.phase == "show" {
            create(app);
        }
        return;
    };
    match (event.phase, at) {
        ("hide", _) | (_, None) => {
            let _ = win.set_position(PhysicalPosition::new(PARKED.0, PARKED.1));
        }
        (_, Some(r)) => move_to(&win, r),
    }
    let _ = app.emit_to(LABEL, EVENT, event);
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
