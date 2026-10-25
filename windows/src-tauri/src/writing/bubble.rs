// The mini lantern beside the text field (1e, docs/phase1-plan.md §7a): the
// logic only. Where it goes (`place`), whether it shows (`shown`), and
// whether the field is holding still (`Tracker`). No window is created here
// and nothing calls this yet: the window waits for the owner's live session.
//
// Decided 2026-10-13 (owner): outside the right edge of a single-line field,
// inside only at a monitor edge; 22 logical px; off by default
// (`settings.writing_bubble`); a tall field (cut off by the monitor, or
// taller than half its work area) keeps it level with the caret's line.
//
// Amended 2026-10-18 (owner's retest: in Outlook's body it sat in the far
// bottom-right corner): every multi-line field with a caret (Outlook and
// Word bodies, short or tall) gets it level with the caret's line, just
// right of where that line's text ends, inside the visible part, following
// the caret. The corner stays only for a multi-line field without a caret.
//
// Everything is in physical screen pixels, as UI Automation reports them;
// sizes and gaps are logical pixels times the monitor's scale.
//
// Amended from §7a.1 (2026-10-13): the plan hides the lantern when the field
// is "more than half outside its monitor". A long Gmail or Word body can be
// taller than the screen, so that would hide it exactly where it's wanted.
// Instead the field is clipped to the monitor: hidden when nothing of it is
// on screen, or when the caret is scrolled out of the visible part.
#![allow(dead_code)] // the window (1e's next step) calls it

/// A rectangle in physical screen pixels; right and bottom are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }
    fn center(&self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }
    fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(self.left.max(o.left), self.top.max(o.top), self.right.min(o.right), self.bottom.min(o.bottom))
    }
    fn overlaps(&self, o: &Rect) -> bool {
        !self.intersect(o).is_empty()
    }
    fn grow(&self, by: i32) -> Rect {
        Rect::new(self.left - by, self.top - by, self.right + by, self.bottom + by)
    }
    /// The furthest any edge moved.
    fn moved_from(&self, o: &Rect) -> i32 {
        [self.left - o.left, self.top - o.top, self.right - o.right, self.bottom - o.bottom].iter().map(|d| d.abs()).max().unwrap_or(0)
    }
}

/// One display: its work area (physical pixels, taskbar excluded) and its
/// scale (1.0 = 96 dpi, 1.25 = 120 dpi …, from GetDpiForMonitor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub work: Rect,
    pub scale: f64,
}

/// The lantern's height in logical pixels (decided 2026-10-13).
pub const SIZE: f64 = 22.0;
/// Gap between a single-line field's right edge and the lantern.
const GAP: f64 = 4.0;
/// Inset from a field's edge when the lantern sits inside it.
const INSET: f64 = 6.0;
/// The caret's rectangle, grown by this, is never covered.
const CARET_CLEAR: f64 = 4.0;
/// Without a caret, a field this tall or less is single-line.
const SINGLE_LINE_MAX: f64 = 40.0;
/// Without a field, the lantern sits this far right of the caret.
const CARET_GAP: f64 = 8.0;
/// Moves of this many pixels or fewer don't move the lantern.
pub const JITTER: i32 = 2;

fn px(logical: f64, scale: f64) -> i32 {
    (logical * scale).round() as i32
}

/// Where the lantern goes: its top-left corner and its size, in physical
/// pixels, and which monitor (index into `monitors`) it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub size: i32,
    pub monitor: usize,
}

impl Placement {
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.size, self.y + self.size)
    }
}

/// Why it isn't placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPlace {
    /// Neither rectangle (some Chromium fields report none, or 0×0).
    NoAnchor,
    /// Not on any monitor, nothing of the field on screen, or the caret
    /// scrolled out of the field's visible part.
    OffScreen,
    /// Every place would cover the caret.
    CoversCaret,
}

/// The lantern's place for a field and its caret (§7a.1), without the
/// caret line's rectangle.
pub fn place(field: Option<Rect>, caret: Option<Rect>, monitors: &[Monitor]) -> Result<Placement, NoPlace> {
    place_on_line(field, caret, None, monitors)
}

/// The lantern's place for a field, its caret and the caret's line (the
/// line's rectangle from UI Automation, when the app gives one: its right
/// edge is where the line's text ends).
pub fn place_on_line(field: Option<Rect>, caret: Option<Rect>, caret_line: Option<Rect>, monitors: &[Monitor]) -> Result<Placement, NoPlace> {
    let field = field.filter(|r| !r.is_empty());
    // A collapsed caret is often 0 or 1 px wide: only its height must be real.
    let caret = caret.filter(|r| r.height() > 0 && r.width() >= 0);
    let anchor = match (field, caret) {
        (Some(f), _) => f.center(),
        (None, Some(c)) => c.center(),
        (None, None) => return Err(NoPlace::NoAnchor),
    };
    // The caret decides the monitor when it's on screen: a field can span two.
    let point = caret.map(|c| c.center()).filter(|p| monitors.iter().any(|m| m.work.contains(*p))).unwrap_or(anchor);
    let (index, mon) = monitors.iter().enumerate().find(|(_, m)| m.work.contains(point)).ok_or(NoPlace::OffScreen)?;
    let s = mon.scale;
    let size = px(SIZE, s);
    let work = mon.work;

    let (x, y) = match field {
        Some(f) => {
            let vis = f.intersect(&work);
            if vis.is_empty() {
                return Err(NoPlace::OffScreen);
            }
            if let Some(c) = caret {
                if !vis.contains(c.center()) && !(c.width() == 0 && vis.grow(1).contains(c.center())) {
                    return Err(NoPlace::OffScreen);
                }
            }
            let line = caret.map_or(px(SINGLE_LINE_MAX, s) * 2 / 5, |c| c.height());
            let single = match caret {
                Some(_) => (vis.height() as f64) < 2.5 * line as f64,
                None => vis.height() <= px(SINGLE_LINE_MAX, s),
            };
            if single {
                let outside = vis.right + px(GAP, s);
                let y = vis.top + vis.height() / 2 - size / 2;
                if outside + size <= work.right {
                    (outside, y)
                } else {
                    (vis.right - px(INSET, s) - size, y)
                }
            } else if let Some(c) = caret {
                // Owner, 2026-10-18 (Outlook and Word bodies): level with the
                // caret's line, just right of where the line's text ends (the
                // caret itself when the app gives no line), kept inside the
                // visible part, so it follows the line being typed. Touching
                // the caret: just above its line, or just below; neither fits
                // in the visible part: not shown.
                let mid = c.top + c.height() / 2;
                let end = caret_line.filter(|l| l.top <= mid && mid < l.bottom && l.right >= c.right).map_or(c.right, |l| l.right);
                let x = (end + px(GAP, s)).min(vis.right - px(INSET, s) - size).max(vis.left);
                let clear = c.grow(px(CARET_CLEAR, s));
                let mut y = (mid - size / 2).clamp(vis.top, (vis.bottom - size).max(vis.top));
                if Rect::new(x, y, x + size, y + size).overlaps(&clear) {
                    y = if clear.top - size >= vis.top {
                        clear.top - size
                    } else if clear.bottom + size <= vis.bottom {
                        clear.bottom
                    } else {
                        return Err(NoPlace::CoversCaret);
                    };
                }
                (x, y)
            } else {
                // No caret: the visible part's bottom-right corner.
                let x = vis.right - px(INSET, s) - size;
                let y = vis.bottom - px(INSET, s) - size;
                (x, y.max(vis.top))
            }
        }
        None => {
            let c = caret.ok_or(NoPlace::NoAnchor)?;
            (c.right + px(CARET_GAP, s), c.top + c.height() / 2 - size / 2)
        }
    };
    // Kept on the monitor.
    let x = x.clamp(work.left, (work.right - size).max(work.left));
    let y = y.clamp(work.top, (work.bottom - size).max(work.top));
    let placed = Placement { x, y, size, monitor: index };
    if let Some(c) = caret {
        if placed.rect().overlaps(&c.grow(px(CARET_CLEAR, s))) {
            return Err(NoPlace::CoversCaret);
        }
    }
    Ok(placed)
}

/// Whether a new placement should move the lantern: only past JITTER, so a
/// rectangle that wobbles by a pixel doesn't make it twitch.
pub fn should_move(from: &Placement, to: &Placement) -> bool {
    from.monitor != to.monitor || from.size != to.size || from.rect().moved_from(&to.rect()) > JITTER
}

/// Holds the field's last rectangle: while it moves (scrolling, a window
/// being dragged) the lantern hides, and it shows again after one reading
/// where the field held still (§7a.1).
#[derive(Default, Debug)]
pub struct Tracker {
    last: Option<Rect>,
}

impl Tracker {
    /// A new reading. True when the field is steady: the first reading of a
    /// field, or one within JITTER of the last.
    pub fn steady(&mut self, field: Option<Rect>) -> bool {
        let steady = match (self.last, field) {
            (Some(a), Some(b)) => a.moved_from(&b) <= JITTER,
            _ => true,
        };
        self.last = field;
        steady
    }

    /// Focus moved to another field: its first reading counts as steady.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Everything §7a.5 checks on each reading. Each `true` here is what lets it
/// show; the `Default` hides it (no setting, no suggestions).
#[derive(Clone, Copy, Debug, Default)]
pub struct Conditions {
    /// Settings → Writing → the mini lantern (`writing_bubble`).
    pub setting_on: bool,
    /// The checker's suggestions for this field.
    pub suggestions: usize,
    /// A 1c rewrite of this field is on its way (s-delegate).
    pub rewriting: bool,
    pub editable: bool,
    pub password: bool,
    /// Capture read this field: false for what PR #8 skips (sensitive
    /// labels, payment fields, withheld readings).
    pub read: bool,
    /// The app is switched on in Settings → Writing → Apps (1d §14.1).
    pub app_on: bool,
    /// The app or site is on the pause list (§14.5).
    pub app_paused: bool,
    /// Glim is paused (Ctrl+Alt+Shift+P, §14.6).
    pub paused: bool,
    /// Presence is Hidden.
    pub hidden: bool,
    /// A fullscreen app is in front on that monitor.
    pub fullscreen: bool,
    pub field_focused: bool,
    /// The field's app owns the foreground window.
    pub app_foreground: bool,
}

/// Why it is hidden; the first that applies, privacy first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hide {
    SettingOff,
    Password,
    NotRead,
    ReadOnly,
    AppPaused,
    AppOff,
    Paused,
    Hidden,
    Fullscreen,
    NoFocus,
    NotForeground,
    NoSuggestions,
}

/// §7a.5: shown only when every condition holds.
pub fn shown(c: &Conditions) -> Result<(), Hide> {
    let checks = [
        (c.setting_on, Hide::SettingOff),
        (!c.password, Hide::Password),
        (c.read, Hide::NotRead),
        (c.editable, Hide::ReadOnly),
        (!c.app_paused, Hide::AppPaused),
        (c.app_on, Hide::AppOff),
        (!c.paused, Hide::Paused),
        (!c.hidden, Hide::Hidden),
        (!c.fullscreen, Hide::Fullscreen),
        (c.field_focused, Hide::NoFocus),
        (c.app_foreground, Hide::NotForeground),
        (c.suggestions > 0 || c.rewriting, Hide::NoSuggestions),
    ];
    checks.iter().find(|(ok, _)| !ok).map_or(Ok(()), |(_, why)| Err(*why))
}

/// The lantern's state while it shows: the rewrite (data leaving the
/// machine) wins over the count (§7a.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// `s-suggest` with the count badge ("9+" past nine).
    Suggest(usize),
    /// `s-delegate`.
    Delegate,
}

pub fn look(c: &Conditions) -> Look {
    if c.rewriting { Look::Delegate } else { Look::Suggest(c.suggestions) }
}

/// The badge text, as the pill's.
pub fn badge(count: usize) -> String {
    if count > 9 { "9+".into() } else { count.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The owner's displays (glim.log, 2026-10-13): DISPLAY3 1920×1080 at
    // 0,0 (100%), DISPLAY2 at 1920,0 (100%), DISPLAY1 1536×960 at −1536,−46
    // (125%). Work areas leave 40 px for the taskbar on the main one.
    fn owner() -> Vec<Monitor> {
        vec![
            Monitor { work: Rect::new(0, 0, 1920, 1040), scale: 1.0 },
            Monitor { work: Rect::new(1920, 0, 3840, 1080), scale: 1.0 },
            Monitor { work: Rect::new(-1920, -58, 0, 1142), scale: 1.25 },
        ]
    }

    fn caret_at(x: i32, top: i32, h: i32) -> Rect {
        Rect::new(x, top, x + 1, top + h)
    }

    #[test]
    fn a_single_line_field_gets_it_outside_its_right_edge() {
        let field = Rect::new(400, 300, 900, 330);
        let p = place(Some(field), Some(caret_at(500, 306, 18)), &owner()).unwrap();
        assert_eq!((p.x, p.size, p.monitor), (904, 22, 0));
        assert_eq!(p.y, 315 - 11, "vertically centred on the field");
        assert!(!p.rect().overlaps(&field), "outside: it never covers the text");
    }

    #[test]
    fn at_a_monitor_edge_it_goes_inside_the_field() {
        let field = Rect::new(1500, 300, 1915, 330); // ends 5 px from the right edge
        let p = place(Some(field), Some(caret_at(1600, 306, 18)), &owner()).unwrap();
        assert_eq!(p.x, 1915 - 6 - 22);
        assert!(p.rect().right <= 1920);
    }

    /// Owner, 2026-10-18: in Outlook's body (short, fully on screen) it sat
    /// in the far bottom-right corner. Now level with the caret's line, just
    /// right of the line's end.
    #[test]
    fn a_multi_line_field_gets_it_just_right_of_the_caret_lines_end() {
        let body = Rect::new(200, 200, 1200, 600);
        let c = caret_at(500, 250, 18);
        // "We musn't|": the line's text ends at the caret.
        let line = Rect::new(210, 250, 501, 268);
        let p = place_on_line(Some(body), Some(c), Some(line), &owner()).unwrap();
        assert_eq!((p.x, p.y), (501 + 4, 259 - 11), "just right of the line, level with it");
        assert!(!p.rect().overlaps(&c.grow(4)));
        // The caret mid-line: after the line's end, not over the words after the caret.
        let p = place_on_line(Some(body), Some(caret_at(300, 250, 18)), Some(Rect::new(210, 250, 820, 268)), &owner()).unwrap();
        assert_eq!(p.x, 820 + 4);
        // No line from the app: right of the caret.
        let p = place(Some(body), Some(c), &owner()).unwrap();
        assert_eq!((p.x, p.y), (501 + 4, 259 - 11));
        // A line rectangle that isn't the caret's line (another row) is ignored.
        let p = place_on_line(Some(body), Some(c), Some(Rect::new(210, 400, 900, 418)), &owner()).unwrap();
        assert_eq!(p.x, 501 + 4);
    }

    #[test]
    fn it_follows_the_caret_as_you_type_and_move_down() {
        let body = Rect::new(200, 200, 1200, 600);
        let mut last = None;
        for (x, top) in [(300, 250), (310, 250), (320, 250), (220, 268), (400, 286)] {
            let c = caret_at(x, top, 18);
            let p = place_on_line(Some(body), Some(c), Some(Rect::new(210, top, x + 1, top + 18)), &owner()).unwrap();
            assert_eq!((p.x, p.y), (x + 1 + 4, top + 9 - 11), "caret at ({x},{top})");
            if let Some(prev) = last {
                assert!(should_move(&prev, &p), "it moves with the caret");
            }
            last = Some(p);
        }
    }

    #[test]
    fn a_line_ending_at_the_right_edge_puts_it_above_or_below_the_line() {
        let body = Rect::new(200, 200, 1200, 600);
        // The line runs to the right edge and the caret is at its end.
        let c = caret_at(1185, 400, 18);
        let p = place_on_line(Some(body), Some(c), Some(Rect::new(210, 400, 1186, 418)), &owner()).unwrap();
        assert_eq!(p.x, 1200 - 6 - 22, "kept inside the visible part");
        assert_eq!(p.rect().bottom, c.top - 4, "just above the caret's line");
        // On the first line: just below it.
        let c = caret_at(1185, 202, 18);
        let p = place_on_line(Some(body), Some(c), Some(Rect::new(210, 202, 1186, 220)), &owner()).unwrap();
        assert_eq!(p.y, c.bottom + 4);
        for caret in [caret_at(1170, 572, 18), caret_at(1150, 560, 18), caret_at(1185, 575, 20), caret_at(1160, 578, 18)] {
            let p = place(Some(body), Some(caret), &owner()).unwrap();
            assert!(!p.rect().overlaps(&caret.grow(4)), "{caret:?} → {p:?}");
            assert!(p.rect().bottom <= 600 && p.rect().right <= 1200, "inside the body: {caret:?} → {p:?}");
        }
    }

    /// Word's document body: a page taller than the screen, cut off at the
    /// bottom, the caret's line from Word's own text range.
    #[test]
    fn in_a_word_body_it_sits_beside_the_line_being_typed() {
        let page = Rect::new(400, 150, 1500, 2200);
        let c = caret_at(760, 700, 20);
        let p = place_on_line(Some(page), Some(c), Some(Rect::new(480, 700, 761, 720)), &owner()).unwrap();
        assert_eq!((p.x, p.y, p.monitor), (761 + 4, 710 - 11, 0));
        // Scrolled so the caret's line is near the bottom of the screen: still level with it, inside the screen.
        let c = caret_at(760, 1010, 20);
        let p = place_on_line(Some(page), Some(c), Some(Rect::new(480, 1010, 761, 1030)), &owner()).unwrap();
        assert!(p.rect().bottom <= 1040, "{p:?}");
        assert!(!p.rect().overlaps(&c.grow(4)));
        // On the 125% monitor: sizes and gaps scale.
        let page = Rect::new(-1800, 0, -400, 2000);
        let c = caret_at(-1500, 300, 25);
        let p = place_on_line(Some(page), Some(c), Some(Rect::new(-1750, 300, -1499, 325)), &owner()).unwrap();
        assert_eq!((p.size, p.x, p.monitor), (28, -1499 + 5, 2));
    }

    #[test]
    fn without_a_field_rectangle_it_follows_the_caret_and_without_either_it_hides() {
        let c = caret_at(700, 400, 20);
        let p = place(None, Some(c), &owner()).unwrap();
        assert_eq!((p.x, p.y), (701 + 8, 410 - 11));
        assert_eq!(place(Some(Rect::new(10, 10, 10, 10)), None, &owner()), Err(NoPlace::NoAnchor), "0×0 counts as none");
        assert_eq!(place(None, None, &owner()), Err(NoPlace::NoAnchor));
    }

    #[test]
    fn off_screen_fields_and_scrolled_away_carets_hide_it() {
        // Entirely below every monitor.
        assert_eq!(place(Some(Rect::new(100, 2000, 600, 2030)), None, &owner()), Err(NoPlace::OffScreen));
        // A body taller than the screen still gets it, level with the caret's line.
        let tall = Rect::new(200, -500, 1200, 3000);
        let p = place(Some(tall), Some(caret_at(300, 500, 18)), &owner()).unwrap();
        assert_eq!((p.x, p.y), (301 + 4, 509 - 11));
        // The caret scrolled out of the visible part: hidden.
        assert_eq!(place(Some(tall), Some(caret_at(300, 1500, 18)), &owner()), Err(NoPlace::OffScreen));
    }

    /// §7a.1 rule 3 (Anthony, 2026-10-13), widened 2026-10-18 to every
    /// multi-line field: level with the caret's line.
    #[test]
    fn a_tall_field_keeps_it_level_with_the_caret_line() {
        // Cut off by the bottom of the screen (an Outlook body scrolled long).
        let cut = Rect::new(200, 300, 1200, 1500);
        for top in [320, 500, 900, 1000] {
            let c = caret_at(400, top, 18);
            let p = place(Some(cut), Some(c), &owner()).unwrap();
            assert_eq!((p.x, p.y), (401 + 4, top + 9 - 11), "caret at {top}");
            assert!(!p.rect().overlaps(&c.grow(4)));
        }
        // A short multi-line field: level with the line too (was the corner before 2026-10-18).
        let short = Rect::new(200, 100, 1200, 400);
        assert_eq!(place(Some(short), Some(caret_at(400, 150, 18)), &owner()).unwrap().y, 159 - 11);
        // Without a caret: the visible part's corner.
        assert_eq!(place(Some(cut), None, &owner()).unwrap().y, 1040 - 6 - 22);
    }

    #[test]
    fn the_125_percent_monitor_left_of_the_main_one_scales_it() {
        // DISPLAY1: negative coordinates, 125%.
        let field = Rect::new(-1500, 100, -800, 140);
        let p = place(Some(field), Some(caret_at(-1400, 108, 23)), &owner()).unwrap();
        assert_eq!((p.size, p.monitor), (28, 2), "22 × 1.25 = 27.5 → 28 px");
        assert_eq!(p.x, -800 + 5, "the gap scales too (4 × 1.25)");
        // The same field moved to the 100% monitor: back to 22 px.
        let moved = Rect::new(2000, 100, 2700, 140);
        assert_eq!(place(Some(moved), Some(caret_at(2100, 108, 18)), &owner()).unwrap().size, 22);
    }

    #[test]
    fn a_field_across_two_monitors_follows_the_caret() {
        let wide = Rect::new(1700, 300, 2300, 330);
        let on_second = place(Some(wide), Some(caret_at(2100, 306, 18)), &owner()).unwrap();
        assert_eq!(on_second.monitor, 1);
        assert!(on_second.rect().left >= 1920);
        let on_first = place(Some(wide), Some(caret_at(1750, 306, 18)), &owner()).unwrap();
        assert_eq!(on_first.monitor, 0);
        assert!(on_first.rect().right <= 1920);
    }

    #[test]
    fn when_every_place_would_cover_the_caret_it_isnt_shown() {
        // A small multi-line field, the caret by its right edge: no room
        // beside the line, above it or below it inside the field.
        let field = Rect::new(100, 100, 300, 150);
        assert_eq!(place(Some(field), Some(caret_at(280, 112, 18)), &owner()), Err(NoPlace::CoversCaret));
        // Caret only, at the monitor's right edge: kept on the monitor means
        // on top of the caret.
        assert_eq!(place(None, Some(caret_at(1915, 500, 18)), &owner()), Err(NoPlace::CoversCaret));
    }

    #[test]
    fn it_stays_on_the_monitor() {
        // A single-line field at the top edge: centred would poke above it.
        let p = place(Some(Rect::new(100, 0, 600, 10)), None, &owner()).unwrap();
        assert!(p.y >= 0 && p.rect().bottom <= 1040);
    }

    #[test]
    fn a_wobble_of_two_pixels_doesnt_move_it() {
        let a = Placement { x: 100, y: 100, size: 22, monitor: 0 };
        assert!(!should_move(&a, &Placement { x: 102, y: 99, ..a }));
        assert!(should_move(&a, &Placement { x: 103, ..a }));
        assert!(should_move(&a, &Placement { monitor: 1, ..a }));
        assert!(should_move(&a, &Placement { size: 28, ..a }));
    }

    #[test]
    fn it_hides_while_the_field_moves_and_returns_after_one_still_reading() {
        let mut t = Tracker::default();
        let r = Rect::new(0, 100, 500, 400);
        let down = |by: i32| Rect::new(0, 100 - by, 500, 400 - by);
        assert!(t.steady(Some(r)), "a new field shows at once");
        assert!(t.steady(Some(r)));
        assert!(!t.steady(Some(down(40))), "scrolling: hidden");
        assert!(!t.steady(Some(down(80))), "still scrolling");
        assert!(t.steady(Some(down(80))), "held still for one reading: back");
        assert!(t.steady(Some(down(81))), "a 1 px wobble isn't a move");
        t.reset();
        assert!(t.steady(Some(Rect::new(900, 900, 1000, 930))), "another field: steady at once");
    }

    fn all_good() -> Conditions {
        Conditions {
            setting_on: true,
            suggestions: 3,
            rewriting: false,
            editable: true,
            password: false,
            read: true,
            app_on: true,
            app_paused: false,
            paused: false,
            hidden: false,
            fullscreen: false,
            field_focused: true,
            app_foreground: true,
        }
    }

    #[test]
    fn it_shows_only_when_every_condition_holds_and_each_alone_hides_it() {
        assert_eq!(shown(&all_good()), Ok(()));
        type Break = fn(&mut Conditions);
        let cases: [(Break, Hide); 12] = [
            (|c| c.setting_on = false, Hide::SettingOff),
            (|c| c.password = true, Hide::Password),
            (|c| c.read = false, Hide::NotRead),
            (|c| c.editable = false, Hide::ReadOnly),
            (|c| c.app_paused = true, Hide::AppPaused),
            (|c| c.app_on = false, Hide::AppOff),
            (|c| c.paused = true, Hide::Paused),
            (|c| c.hidden = true, Hide::Hidden),
            (|c| c.fullscreen = true, Hide::Fullscreen),
            (|c| c.field_focused = false, Hide::NoFocus),
            (|c| c.app_foreground = false, Hide::NotForeground),
            (|c| c.suggestions = 0, Hide::NoSuggestions),
        ];
        for (break_it, why) in cases {
            let mut c = all_good();
            break_it(&mut c);
            assert_eq!(shown(&c), Err(why));
        }
    }

    #[test]
    fn off_by_default_and_privacy_reasons_come_first() {
        assert_eq!(shown(&Conditions::default()), Err(Hide::SettingOff));
        let mut c = all_good();
        c.password = true;
        c.suggestions = 0;
        c.paused = true;
        assert_eq!(shown(&c), Err(Hide::Password));
    }

    #[test]
    fn a_rewrite_keeps_it_shown_and_wins_the_look() {
        let mut c = all_good();
        c.suggestions = 0;
        c.rewriting = true;
        assert_eq!(shown(&c), Ok(()));
        assert_eq!(look(&c), Look::Delegate);
        assert_eq!(look(&all_good()), Look::Suggest(3));
        // Paused still hides it, rewrite or not.
        c.paused = true;
        assert_eq!(shown(&c), Err(Hide::Paused));
    }

    #[test]
    fn the_badge_reads_like_the_pills() {
        assert_eq!(badge(1), "1");
        assert_eq!(badge(9), "9");
        assert_eq!(badge(10), "9+");
    }

    #[test]
    fn the_setting_reaches_the_writing_prefs() {
        let mut s = crate::settings::Settings::default();
        assert!(!super::super::prefs::Prefs::from_settings(&s).bubble);
        s.writing_bubble = true;
        assert!(super::super::prefs::Prefs::from_settings(&s).bubble);
    }
}
