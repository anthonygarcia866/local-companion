// The `window` action (owner, 2026-10-28): "put Outlook on the left",
// "maximize Chrome", "move it to the other screen". Ask Glim's typed tool
// `window { app, where, display? }`; also a learned shortcut's and a routine
// step's action. The geometry here is pure (unit tested); Windows does the
// moving (platform::arrange_window: SetWindowPos / ShowWindow on the app's
// own top-level window, per-monitor work area, DPI-aware).
//
// Never a password manager's window, never Glim's own: refused with a card.

/// Where the window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Left,
    Right,
    Maximize,
    Minimize,
    Restore,
    Center,
}

impl Where {
    pub const ALL: [&'static str; 6] = ["left", "right", "maximize", "minimize", "restore", "center"];

    pub fn parse(s: &str) -> Option<Where> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Where::Left),
            "right" => Some(Where::Right),
            "maximize" => Some(Where::Maximize),
            "minimize" => Some(Where::Minimize),
            "restore" => Some(Where::Restore),
            "center" => Some(Where::Center),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Where::Left => "left",
            Where::Right => "right",
            Where::Maximize => "maximize",
            Where::Minimize => "minimize",
            Where::Restore => "restore",
            Where::Center => "center",
        }
    }
}

/// Which screen: the one the window is on (default), or another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    This,
    Other,
}

impl Display {
    pub fn parse(s: &str) -> Option<Display> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "this" => Some(Display::This),
            "other" => Some(Display::Other),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Display::This => "this",
            Display::Other => "other",
        }
    }
}

/// A rectangle in physical (per-monitor-DPI) pixels, as Win32 gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn contains(&self, (px, py): (i32, i32)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    fn overlap(&self, o: &Rect) -> i64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0 && h > 0 { w as i64 * h as i64 } else { 0 }
    }
}

/// How far the window's rectangle reaches past what is drawn (Windows 10/11's
/// invisible resize borders, DwmGetWindowAttribute's extended frame): added
/// back so the visible edges meet the work area's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Margins {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The monitor the window is mostly on: the one it overlaps most, else the
/// one holding its center, else the first.
pub fn monitor_of(works: &[Rect], window: &Rect) -> usize {
    let best = works.iter().enumerate().map(|(i, m)| (m.overlap(window), i)).max_by_key(|(o, i)| (*o, std::cmp::Reverse(*i)));
    match best {
        Some((o, i)) if o > 0 => i,
        _ => works.iter().position(|m| m.contains(window.center())).unwrap_or(0),
    }
}

/// The work area to use: the window's own monitor, or (Display::Other) the
/// next one in order. None when there is no other monitor.
pub fn pick(works: &[Rect], window: &Rect, display: Display) -> Option<usize> {
    if works.is_empty() {
        return None;
    }
    let mine = monitor_of(works, window);
    match display {
        Display::This => Some(mine),
        Display::Other if works.len() < 2 => None,
        Display::Other => Some((mine + 1) % works.len()),
    }
}

/// The window rectangle for left / right / center on `work` (None for the
/// others: Windows maximizes, minimizes and restores itself). Halves split
/// an odd width with the extra pixel on the right. `window` is the window's
/// current rectangle (center keeps its size, shrunk to fit).
pub fn target(at: Where, work: &Rect, window: &Rect, m: &Margins) -> Option<Rect> {
    let visible = match at {
        Where::Left => Rect { x: work.x, y: work.y, w: work.w / 2, h: work.h },
        Where::Right => Rect { x: work.x + work.w / 2, y: work.y, w: work.w - work.w / 2, h: work.h },
        Where::Center => {
            let w = (window.w - m.left - m.right).clamp(1, work.w);
            let h = (window.h - m.top - m.bottom).clamp(1, work.h);
            Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h }
        }
        Where::Maximize | Where::Minimize | Where::Restore => return None,
    };
    Some(Rect { x: visible.x - m.left, y: visible.y - m.top, w: visible.w + m.left + m.right, h: visible.h + m.top + m.bottom })
}

/// Moving a window to another screen for maximize: where to put it first
/// (restored, its size kept within the target's work area, centered), so
/// Windows maximizes it there.
pub fn onto(work: &Rect, window: &Rect) -> Rect {
    let w = window.w.clamp(1, work.w);
    let h = window.h.clamp(1, work.h);
    Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h }
}

/// How a window ended up, read back after moving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub rect: Rect,
    pub maximized: bool,
    pub minimized: bool,
    /// The monitor it is on now (monitor_of).
    pub monitor: usize,
}

/// Whether the window is where it was asked to go (the check before a
/// window call is learned, and for the card). A few pixels' slack: some
/// apps round their size to a character grid.
pub fn placed(at: Where, wanted: Option<Rect>, monitor: usize, s: &State) -> bool {
    const SLACK: i32 = 16;
    let near = |a: i32, b: i32| (a - b).abs() <= SLACK;
    match at {
        Where::Maximize => s.maximized && s.monitor == monitor,
        Where::Minimize => s.minimized,
        Where::Restore => !s.maximized && !s.minimized,
        Where::Left | Where::Right | Where::Center => {
            let Some(w) = wanted else { return false };
            !s.maximized && !s.minimized && near(s.rect.x, w.x) && near(s.rect.y, w.y) && near(s.rect.w, w.w) && near(s.rect.h, w.h)
        }
    }
}

/// Password managers: Glim never moves, shows or hides their windows (their
/// file names, lower case).
pub const PASSWORD_MANAGERS: [&str; 12] = [
    "1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe", "lastpass.exe", "dashlane.exe", "keeper.exe",
    "keeperpasswordmanager.exe", "nordpass.exe", "roboform.exe", "enpass.exe", "proton pass.exe",
];

/// Why Glim won't touch a window of `exe` (None: it may). Glim's own windows
/// are refused by process id in platform::arrange_window as well.
pub fn refused(exe: &str) -> Option<&'static str> {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase();
    if PASSWORD_MANAGERS.contains(&file.as_str()) {
        return Some("window-refused");
    }
    if file == "glim.exe" || file == "msedgewebview2.exe" {
        return Some("window-refused");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 0, y: 0, w: 0, h: 0 };
    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn words_parse_and_round_trip() {
        for s in Where::ALL {
            assert_eq!(Where::parse(s).unwrap().as_str(), s);
        }
        assert_eq!(Where::parse(" Left "), Some(Where::Left));
        assert_eq!(Where::parse("top"), None);
        assert_eq!(Display::parse(""), Some(Display::This));
        assert_eq!(Display::parse("other"), Some(Display::Other));
        assert_eq!(Display::parse("left"), None);
    }

    /// One 1920x1080 monitor at 100%, the taskbar at the bottom (40 px).
    #[test]
    fn one_monitor_halves_and_center() {
        let work = [r(0, 0, 1920, 1040)];
        let win = r(300, 200, 800, 600);
        let none = Margins::default();
        assert_eq!(pick(&work, &win, Display::This), Some(0));
        assert_eq!(pick(&work, &win, Display::Other), None, "no other screen");
        assert_eq!(target(Where::Left, &work[0], &win, &none), Some(r(0, 0, 960, 1040)));
        assert_eq!(target(Where::Right, &work[0], &win, &none), Some(r(960, 0, 960, 1040)));
        assert_eq!(target(Where::Center, &work[0], &win, &none), Some(r(560, 220, 800, 600)));
        assert_eq!(target(Where::Maximize, &work[0], &win, &none), None);
        // Windows 11's invisible borders (7 px left/right/bottom at 100%):
        // the drawn edges meet the work area's.
        let m = Margins { left: 7, top: 0, right: 7, bottom: 7 };
        assert_eq!(target(Where::Left, &work[0], &win, &m), Some(r(-7, 0, 974, 1047)));
        // An odd width: the extra pixel goes right.
        let odd = r(0, 0, 1921, 1040);
        assert_eq!(target(Where::Left, &odd, &win, &none).unwrap().w + target(Where::Right, &odd, &win, &none).unwrap().w, 1921);
        // Center shrinks a window bigger than the screen to fit.
        assert_eq!(target(Where::Center, &work[0], &r(0, 0, 3000, 2000), &none), Some(r(0, 0, 1920, 1040)));
    }

    /// Two monitors: a laptop at 125% (2560x1600 physical, taskbar 60 px)
    /// and a 1920x1080 at 100% to its right. Physical pixels throughout, as
    /// a per-monitor-DPI-aware process sees them.
    #[test]
    fn two_monitors_at_125_percent_other_display() {
        let works = [r(0, 0, 2560, 1540), r(2560, 0, 1920, 1040)];
        let none = Margins::default();
        let on_laptop = r(100, 100, 1200, 900);
        let on_second = r(3000, 100, 800, 600);
        assert_eq!(monitor_of(&works, &on_laptop), 0);
        assert_eq!(monitor_of(&works, &on_second), 1);
        // Straddling: the monitor it overlaps most.
        assert_eq!(monitor_of(&works, &r(2300, 100, 800, 600)), 1);
        assert_eq!(pick(&works, &on_laptop, Display::Other), Some(1));
        assert_eq!(pick(&works, &on_second, Display::Other), Some(0));
        assert_eq!(target(Where::Right, &works[1], &on_laptop, &none), Some(r(3520, 0, 960, 1040)));
        assert_eq!(target(Where::Left, &works[0], &on_second, &none), Some(r(0, 0, 1280, 1540)));
        // At 125% the invisible border is 9 px: the same formula.
        let m = Margins { left: 9, top: 0, right: 9, bottom: 9 };
        assert_eq!(target(Where::Right, &works[0], &on_laptop, &m), Some(r(1271, 0, 1298, 1549)));
        // Maximize on the other screen: restored there first, its size kept
        // within that screen, centered.
        assert_eq!(onto(&works[1], &on_laptop), r(2560 + 360, 70, 1200, 900));
        assert_eq!(onto(&works[1], &r(0, 0, 2500, 1500)), r(2560, 0, 1920, 1040));
        // A window off every screen counts as the first one's.
        assert_eq!(monitor_of(&works, &r(-5000, -5000, 10, 10)), 0);
        assert_eq!(pick(&[], &on_laptop, Display::This), None);
    }

    #[test]
    fn placed_checks_what_was_asked() {
        let want = Some(r(0, 0, 960, 1040));
        let st = |rect, maximized, minimized, monitor| State { rect, maximized, minimized, monitor };
        assert!(placed(Where::Left, want, 0, &st(r(4, 0, 952, 1040), false, false, 0)), "a few px of slack");
        assert!(!placed(Where::Left, want, 0, &st(r(300, 200, 800, 600), false, false, 0)), "didn't move");
        assert!(!placed(Where::Left, want, 0, &st(r(0, 0, 960, 1040), true, false, 0)), "maximized isn't left");
        assert!(placed(Where::Maximize, None, 1, &st(R, true, false, 1)));
        assert!(!placed(Where::Maximize, None, 1, &st(R, true, false, 0)), "maximized on the wrong screen");
        assert!(placed(Where::Minimize, None, 0, &st(R, false, true, 0)));
        assert!(placed(Where::Restore, None, 0, &st(R, false, false, 0)));
        assert!(!placed(Where::Center, None, 0, &st(R, false, false, 0)));
    }

    #[test]
    fn password_managers_and_glim_are_never_moved() {
        assert_eq!(refused(r"C:\Program Files\1Password\app\8\1Password.exe"), Some("window-refused"));
        assert_eq!(refused("KeePassXC.exe"), Some("window-refused"));
        assert_eq!(refused("Bitwarden.exe"), Some("window-refused"));
        assert_eq!(refused(r"C:\Users\me\AppData\Local\Glim\glim.exe"), Some("window-refused"));
        assert_eq!(refused("OUTLOOK.EXE"), None);
        assert_eq!(refused("chrome.exe"), None);
    }
}
