// Where on a display the island's window goes: the dock (top centre, a top
// corner, or standing upright along the left or right edge) and the window's
// shape (the full panel, the invisible wake strip, or the ember dot). Pure
// geometry in logical pixels; island.rs turns it into physical placement.
//
// Docks are remembered per display (settings.docks, keyed like `at:` display
// preferences). Dragging the pill drops it on the nearest dock.

use crate::island::{PANEL_H, PANEL_W, STRIP_H, STRIP_W};

/// The panel's height when docked upright along an edge: the large pill
/// (352 px long) stands in it, and the island opens sideways inside it.
pub const PANEL_V_H: f64 = 400.0;
/// The ember: a small glowing dot at the dock's edge.
pub const EMBER: f64 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    TopCenter,
    TopLeft,
    TopRight,
    Left,
    Right,
}

pub const DOCKS: [Dock; 5] = [Dock::TopCenter, Dock::TopLeft, Dock::TopRight, Dock::Left, Dock::Right];

impl Dock {
    /// The stored name. `--dock` also takes `left-vertical` / `right-vertical`.
    pub fn as_str(self) -> &'static str {
        match self {
            Dock::TopCenter => "top-center",
            Dock::TopLeft => "top-left",
            Dock::TopRight => "top-right",
            Dock::Left => "left",
            Dock::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Dock> {
        match s.trim() {
            "top-center" | "top" => Some(Dock::TopCenter),
            "top-left" => Some(Dock::TopLeft),
            "top-right" => Some(Dock::TopRight),
            "left" | "left-vertical" => Some(Dock::Left),
            "right" | "right-vertical" => Some(Dock::Right),
            _ => None,
        }
    }

    /// Upright along an edge: the pill stands vertically, the island opens sideways.
    pub fn vertical(self) -> bool {
        matches!(self, Dock::Left | Dock::Right)
    }

    /// The point on a `w`×`h` display a drop is measured to.
    fn anchor(self, w: f64, h: f64) -> (f64, f64) {
        match self {
            Dock::TopCenter => (w / 2.0, 0.0),
            Dock::TopLeft => (0.0, 0.0),
            Dock::TopRight => (w, 0.0),
            Dock::Left => (0.0, h / 2.0),
            Dock::Right => (w, h / 2.0),
        }
    }
}

/// The dock nearest to a drop at (`x`, `y`), logical pixels from the display's
/// top-left corner, on a `w`×`h` display.
pub fn nearest_dock(x: f64, y: f64, w: f64, h: f64) -> Dock {
    let dist = |d: &Dock| {
        let (ax, ay) = d.anchor(w, h);
        (ax - x).powi(2) + (ay - y).powi(2)
    };
    DOCKS.into_iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap_or(Dock::TopCenter)
}

/// What the island's window is at the moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The full panel the island is drawn in.
    Panel,
    /// The invisible strip that wakes a hidden island.
    Strip,
    /// The ember dot (and the recording indicator).
    Ember,
}

/// The window's logical size for a dock and shape.
pub fn window_size(dock: Dock, shape: Shape) -> (f64, f64) {
    match (shape, dock.vertical()) {
        (Shape::Panel, false) => (PANEL_W, PANEL_H),
        (Shape::Panel, true) => (PANEL_W, PANEL_V_H),
        (Shape::Strip, false) => (STRIP_W, STRIP_H),
        (Shape::Strip, true) => (STRIP_H, STRIP_W),
        (Shape::Ember, _) => (EMBER, EMBER),
    }
}

/// The window's logical rectangle (x, y, w, h) relative to the top-left corner
/// of a `mw`×`mh` display.
pub fn window_rect(dock: Dock, shape: Shape, mw: f64, mh: f64) -> (f64, f64, f64, f64) {
    let (w, h) = window_size(dock, shape);
    let (x, y) = match dock {
        Dock::TopCenter => ((mw - w) / 2.0, 0.0),
        Dock::TopLeft => (0.0, 0.0),
        Dock::TopRight => (mw - w, 0.0),
        Dock::Left => (0.0, (mh - h) / 2.0),
        Dock::Right => (mw - w, (mh - h) / 2.0),
    };
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_the_dev_spellings_parse() {
        for d in DOCKS {
            assert_eq!(Dock::parse(d.as_str()), Some(d));
        }
        assert_eq!(Dock::parse("left-vertical"), Some(Dock::Left));
        assert_eq!(Dock::parse("right-vertical"), Some(Dock::Right));
        assert_eq!(Dock::parse("bottom"), None);
    }

    #[test]
    fn a_drop_snaps_to_the_nearest_dock() {
        let (w, h) = (1920.0, 1080.0);
        assert_eq!(nearest_dock(900.0, 40.0, w, h), Dock::TopCenter);
        assert_eq!(nearest_dock(60.0, 30.0, w, h), Dock::TopLeft);
        assert_eq!(nearest_dock(1880.0, 60.0, w, h), Dock::TopRight);
        assert_eq!(nearest_dock(30.0, 600.0, w, h), Dock::Left);
        assert_eq!(nearest_dock(1900.0, 500.0, w, h), Dock::Right);
        // The middle of the screen is nearest the top centre.
        assert_eq!(nearest_dock(960.0, 400.0, w, h), Dock::TopCenter);
    }

    #[test]
    fn every_dock_keeps_the_window_on_the_display() {
        for (mw, mh) in [(1920.0, 1080.0), (1536.0, 960.0), (1280.0, 720.0)] {
            for d in DOCKS {
                for s in [Shape::Panel, Shape::Strip, Shape::Ember] {
                    let (x, y, w, h) = window_rect(d, s, mw, mh);
                    assert!(x >= 0.0 && y >= 0.0 && x + w <= mw && y + h <= mh, "{d:?} {s:?} on {mw}x{mh}");
                }
            }
        }
    }

    #[test]
    fn edges_are_hugged() {
        let (mw, mh) = (1920.0, 1080.0);
        assert_eq!(window_rect(Dock::TopCenter, Shape::Panel, mw, mh), (600.0, 0.0, 720.0, 320.0));
        assert_eq!(window_rect(Dock::Left, Shape::Panel, mw, mh), (0.0, 340.0, 720.0, 400.0));
        assert_eq!(window_rect(Dock::Right, Shape::Strip, mw, mh), (1914.0, 420.0, 6.0, 240.0));
        assert_eq!(window_rect(Dock::TopRight, Shape::Ember, mw, mh), (1892.0, 0.0, 28.0, 28.0));
    }
}
