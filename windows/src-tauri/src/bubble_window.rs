// The mini lantern's window (1e, docs/phase1-plan.md §7a): one small webview
// beside the text field, holding the lantern and, after a click, its popover.
// writing::bubble decides where and whether (pure); this file turns that into
// a window: created once, parked off screen, moved and sized on each reading.
//
// Focus (§7a.4): never takes it. Like desktop Mochi, the window is created
// visible at a parked spot and only ever moved, never shown or hidden
// (SW_SHOW may take the keyboard focus from the field the user is typing in);
// it is a tool window, topmost, WS_EX_NOACTIVATE through tao's FOCUSABLE
// flag, with BROWSER_ARGS. Nothing here hands it the keyboard (a test checks).
//
// Off by default (Settings → Writing, `writingBubble`): with the setting off
// the window is never created.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::writing::bubble::{self, Conditions, Hide, Look, Monitor, NoPlace, Placement, Rect, Tracker};

pub const LABEL: &str = "bubble";
/// What the bubble's page hears: where things are, the look, the count.
pub const EVENT: &str = "bubble-state";
/// The caret left the field (or the field lost focus): the popover closes.
pub const FIELD_LEFT: &str = "bubble-field-left";

/// The popover (§7a.3): 320 logical px wide; three rows and the footer.
pub const POPOVER_W: f64 = 320.0;
pub const POPOVER_H: f64 = 284.0;
/// Between the lantern and its popover.
const POPOVER_GAP: f64 = 6.0;
/// Far off every monitor: where the window waits while not shown.
const PARKED: (i32, i32) = (-32000, -32000);

/// The popover is open (the page says so through `bubble_popover`).
static OPEN: AtomicBool = AtomicBool::new(false);
/// The window was asked for (created once, on the main thread).
static CREATING: AtomicBool = AtomicBool::new(false);

/// Where the window goes and where its two parts sit inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The window, physical screen pixels.
    pub window: Rect,
    /// The lantern's top-left inside the window, physical pixels.
    pub lantern: (i32, i32),
    /// The popover's top-left inside the window, when open.
    pub popover: Option<(i32, i32)>,
    /// The popover opens above the lantern (no room below).
    pub above: bool,
}

fn px(logical: f64, scale: f64) -> i32 {
    (logical * scale).round() as i32
}

/// The window for a placed lantern: just the lantern, or with the popover
/// below it (else above), left-aligned to it and kept inside the work area.
pub fn layout(p: &Placement, work: Rect, scale: f64, open: bool) -> Layout {
    let lantern = p.rect();
    if !open {
        return Layout { window: lantern, lantern: (0, 0), popover: None, above: false };
    }
    let (w, h, gap) = (px(POPOVER_W, scale), px(POPOVER_H, scale), px(POPOVER_GAP, scale));
    let below = lantern.bottom + gap;
    let above = below + h > work.bottom && lantern.top - gap - h >= work.top;
    let top = if above { lantern.top - gap - h } else { below.min(work.bottom - h).max(work.top) };
    let left = lantern.left.min(work.right - w).max(work.left);
    let pop = Rect::new(left, top, left + w, top + h);
    let window = Rect::new(lantern.left.min(pop.left), lantern.top.min(pop.top), lantern.right.max(pop.right), lantern.bottom.max(pop.bottom));
    Layout {
        window,
        lantern: (lantern.left - window.left, lantern.top - window.top),
        popover: Some((pop.left - window.left, pop.top - window.top)),
        above,
    }
}

/// What the page is told, in logical pixels inside the window.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub visible: bool,
    /// "suggest" (s-suggest with the count) or "delegate" (a rewrite).
    pub look: &'static str,
    pub count: usize,
    pub size: f64,
    pub lantern_x: f64,
    pub lantern_y: f64,
    pub open: bool,
    pub popover_x: f64,
    pub popover_y: f64,
    pub above: bool,
}

/// Everything one reading tells the driver.
pub struct Input<'a> {
    pub conditions: Conditions,
    pub field: Option<Rect>,
    pub caret: Option<Rect>,
    /// The caret's line, when the app gives it (2026-10-18).
    pub line: Option<Rect>,
    /// Focus moved to another element since the last reading.
    pub new_field: bool,
    pub monitors: &'a [Monitor],
}

/// What the driver decided: where the window goes (None = parked) and what
/// the page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub window: Option<Rect>,
    pub state: State,
    /// The popover must close (field left, or the lantern hid).
    pub close: bool,
}

/// Why the last reading showed it or not: the log's line when it changes
/// (bug 3, 2026-10-18). Reasons and rectangles only, never text.
#[derive(Clone, Debug, PartialEq)]
pub enum Why {
    Shown(Placement),
    Hidden(Hide),
    /// The field moved since the last reading.
    Moving,
    /// No place for it: why, and the field's and caret's rectangles.
    NoPlace(NoPlace, Option<Rect>, Option<Rect>),
}

/// The capture thread's side: steadiness, the last placement, open or not.
#[derive(Default)]
pub struct Driver {
    tracker: Tracker,
    placed: Option<Placement>,
    last: Option<Decision>,
    why: Option<Why>,
}

impl Why {
    /// The same reason as `o`, wherever it is: the log says nothing new
    /// while the caret moves along a line.
    pub fn same_reason(&self, o: &Why) -> bool {
        match (self, o) {
            (Why::Shown(_), Why::Shown(_)) | (Why::Moving, Why::Moving) => true,
            (Why::Hidden(a), Why::Hidden(b)) => a == b,
            (Why::NoPlace(a, ..), Why::NoPlace(b, ..)) => a == b,
            _ => false,
        }
    }
}

fn rect_text(r: Option<Rect>) -> String {
    r.map_or("none".into(), |r| format!("({},{},{},{})", r.left, r.top, r.right, r.bottom))
}

/// The log line for `why`.
pub fn log_line(why: &Why, monitors: &[Monitor]) -> String {
    match why {
        Why::Shown(p) => format!("bubble: shown at ({},{}) size {} on monitor {}", p.x, p.y, p.size, p.monitor),
        Why::Hidden(h) => format!("bubble: hidden ({h:?})"),
        Why::Moving => "bubble: hidden while the field moves".into(),
        Why::NoPlace(n, field, caret) => {
            let mons: Vec<String> = monitors.iter().map(|m| format!("{}@{}", rect_text(Some(m.work)), m.scale)).collect();
            format!("bubble: no place ({n:?}) field={} caret={} monitors=[{}]", rect_text(*field), rect_text(*caret), mons.join(", "))
        }
    }
}

impl Driver {
    /// Why the last reading showed it or not.
    pub fn why(&self) -> Option<&Why> {
        self.why.as_ref()
    }

    /// One reading. `open` is whether the page has the popover open.
    pub fn step(&mut self, input: &Input, open: bool) -> Decision {
        if input.new_field {
            self.tracker.reset();
            self.placed = None;
        }
        let steady = self.tracker.steady(input.field);
        let place = match bubble::shown(&input.conditions) {
            Err(h) => Err(Why::Hidden(h)),
            Ok(()) if !steady => Err(Why::Moving),
            Ok(()) => bubble::place_on_line(input.field, input.caret, input.line, input.monitors).map_err(|n| Why::NoPlace(n, input.field, input.caret)),
        };
        let mut p = match place {
            Ok(p) => p,
            Err(why) => {
                self.why = Some(why);
                self.placed = None;
                let d = Decision { window: None, state: State::default(), close: open };
                self.last = Some(d.clone());
                return d;
            }
        };
        // Small wobbles don't move it (§7a.1).
        if let Some(prev) = self.placed.filter(|prev| !bubble::should_move(prev, &p)) {
            p = prev;
        }
        self.placed = Some(p);
        self.why = Some(Why::Shown(p));
        let m = input.monitors[p.monitor];
        let open = open && !input.new_field;
        let l = layout(&p, m.work, m.scale, open);
        let look = match bubble::look(&input.conditions) {
            Look::Delegate => "delegate",
            Look::Suggest(_) => "suggest",
        };
        let lg = |v: i32| v as f64 / m.scale;
        let state = State {
            visible: true,
            look,
            count: input.conditions.suggestions,
            size: bubble::SIZE,
            lantern_x: lg(l.lantern.0),
            lantern_y: lg(l.lantern.1),
            open,
            popover_x: l.popover.map_or(0.0, |p| lg(p.0)),
            popover_y: l.popover.map_or(0.0, |p| lg(p.1)),
            above: l.above,
        };
        let d = Decision { window: Some(l.window), state, close: input.new_field };
        self.last = Some(d.clone());
        d
    }
}

/// The popover opened or closed (the page): the next reading lays it out.
pub fn set_open(open: bool) {
    OPEN.store(open, Ordering::SeqCst);
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst)
}

/// The monitors as writing::bubble wants them: work areas and scales.
pub fn monitors(app: &AppHandle) -> Vec<Monitor> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let w = m.work_area();
            Monitor {
                work: Rect::new(w.position.x, w.position.y, w.position.x + w.size.width as i32, w.position.y + w.size.height as i32),
                scale: m.scale_factor(),
            }
        })
        .collect()
}

/// Applies a decision: moves and sizes the window (creating it the first
/// time it is needed), or parks it; tells the page.
pub fn apply(app: &AppHandle, d: &Decision, before: Option<&Decision>) {
    if before == Some(d) {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        if d.window.is_some() {
            create(app);
        }
        return;
    };
    if d.close {
        set_open(false);
        let _ = app.emit_to(LABEL, FIELD_LEFT, ());
    }
    match d.window {
        Some(r) => {
            let _ = win.set_size(PhysicalSize::new(r.width().max(1) as u32, r.height().max(1) as u32));
            let _ = win.set_position(PhysicalPosition::new(r.left, r.top));
        }
        None => {
            let _ = win.set_position(PhysicalPosition::new(PARKED.0, PARKED.1));
        }
    }
    let _ = app.emit_to(LABEL, EVENT, &d.state);
}

fn page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/bubble.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("bubble.html".into())
}

/// Creates the window once, on the main thread: visible, parked, never
/// focusable. The next reading moves it into place.
fn create(app: &AppHandle) {
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let built = WebviewWindowBuilder::new(&handle, LABEL, page_url(&handle))
            .additional_browser_args(&crate::browser_args())
            .title("Glim")
            .inner_size(bubble::SIZE, bubble::SIZE)
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
                crate::log::line(format!("bubble window failed: {err}"));
                CREATING.store(false, Ordering::SeqCst);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_monitor() -> Vec<Monitor> {
        vec![Monitor { work: Rect::new(0, 0, 1920, 1040), scale: 1.0 }, Monitor { work: Rect::new(-1920, -58, 0, 1142), scale: 1.25 }]
    }

    fn conditions(count: usize) -> Conditions {
        Conditions {
            setting_on: true,
            suggestions: count,
            editable: true,
            read: true,
            app_on: true,
            field_focused: true,
            app_foreground: true,
            ..Default::default()
        }
    }

    fn input(c: Conditions, field: Rect, new_field: bool, monitors: &[Monitor]) -> Input<'_> {
        // The caret moves with the field (on its first line).
        let caret = Rect::new(field.left + 100, field.top + 6, field.left + 101, field.top + 24);
        Input { conditions: c, field: Some(field), caret: Some(caret), line: None, new_field, monitors }
    }

    #[test]
    fn closed_it_is_just_the_lantern_and_open_the_popover_goes_below_else_above() {
        let p = Placement { x: 904, y: 304, size: 22, monitor: 0 };
        let work = Rect::new(0, 0, 1920, 1040);
        let closed = layout(&p, work, 1.0, false);
        assert_eq!((closed.window, closed.lantern, closed.popover), (p.rect(), (0, 0), None));
        let open = layout(&p, work, 1.0, true);
        assert!(!open.above);
        assert_eq!(open.popover, Some((0, 22 + 6)), "below, left-aligned");
        assert_eq!(open.window, Rect::new(904, 304, 904 + 320, 304 + 22 + 6 + 284));
        // Near the bottom: above.
        let low = Placement { x: 904, y: 1000, size: 22, monitor: 0 };
        let l = layout(&low, work, 1.0, true);
        assert!(l.above);
        assert_eq!(l.window.bottom, 1022);
        assert_eq!(l.lantern.1, 284 + 6);
        // Near the right edge: the popover is pulled in, still inside the work area.
        let right = Placement { x: 1890, y: 300, size: 22, monitor: 0 };
        let l = layout(&right, work, 1.0, true);
        assert_eq!(l.window.right, 1920, "the popover pulled in to the work area's edge");
        assert!(l.window.left + l.popover.unwrap().0 + 320 <= 1920);
        // 125%: 320 → 400 px, 22 → 28 px.
        let scaled = Placement { x: -1500, y: 100, size: 28, monitor: 1 };
        let l = layout(&scaled, Rect::new(-1920, -58, 0, 1142), 1.25, true);
        assert_eq!(l.window.width(), 400);
    }

    #[test]
    fn the_driver_places_it_tells_the_page_and_hides_it_when_any_condition_fails() {
        let mons = main_monitor();
        let field = Rect::new(400, 300, 900, 330);
        let mut d = Driver::default();
        let shown = d.step(&input(conditions(3), field, true, &mons), false);
        assert_eq!(shown.window, Some(Rect::new(904, 304, 926, 326)));
        assert!(shown.state.visible);
        assert_eq!((shown.state.look, shown.state.count, shown.state.size), ("suggest", 3, 22.0));
        // No suggestions: parked.
        let none = d.step(&input(conditions(0), field, false, &mons), false);
        assert_eq!(none.window, None);
        assert!(!none.state.visible);
        // The setting off: parked.
        let mut c = conditions(2);
        c.setting_on = false;
        assert_eq!(d.step(&input(c, field, false, &mons), false).window, None);
        // A rewrite running: the delegate look, even with no suggestions.
        let mut c = conditions(0);
        c.rewriting = true;
        assert_eq!(d.step(&input(c, field, false, &mons), false).state.look, "delegate");
    }

    #[test]
    fn moving_fields_hide_it_and_a_new_field_closes_the_popover() {
        let mons = main_monitor();
        let mut d = Driver::default();
        let field = Rect::new(400, 300, 900, 330);
        d.step(&input(conditions(1), field, true, &mons), false);
        // Scrolling: hidden while it moves, back after one still reading.
        let moved = Rect::new(400, 200, 900, 230);
        assert_eq!(d.step(&input(conditions(1), moved, false, &mons), true).window, None);
        assert!(d.step(&input(conditions(1), moved, false, &mons), true).state.open);
        // Another field: the popover closes and the page is told.
        let other = d.step(&input(conditions(1), Rect::new(400, 600, 900, 630), true, &mons), true);
        assert!(other.close && !other.state.open);
        // A 1 px wobble doesn't move it.
        let a = d.step(&input(conditions(1), Rect::new(400, 600, 900, 630), false, &mons), false);
        let b = d.step(&input(conditions(1), Rect::new(401, 600, 901, 631), false, &mons), false);
        assert_eq!(a.window, b.window);
    }

    // Bug 3 (owner's retest 2026-10-18): it never showed in classic
    // Outlook's mail body and glim.log said nothing. Each reading now says
    // why, in geometry and reasons only, for the log.
    #[test]
    fn each_reading_says_why_it_shows_or_hides() {
        let mons = main_monitor();
        let field = Rect::new(400, 300, 900, 330);
        let mut d = Driver::default();
        d.step(&input(conditions(2), field, true, &mons), false);
        assert_eq!(d.why(), Some(&Why::Shown(Placement { x: 904, y: 304, size: 22, monitor: 0 })));
        d.step(&input(conditions(0), field, false, &mons), false);
        assert_eq!(d.why(), Some(&Why::Hidden(Hide::NoSuggestions)));
        let mut c = conditions(1);
        c.app_foreground = false;
        d.step(&input(c, field, false, &mons), false);
        assert_eq!(d.why(), Some(&Why::Hidden(Hide::NotForeground)));
        d.step(&input(conditions(1), Rect::new(400, 200, 900, 230), false, &mons), false);
        assert_eq!(d.why(), Some(&Why::Moving));
        // Off every monitor: no place, with the rectangles it had.
        let off = Rect::new(5000, 300, 5500, 330);
        d.step(&input(conditions(1), off, true, &mons), false);
        let Some(Why::NoPlace(NoPlace::OffScreen, f, c)) = d.why() else { panic!("{:?}", d.why()) };
        assert_eq!((*f, c.map(|c| c.left)), (Some(off), Some(5100)));
        // The log line: reasons and numbers, never text.
        let line = log_line(d.why().unwrap(), &mons);
        assert_eq!(line, "bubble: no place (OffScreen) field=(5000,300,5500,330) caret=(5100,306,5101,324) monitors=[(0,0,1920,1040)@1, (-1920,-58,0,1142)@1.25]");
        assert_eq!(log_line(&Why::Hidden(Hide::NotRead), &mons), "bubble: hidden (NotRead)");
        // Logged when the reason changes, not on every caret move.
        let at = |x| Why::Shown(Placement { x, y: 304, size: 22, monitor: 0 });
        assert!(at(904).same_reason(&at(950)));
        assert!(!Why::Hidden(Hide::NotRead).same_reason(&Why::Hidden(Hide::Paused)));
        assert!(Why::NoPlace(NoPlace::OffScreen, None, None).same_reason(&Why::NoPlace(NoPlace::OffScreen, Some(off), None)));
        assert!(!at(904).same_reason(&Why::Moving));
        assert_eq!(log_line(&Why::Shown(Placement { x: 904, y: 304, size: 22, monitor: 0 }), &mons), "bubble: shown at (904,304) size 22 on monitor 0");
    }

    /// §7a.4: never takes focus.
    #[test]
    fn the_window_never_takes_focus() {
        let src = include_str!("bubble_window.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for needed in [".focusable(false)", ".focused(false)", ".always_on_top(true)", ".skip_taskbar(true)", "crate::browser_args()", "make_non_activating(&win)"] {
            assert!(code.contains(needed), "{needed}");
        }
        for banned in ["set_focus", ".show()", ".hide()", "set_focusable(true)"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }
}
