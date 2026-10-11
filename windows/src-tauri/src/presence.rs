// How present the island is: the visibility mode the user picked (Normal =
// the pill, Ember = a small glowing dot at the dock's edge, Hidden = gone,
// back through the tray icon or the hotkey), auto-hidden while a fullscreen
// app is in front (a presentation, a video, a screen share), and one rule over
// all of it: while the screen is being recorded (Phase 2), a recording
// indicator stays on screen in every mode, Hidden and fullscreen included.
// `presence()` is that rule; tests below enforce it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::island::WINDOW_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Normal,
    Ember,
    Hidden,
}

impl Visibility {
    pub fn parse(s: &str) -> Option<Visibility> {
        match s.trim() {
            "normal" => Some(Visibility::Normal),
            "ember" => Some(Visibility::Ember),
            "hidden" => Some(Visibility::Hidden),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Normal => "normal",
            Visibility::Ember => "ember",
            Visibility::Hidden => "hidden",
        }
    }

    /// The hotkey's cycle: Normal → Ember → Hidden → Normal.
    pub fn next(self) -> Visibility {
        match self {
            Visibility::Normal => Visibility::Ember,
            Visibility::Ember => Visibility::Hidden,
            Visibility::Hidden => Visibility::Normal,
        }
    }
}

/// What is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// The pill, and the island it opens into.
    Pill,
    /// The ember dot.
    Ember,
    /// Nothing at all: the window is hidden.
    Hidden,
    /// The recording indicator alone (the ember's place, s-record): what
    /// stays when the island would otherwise be hidden while recording.
    Indicator,
}

impl Presence {
    /// The small window: the ember or the recording indicator.
    pub fn is_dot(self) -> bool {
        matches!(self, Presence::Ember | Presence::Indicator)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Presence::Pill => "pill",
            Presence::Ember => "ember",
            Presence::Hidden => "hidden",
            Presence::Indicator => "indicator",
        }
    }
}

/// The rule. Recording can never end in `Hidden`. `peek`: the ember was
/// clicked to see the writing suggestions, so the pill (and the island) is
/// out until the island closes again; Ember stays the saved mode.
pub fn presence(vis: Visibility, fullscreen: bool, recording: bool, peek: bool) -> Presence {
    match (vis, fullscreen) {
        (Visibility::Normal, false) => Presence::Pill,
        (Visibility::Ember, false) if peek => Presence::Pill,
        (Visibility::Ember, false) => Presence::Ember,
        _ if recording => Presence::Indicator,
        _ => Presence::Hidden,
    }
}

/// Set by the screen recorder (Phase 2) for as long as it records. Nothing
/// sets it yet: there is no recorder.
static RECORDING: AtomicBool = AtomicBool::new(false);
/// The ember was clicked to open the writing suggestions (see `presence`).
static PEEK: AtomicBool = AtomicBool::new(false);
/// A fullscreen app is in front on the island's display (the display watch).
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
/// `--fullscreen on|off|auto` in a dev session: pretends, for testing.
static DEV_FULLSCREEN: Mutex<Option<bool>> = Mutex::new(None);
/// What was last applied, so only a change is acted on.
static LAST: Mutex<Option<Presence>> = Mutex::new(None);

/// For the screen recorder (Phase 2) only: recording started or stopped.
#[allow(dead_code)]
pub fn set_recording(app: &AppHandle, on: bool) {
    RECORDING.store(on, Ordering::SeqCst);
    refresh(app);
}

pub fn set_fullscreen(app: &AppHandle, on: bool) {
    if FULLSCREEN.swap(on, Ordering::SeqCst) != on {
        crate::log::line(format!("fullscreen app in front: {on}"));
        refresh(app);
    }
}

pub fn set_dev_fullscreen(app: &AppHandle, on: Option<bool>) {
    *DEV_FULLSCREEN.lock().unwrap() = on;
    refresh(app);
}

fn fullscreen() -> bool {
    DEV_FULLSCREEN.lock().unwrap().unwrap_or_else(|| FULLSCREEN.load(Ordering::SeqCst))
}

fn visibility(app: &AppHandle) -> Visibility {
    app.try_state::<crate::Shared>()
        .and_then(|s| Visibility::parse(&s.settings.lock().unwrap().visibility))
        .unwrap_or(Visibility::Normal)
}

/// What should be on screen now.
pub fn current(app: &AppHandle) -> Presence {
    presence(visibility(app), fullscreen(), RECORDING.load(Ordering::SeqCst), PEEK.load(Ordering::SeqCst))
}

/// The ember's click with writing suggestions waiting (on), and the island
/// closing after it (off). Never saved: Ember stays the user's mode.
pub fn set_peek(app: &AppHandle, on: bool) {
    if PEEK.swap(on, Ordering::SeqCst) != on {
        refresh(app);
    }
}

#[derive(Serialize, Clone)]
pub struct PresencePayload {
    pub kind: &'static str,
    pub visibility: &'static str,
    pub recording: bool,
}

pub fn payload(app: &AppHandle) -> PresencePayload {
    PresencePayload {
        kind: current(app).as_str(),
        visibility: visibility(app).as_str(),
        recording: RECORDING.load(Ordering::SeqCst),
    }
}

/// Acts on a change: hides or shows the window, gives it its shape, and tells
/// the page.
pub fn refresh(app: &AppHandle) {
    let now = current(app);
    let before = LAST.lock().unwrap().replace(now);
    if before == Some(now) {
        return;
    }
    crate::log::line(format!("presence: {}", now.as_str()));
    if let Some(shared) = app.try_state::<crate::Shared>() {
        let pref = shared.settings.lock().unwrap().screen.clone();
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        crate::island::apply_geometry(app, &pref, collapsed);
        crate::island::refresh_click_through(app, &shared.gate);
    }
    let _ = app.emit_to(WINDOW_LABEL, "presence", payload(app));
}

/// Picks a visibility mode, saves it, and applies it.
pub fn set_visibility(app: &AppHandle, vis: Visibility) {
    let Some(shared) = app.try_state::<crate::Shared>() else { return };
    // Picking a mode ends a peek.
    PEEK.store(false, Ordering::SeqCst);
    let settings = {
        let mut s = shared.settings.lock().unwrap();
        s.visibility = vis.as_str().to_string();
        s.clone()
    };
    if let Err(err) = crate::settings::save(&settings) {
        crate::log::line(format!("could not save settings: {err}"));
    }
    crate::log::line(format!("visibility: {}", vis.as_str()));
    refresh(app);
    let _ = app.emit_to(WINDOW_LABEL, "settings-changed", settings.clone());
    let _ = app.emit_to("settings", "settings-changed", settings);
}

/// The hotkey: the next mode in the cycle.
pub fn cycle(app: &AppHandle) {
    set_visibility(app, visibility(app).next());
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Visibility; 3] = [Visibility::Normal, Visibility::Ember, Visibility::Hidden];

    #[test]
    fn each_mode_shows_what_it_says() {
        assert_eq!(presence(Visibility::Normal, false, false, false), Presence::Pill);
        assert_eq!(presence(Visibility::Ember, false, false, false), Presence::Ember);
        assert_eq!(presence(Visibility::Hidden, false, false, false), Presence::Hidden);
    }

    #[test]
    fn a_fullscreen_app_hides_every_mode() {
        for v in ALL {
            for peek in [false, true] {
                assert_eq!(presence(v, true, false, peek), Presence::Hidden, "{v:?}");
            }
        }
    }

    /// A peek brings the pill out of the ember only: never out of Hidden,
    /// never over a fullscreen app, and it is never saved.
    #[test]
    fn a_peek_opens_the_ember_only() {
        assert_eq!(presence(Visibility::Ember, false, false, true), Presence::Pill);
        assert_eq!(presence(Visibility::Hidden, false, false, true), Presence::Hidden);
        assert_eq!(presence(Visibility::Normal, false, false, true), Presence::Pill);
        let src = include_str!("presence.rs").replace("\r\n", "\n");
        let peek = &src[src.find("pub fn set_peek").unwrap()..];
        let body = &peek[..peek.find("\n}\n").unwrap()];
        assert!(!body.contains("save"), "a peek is never saved");
        let set = &src[src.find("pub fn set_visibility").unwrap()..];
        assert!(set[..set.find("\n}\n").unwrap()].contains("PEEK.store(false"), "picking a mode ends a peek");
    }

    /// The recording rule: while recording, something is always on screen —
    /// in every mode, Hidden and fullscreen included.
    #[test]
    fn recording_is_never_hidden() {
        for v in ALL {
            for fullscreen in [false, true] {
                let p = presence(v, fullscreen, true, false);
                assert_ne!(p, Presence::Hidden, "{v:?}, fullscreen {fullscreen}");
            }
        }
        assert_eq!(presence(Visibility::Hidden, false, true, false), Presence::Indicator);
        assert_eq!(presence(Visibility::Normal, true, true, false), Presence::Indicator);
    }

    /// Hidden → Normal once left the window hidden: it was hidden through tao
    /// and shown with a raw ShowWindow, and tao re-applies its own idea of
    /// visibility on every later flag change. Both ways go through tao.
    #[test]
    fn the_island_is_shown_and_hidden_through_tao_only() {
        let island = include_str!("island.rs").replace("\r\n", "\n");
        assert!(!island.contains("ShowWindow") && !island.contains("show_no_activate"));
        let start = island.find("pub fn apply_geometry").unwrap();
        let body = &island[start..start + island[start..].find("\n}\n").unwrap()];
        assert!(body.contains("let _ = win.show();") && body.contains("let _ = win.hide();"));
        let platform = include_str!("platform/windows.rs");
        assert!(!platform.contains("fn show_no_activate"));
    }

    #[test]
    fn the_hotkey_cycles_through_all_three() {
        let mut v = Visibility::Normal;
        let mut seen = Vec::new();
        for _ in 0..3 {
            v = v.next();
            seen.push(v);
        }
        assert_eq!(seen, [Visibility::Ember, Visibility::Hidden, Visibility::Normal]);
        for v in ALL {
            assert_eq!(Visibility::parse(v.as_str()), Some(v));
        }
    }
}
