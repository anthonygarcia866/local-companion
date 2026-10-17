// Glim's Pause (1d, docs/phase1-plan.md §14.6): the tray's Pause / Resume
// and the pause key (Ctrl+Alt+Shift+P), both here in Rust so the key works
// with the island hidden. Pause stops the writing features only (reading,
// checking, keeping, fixes, rewrites: writing::set_paused); agents carry on.
// It is never stored: Glim always starts unpaused.

use tauri::{AppHandle, Emitter};

use crate::island::WINDOW_LABEL;

/// The event the island hears the state on (true = paused).
pub const EVENT: &str = "paused";

pub fn set(app: &AppHandle, on: bool) {
    crate::writing::set_paused(on);
    // The capture thread drops what it kept at once.
    #[cfg(windows)]
    crate::capture::uia::request(crate::capture::uia::Request::Wake);
    crate::tray::show_paused(app, on);
    crate::log::line(format!("pause: {}", if on { "on" } else { "off" }));
    let _ = app.emit_to(WINDOW_LABEL, EVENT, on);
}

pub fn toggle(app: &AppHandle) {
    set(app, !crate::writing::paused());
}
