// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), click-through and the cursor poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::placement::{nearest_dock, window_rect, Dock, Shape};
use crate::platform::{self, cursor_physical, left_button_down};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;

pub const WINDOW_LABEL: &str = "island";

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into the OS when it changes.
    ignoring: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    pub(crate) fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// A display's logical origin, the key `at:<x>,<y>` preferences are matched on.
/// Names are no good for that: two monitors of the same model share one.
fn logical_origin(m: &Monitor) -> (i32, i32) {
    let scale = m.scale_factor();
    let p = m.position();
    ((p.x as f64 / scale).round() as i32, (p.y as f64 / scale).round() as i32)
}

/// One entry of the "Island lives on" list in Settings.
#[derive(Serialize, Clone)]
pub struct MonitorChoice {
    pub key: String,
    pub label: String,
}

pub fn monitor_choices(app: &AppHandle) -> Vec<MonitorChoice> {
    let Ok(monitors) = app.available_monitors() else { return Vec::new() };
    monitors
        .iter()
        .map(|m| {
            let d = describe(m);
            MonitorChoice {
                key: d.key(),
                label: crate::i18n::tf(
                    "{name} — {width}×{height} at {x},{y}",
                    &[
                        ("name", &d.name),
                        ("width", &d.w.to_string()),
                        ("height", &d.h.to_string()),
                        ("x", &d.x.to_string()),
                        ("y", &d.y.to_string()),
                    ],
                ),
            }
        })
        .collect()
}

/// What a display is remembered by: its logical origin, plus its name and
/// logical size, so it is still found after the layout is rearranged or the
/// resolution changes (the Mac keeps the display's UUID for the same reason;
/// Tauri has no stable ID).
#[derive(Debug, Clone, PartialEq)]
struct DisplayId {
    name: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl DisplayId {
    /// `at:<x>,<y>` stays first, so a preference saved before still matches.
    fn key(&self) -> String {
        format!("at:{},{}|{}|{}x{}", self.x, self.y, self.name.replace('|', " "), self.w, self.h)
    }
}

fn describe(m: &Monitor) -> DisplayId {
    let (x, y) = logical_origin(m);
    let scale = m.scale_factor();
    let s = m.size();
    DisplayId {
        name: m.name().cloned().unwrap_or_else(|| "Display".into()),
        x,
        y,
        w: (s.width as f64 / scale).round() as i32,
        h: (s.height as f64 / scale).round() as i32,
    }
}

/// Which display a saved `at:` preference points at, best match first: same
/// place and name; the same name and size elsewhere (layout rearranged); the
/// same name alone when unique (resolution changed); the same place. None
/// means unplugged, and the caller falls back to the primary display.
fn pick_display(pref: &str, displays: &[DisplayId]) -> Option<usize> {
    let rest = pref.strip_prefix("at:")?;
    let mut parts = rest.split('|');
    let (x, y) = parts.next()?.split_once(',')?;
    let (x, y) = (x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?);
    let name = parts.next();
    let size = parts.next().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
    });
    let at = |d: &DisplayId| d.x == x && d.y == y;
    if let Some(name) = name {
        if let Some(i) = displays.iter().position(|d| at(d) && d.name == name) {
            return Some(i);
        }
        if let Some((w, h)) = size {
            if let Some(i) = displays.iter().position(|d| d.name == name && d.w == w && d.h == h) {
                return Some(i);
            }
        }
        let mut same_name = displays.iter().enumerate().filter(|(_, d)| d.name == name);
        if let (Some((i, _)), None) = (same_name.next(), same_name.next()) {
            return Some(i);
        }
    }
    displays.iter().position(at)
}

/// The display "active" last resolved to, by DisplayId key: kept while Glim
/// itself (or nothing) is in front.
static LAST_ACTIVE: Mutex<Option<String>> = Mutex::new(None);

/// The display the island lives on: a chosen one, the primary one, the one of
/// the active (foreground) window, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    let ids: Vec<DisplayId> = monitors.iter().map(describe).collect();
    if let Some(i) = pick_display(pref, &ids) {
        return Some(monitors[i].clone());
    }
    if pref == "active" {
        if let Some((cx, cy)) = platform::foreground_center() {
            if let Some(i) = monitors.iter().position(|m| monitor_contains(m, cx, cy)) {
                *LAST_ACTIVE.lock().unwrap() = Some(ids[i].key());
                return Some(monitors[i].clone());
            }
        }
        let last = LAST_ACTIVE.lock().unwrap().clone();
        if let Some(i) = last.and_then(|k| pick_display(&k, &ids)) {
            return Some(monitors[i].clone());
        }
    }
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

#[cfg(test)]
mod display_tests {
    use super::*;

    fn d(name: &str, x: i32, y: i32, w: i32, h: i32) -> DisplayId {
        DisplayId { name: name.into(), x, y, w, h }
    }

    #[test]
    fn a_display_is_found_again_after_changes() {
        let dell = d("DELL U2720Q", 1920, 0, 2560, 1440);
        let lap = d("eDP-1", 0, 0, 1920, 1200);
        let key = dell.key();
        assert_eq!(pick_display(&key, &[lap.clone(), dell.clone()]), Some(1));
        // Rearranged: the Dell moved to the left of the laptop.
        let moved = [d("eDP-1", 2560, 0, 1920, 1200), d("DELL U2720Q", 0, 0, 2560, 1440)];
        assert_eq!(pick_display(&key, &moved), Some(1));
        // Resolution changed, still the only Dell.
        let rescaled = [lap.clone(), d("DELL U2720Q", 1920, 0, 1920, 1080)];
        assert_eq!(pick_display(&key, &rescaled), Some(1));
        // Unplugged: nothing, so the caller falls back to the primary display.
        assert_eq!(pick_display(&key, &[lap.clone()]), None);
    }

    #[test]
    fn two_identical_monitors_are_told_apart_by_place() {
        let a = d("LG 27UL500", 0, 0, 1920, 1080);
        let b = d("LG 27UL500", 1920, 0, 1920, 1080);
        assert_eq!(pick_display(&b.key(), &[a.clone(), b.clone()]), Some(1));
        assert_eq!(pick_display(&a.key(), &[a, b]), Some(0));
    }

    #[test]
    fn a_window_moved_off_its_place_is_noticed() {
        assert!(!drifted(None, Some((0, 0))));
        assert!(!drifted(Some((100, 0, 720, 320)), Some((100, 0))));
        // Windows moved it to the display on the left while it was folded away.
        assert!(drifted(Some((100, 0, 720, 320)), Some((-1272, -58))));
        assert!(!drifted(Some((100, 0, 720, 320)), None));
    }

    #[test]
    fn active_and_cursor_are_not_saved_displays() {
        let lap = d("eDP-1", 0, 0, 1920, 1200);
        assert_eq!(pick_display("active", &[lap.clone()]), None);
        assert_eq!(pick_display("cursor", &[lap]), None);
    }

    #[test]
    fn preferences_saved_before_still_match() {
        let lap = d("eDP-1", 0, 0, 1920, 1200);
        let ext = d("HDMI-1", 1920, 0, 1920, 1080);
        assert_eq!(pick_display("at:1920,0", &[lap.clone(), ext.clone()]), Some(1));
        assert_eq!(pick_display("primary", &[lap, ext]), None);
        assert_eq!(pick_display("at:nonsense", &[]), None);
    }
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// Where apply_geometry last put the window: physical x, y, width, height.
/// The display watch puts it back if anything else moves it.
static PLACED: Mutex<Option<(i32, i32, u32, u32)>> = Mutex::new(None);

/// The dock the island uses on the display it lives on (see placement.rs).
static CURRENT_DOCK: Mutex<Dock> = Mutex::new(Dock::TopCenter);

pub fn current_dock() -> Dock {
    *CURRENT_DOCK.lock().unwrap()
}

/// What the page needs to lay the island out in its window.
#[derive(Serialize, Clone)]
pub struct PlacementPayload {
    pub dock: &'static str,
    pub vertical: bool,
}

/// The dock remembered for display `m` (settings.docks), matched the way
/// `at:` display preferences are, so a rearranged or rescaled display keeps
/// its dock. Top centre when none was chosen.
fn dock_for(app: &AppHandle, m: &Monitor) -> Dock {
    let Some(shared) = app.try_state::<crate::Shared>() else { return Dock::TopCenter };
    let docks = shared.settings.lock().unwrap().docks.clone();
    let Ok(monitors) = app.available_monitors() else { return Dock::TopCenter };
    let ids: Vec<DisplayId> = monitors.iter().map(describe).collect();
    let me = describe(m);
    let Some(here) = ids.iter().position(|d| *d == me) else { return Dock::TopCenter };
    docks
        .iter()
        .find(|(key, _)| pick_display(key, &ids) == Some(here))
        .and_then(|(_, d)| Dock::parse(d))
        .unwrap_or(Dock::TopCenter)
}

/// Places and sizes the window. `collapsed` picks the wake strip instead of the panel.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let dock = dock_for(app, &m);
    let previous = std::mem::replace(&mut *CURRENT_DOCK.lock().unwrap(), dock);
    if previous != dock {
        let _ = app.emit_to(WINDOW_LABEL, "placement", PlacementPayload { dock: dock.as_str(), vertical: dock.vertical() });
    }
    let shape = match crate::presence::current(app) {
        crate::presence::Presence::Hidden => {
            let _ = win.hide();
            return;
        }
        crate::presence::Presence::Ember | crate::presence::Presence::Indicator => Shape::Ember,
        crate::presence::Presence::Pill if collapsed => Shape::Strip,
        crate::presence::Presence::Pill => Shape::Panel,
    };
    if !win.is_visible().unwrap_or(true) {
        platform::show_no_activate(&win);
    }
    let (rx, ry, lw, lh) = window_rect(dock, shape, ms.width as f64 / scale, ms.height as f64 / scale);
    let pw = (lw * scale).round().max(1.0) as u32;
    let ph = (lh * scale).round().max(1.0) as u32;
    let x = mp.x + (rx * scale).round() as i32;
    let y = mp.y + (ry * scale).round() as i32;

    // GTK never sizes a non-resizable window below its natural size (200 px
    // here), so on Linux the 6 px wake strip would stay a 200 px block. tao
    // re-applies the config's `resizable: false` after the first configure, so
    // this is asked every time, just before the resize. Undecorated, the window
    // still offers the user nothing to resize it by. (Found by @YossiYad, #44.)
    #[cfg(target_os = "linux")]
    let _ = win.set_resizable(true);
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let (lx, ly) = logical_origin(&m);
    platform::pin_to_monitor(&win, lx, ly);
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);

    // Where it really landed (Windows may round on a scaled display): what the
    // display watch compares against, so a rounding never reads as a drift.
    let (x, y) = win.outer_position().map(|p| (p.x, p.y)).unwrap_or((x, y));
    let placed = (x, y, pw, ph);
    let mut last = PLACED.lock().unwrap();
    if last.is_none_or(|l| (l.0, l.1) != (x, y)) {
        let d = describe(&m);
        crate::log::line(format!(
            "island placed on {} ({}x{} at {},{}, scale {}) for screen={pref}, dock {}",
            d.name, d.w, d.h, d.x, d.y, scale, dock.as_str()
        ));
    }
    *last = Some(placed);
}

/// Every display: name, logical size and origin, and scale. For the log.
fn displays_summary(app: &AppHandle) -> String {
    let Ok(monitors) = app.available_monitors() else { return "unknown".into() };
    let list: Vec<String> = monitors
        .iter()
        .map(|m| {
            let d = describe(m);
            format!("{} {}x{} at {},{} scale {}", d.name, d.w, d.h, d.x, d.y, m.scale_factor())
        })
        .collect();
    format!("{} — {}", list.len(), list.join("; "))
}

/// Whether the window is no longer where apply_geometry put it: Windows moves
/// windows itself when a display sleeps, is unplugged or changes scale, and an
/// island left on another display (or off every display) is unreachable.
fn drifted(placed: Option<(i32, i32, u32, u32)>, now: Option<(i32, i32)>) -> bool {
    match (placed, now) {
        (Some(p), Some(n)) => (p.0, p.1) != n,
        _ => false,
    }
}

/// Watches, once a second and whether or not the island is showing: the
/// display the island should be on (plugged in, unplugged, rearranged,
/// rescaled, or the active window moving to another display), and the window
/// drifting from where Glim put it. Either way the page is told to reposition.
pub fn spawn_display_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_screen = current_screen_key(&app);
        let mut last_displays = displays_summary(&app);
        crate::log::line(format!("displays: {last_displays}"));
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if DRAGGING.load(Ordering::Relaxed) {
                continue;
            }
            let displays = displays_summary(&app);
            if displays != last_displays {
                crate::log::line(format!("displays: {displays}"));
                last_displays = displays;
            }
            let pref = app
                .try_state::<crate::Shared>()
                .map(|s| s.settings.lock().unwrap().screen.clone())
                .unwrap_or_else(|| "primary".into());
            if let Some(m) = target_monitor(&app, &pref) {
                let (p, z) = (*m.position(), *m.size());
                crate::presence::set_fullscreen(&app, platform::foreground_fullscreen((p.x, p.y, z.width, z.height)));
            }
            let now = current_screen_key(&app);
            let at = window(&app).and_then(|w| w.outer_position().ok()).map(|p| (p.x, p.y));
            let reason = if now.is_some() && now != last_screen {
                last_screen = now;
                Some("the island's display changed".to_string())
            } else if drifted(*PLACED.lock().unwrap(), at) {
                Some(format!("the window was moved off its place (now at {:?})", at.unwrap_or_default()))
            } else {
                None
            };
            if let Some(reason) = reason {
                crate::log::line(format!("{reason} — repositioning"));
                let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                // Until the page has placed it again, don't report the same drift twice.
                *PLACED.lock().unwrap() = None;
            }
        }
    });
}

/// Set while the pill is being dragged: the watch leaves the window alone.
pub static DRAGGING: AtomicBool = AtomicBool::new(false);

/// Starts dragging the pill: the window follows the cursor (it never takes
/// focus: it is moved, not activated) until the left button is released, then
/// snaps to the nearest dock of the display under the cursor.
pub fn start_dock_drag(app: &AppHandle) {
    if DRAGGING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (Some(win), Some((cx, cy))) = (window(app), cursor_physical()) else {
        DRAGGING.store(false, Ordering::SeqCst);
        return;
    };
    let Ok(origin) = win.outer_position() else {
        DRAGGING.store(false, Ordering::SeqCst);
        return;
    };
    let grab = (cx - origin.x as f64, cy - origin.y as f64);
    let app = app.clone();
    std::thread::spawn(move || {
        while left_button_down() {
            if let Some((cx, cy)) = cursor_physical() {
                let _ = win.set_position(PhysicalPosition::new((cx - grab.0).round() as i32, (cy - grab.1).round() as i32));
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        drop_dock_drag(&app);
        DRAGGING.store(false, Ordering::SeqCst);
    });
}

/// The drag ended: the nearest dock of the display under the cursor.
fn drop_dock_drag(app: &AppHandle) {
    let Some((cx, cy)) = cursor_physical() else { return };
    let Ok(monitors) = app.available_monitors() else { return };
    let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) else { return };
    let scale = m.scale_factor();
    let (mp, ms) = (*m.position(), *m.size());
    let dock = nearest_dock(
        (cx - mp.x as f64) / scale,
        (cy - mp.y as f64) / scale,
        ms.width as f64 / scale,
        ms.height as f64 / scale,
    );
    crate::log::line(format!("pill dropped on {} — dock {}", describe(m).name, dock.as_str()));
    set_dock(app, Some(m), dock);
}

/// Remembers `dock` for display `on` (the island's own display when None) and
/// places the island there. A drop on another display moves the island to it,
/// unless it follows the active window or the cursor anyway.
pub fn set_dock(app: &AppHandle, on: Option<&Monitor>, dock: Dock) {
    let Some(shared) = app.try_state::<crate::Shared>() else { return };
    let pref = shared.settings.lock().unwrap().screen.clone();
    let Some(target) = target_monitor(app, &pref) else { return };
    let m = on.cloned().unwrap_or(target.clone());
    let key = describe(&m).key();
    let settings = {
        let mut s = shared.settings.lock().unwrap();
        // One entry per display: drop any older key that finds the same one.
        let ids: Vec<DisplayId> = app.available_monitors().map(|v| v.iter().map(describe).collect()).unwrap_or_default();
        let here = ids.iter().position(|d| *d == describe(&m));
        s.docks.retain(|k, _| here.is_none() || pick_display(k, &ids) != here);
        s.docks.insert(key.clone(), dock.as_str().to_string());
        if describe(&m) != describe(&target) && pref != "active" && pref != "cursor" {
            s.screen = key;
        }
        s.clone()
    };
    if let Err(err) = crate::settings::save(&settings) {
        crate::log::line(format!("could not save settings: {err}"));
    }
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    apply_geometry(app, &settings.screen, collapsed);
    let _ = app.emit_to(WINDOW_LABEL, "settings-changed", settings);
}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// Emits `cursor` (window-logical coordinates) at ~60 Hz while the island is
/// visible. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        let mut was_down = false;
        // Without a cursor to read (Linux) there's nothing to do at 60 Hz.
        let period = if platform::CURSOR_POLL { 16 } else { 500 };
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            while gate.is_active() {
                std::thread::sleep(Duration::from_millis(period));
                // Display changes are spawn_display_watch's.

                let Some(win) = window(&app) else { continue };
                let Ok(origin) = win.outer_position() else { continue };
                let scale = win.scale_factor().unwrap_or(1.0);
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - origin.x as f64) / scale;
                let y = (cy - origin.y as f64) / scale;
                let size = match win.inner_size() {
                    Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                    Err(_) => (PANEL_W, PANEL_H),
                };
                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let r = *gate.rect.lock().unwrap();
                let on_island = r.w > 0.0
                    && x >= r.x - HIT_MARGIN
                    && x <= r.x + r.w + HIT_MARGIN
                    && y >= r.y - HIT_MARGIN
                    && y <= r.y + r.h + HIT_MARGIN;

                // A file being dragged has to be able to find us. WS_EX_TRANSPARENT
                // — what click-through is on Windows — hides the window from
                // WindowFromPoint, so OLE finds no drop target and shows the "no
                // drop" cursor. macOS has no such problem: AppKit delivers drags to
                // registered destinations whatever ignoresMouseEvents says. So while
                // a button is held anywhere over the panel, the whole panel takes
                // the mouse, which also makes the drop zone as forgiving as the Mac's.
                // A press may be the start of a drag: make sure the drop target is
                // ours before the file arrives.
                let down = left_button_down();
                if down && !was_down {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || platform::unblock_webview_drops(&handle));
                }
                was_down = down;

                let dragging = down
                    && x >= 0.0
                    && x <= size.0
                    && y >= 0.0
                    && y <= size.1;

                // The ember (and the recording indicator) fills its small
                // window, whatever rect the page last sent: all of it is the dot.
                let dot = crate::presence::current(&app).is_dot();
                let accept = on_island || dragging || dot;
                if gate.ignoring.load(Ordering::Relaxed) == accept {
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
        }
    });
}

/// Re-applies click-through after the window or the island changed shape.
///
/// With the cursor poll (Windows) the window takes the mouse again and the next
/// tick decides from the cursor. Without it (Linux) the input region is set to
/// the island itself, or to the whole wake strip while collapsed.
pub fn refresh_click_through(app: &AppHandle, gate: &PollGate) {
    if platform::CURSOR_POLL {
        set_ignore_cursor(app, false);
        gate.forget_ignore_state();
        return;
    }
    let Some(win) = window(app) else { return };
    let region = if gate.collapsed.load(Ordering::Relaxed) {
        // The wake strip itself, never "the whole window": if the window ever
        // fails to shrink to the strip, the rest of it must not swallow clicks
        // meant for whatever sits under the top of the screen.
        Some((0.0, 0.0, STRIP_W, STRIP_H))
    } else {
        let r = *gate.rect.lock().unwrap();
        if r.w <= 0.0 {
            // Nothing drawn yet: nothing takes the mouse.
            Some((0.0, 0.0, 0.0, 0.0))
        } else {
            let x0 = (r.x - HIT_MARGIN).max(0.0);
            let y0 = (r.y - HIT_MARGIN).max(0.0);
            let x1 = r.x + r.w + HIT_MARGIN;
            let y1 = r.y + r.h + HIT_MARGIN;
            Some((x0, y0, x1 - x0, y1 - y0))
        }
    };
    platform::set_input_region(&win, region);
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
}
