// Phase 0b text-capture spike: what UI Automation can read from the focused
// field of other apps. Dev only (GLIM_DEV=1), read-only, and private:
//
// - The text, and the window title, exist only in memory and in the live
//   "capture-debug" event to the debug panel. Nothing here writes either to
//   disk or to the log. `CaptureMeta`, the only thing that is ever recorded,
//   has no field that could hold them, and `Capture`'s Debug output leaves
//   them out, so even a stray `{:?}` can't leak them. Tests check all of it.
// - Password fields (UIA IsPassword) are skipped before any pattern is asked
//   for: nothing is read from them at all.
// - Nothing is written back to any app, and there is no keyboard hook.
//
// The results log (`capture-results.jsonl` in the local data folder) records
// one line per field visited: app, control type, pattern, readable, char
// count. docs/capture-results.md is written from it.

#[cfg(windows)]
pub mod uia;

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What can be recorded about a focused field: never its text or its title.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMeta {
    /// Executable name, e.g. "notepad.exe".
    pub app: String,
    /// UIA control type, e.g. "Edit", "Document".
    pub control_type: String,
    /// "TextPattern2", "TextPattern", "ValuePattern", "none", or "skipped
    /// (password)".
    pub pattern: String,
    pub readable: bool,
    pub password: bool,
    pub char_count: usize,
    /// Caret position in characters from the start, when the pattern gives it.
    pub caret: Option<usize>,
}

/// One reading of the focused field, for the live debug panel only.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub meta: CaptureMeta,
    /// The foreground window's title. Live panel only.
    pub window_title: String,
    /// The part of the field around the caret (see `caret_window`). Live panel
    /// only; None for password fields and when nothing could be read. The
    /// full field text (up to MAX_FIELD_CHARS) never leaves the capture thread.
    pub excerpt: Option<Excerpt>,
    /// What delivered this reading: "event" (a UIA focus-changed event) or
    /// "poll" (the periodic re-read).
    pub via: String,
}

/// A window of the field's text, in characters from the field's start.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excerpt {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl std::fmt::Debug for Capture {
    /// Leaves out the excerpt and the window title, so formatting a Capture
    /// into a log line can never carry them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("meta", &self.meta)
            .field("window_title", &"<not shown>")
            .field("excerpt", &self.excerpt.as_ref().map(|e| (e.start, e.end)))
            .field("via", &self.via)
            .finish()
    }
}

/// The most text read from one field: a safety limit (a whole document can
/// be huge). Read into memory only, never stored.
pub const MAX_FIELD_CHARS: i32 = 20_000;
/// The panel shows at most this much of the current paragraph before the caret…
pub const BEFORE_CARET: usize = 500;
/// …and this much after it.
pub const AFTER_CARET: usize = 200;

/// The current paragraph around the caret, at most BEFORE_CARET characters
/// before it and AFTER_CARET after. Without a caret (ValuePattern), the end of
/// the field, where typing usually happens.
pub fn caret_window(text: &str, caret: Option<usize>) -> Excerpt {
    let chars: Vec<char> = text.chars().collect();
    let caret = caret.unwrap_or(chars.len()).min(chars.len());
    let is_break = |c: &char| *c == '\n' || *c == '\r';
    let par_start = chars[..caret].iter().rposition(is_break).map(|i| i + 1).unwrap_or(0);
    let par_end = chars[caret..].iter().position(is_break).map(|i| caret + i).unwrap_or(chars.len());
    let start = par_start.max(caret.saturating_sub(BEFORE_CARET));
    let end = par_end.min(caret + AFTER_CARET);
    Excerpt { text: chars[start..end].iter().collect(), start, end }
}

/// The results log line for a field: JSON of `CaptureMeta` and a timestamp.
pub fn results_line(meta: &CaptureMeta, unix_seconds: u64) -> String {
    let mut v = serde_json::to_value(meta).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("at".into(), unix_seconds.into());
    }
    v.to_string()
}

/// Where the results log lives.
pub fn results_path() -> PathBuf {
    crate::settings::local_dir().join("capture-results.jsonl")
}

/// Appends one field's metadata to the results log at `path`. Takes only a
/// `CaptureMeta`: a `Capture` (with its text) can't be passed here.
pub fn record_to(path: &Path, meta: &CaptureMeta) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", results_line(meta, now))
}

/// UIA control type id → name (UIA_ButtonControlTypeId = 50000 …).
pub fn control_type_name(id: i32) -> String {
    const NAMES: [&str; 41] = [
        "Button", "Calendar", "CheckBox", "ComboBox", "Edit", "Hyperlink", "Image", "ListItem", "List",
        "Menu", "MenuBar", "MenuItem", "ProgressBar", "RadioButton", "ScrollBar", "Slider", "Spinner",
        "StatusBar", "Tab", "TabItem", "Text", "ToolBar", "ToolTip", "Tree", "TreeItem", "Custom", "Group",
        "Thumb", "DataGrid", "DataItem", "Document", "SplitButton", "Window", "Pane", "Header", "HeaderItem",
        "Table", "TitleBar", "Separator", "SemanticZoom", "AppBar",
    ];
    usize::try_from(id - 50000)
        .ok()
        .and_then(|i| NAMES.get(i))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("type {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "SENTINEL-captured-text-7f3a91";
    const TITLE: &str = "SENTINEL-window-title-55c2";

    fn sample() -> Capture {
        Capture {
            meta: CaptureMeta {
                app: "notepad.exe".into(),
                control_type: "Document".into(),
                pattern: "TextPattern2".into(),
                readable: true,
                password: false,
                char_count: SECRET.chars().count(),
                caret: Some(4),
            },
            window_title: TITLE.into(),
            excerpt: Some(Excerpt { text: SECRET.into(), start: 0, end: 29 }),
            via: "event".into(),
        }
    }

    #[test]
    fn the_window_is_the_current_paragraph_around_the_caret() {
        let text = "First paragraph.\nSecond one, where the caret is.\nThird.";
        let caret = text.find("caret").unwrap();
        let w = caret_window(text, Some(caret));
        assert_eq!(w.text, "Second one, where the caret is.");
        assert_eq!((w.start, w.end), (17, 48));
    }

    #[test]
    fn the_window_is_at_most_500_before_and_200_after() {
        let text = "a".repeat(2000);
        let w = caret_window(&text, Some(1000));
        assert_eq!((w.start, w.end), (500, 1200));
        assert_eq!(w.text.chars().count(), 700);
        // Near the edges it stops at the field.
        assert_eq!(caret_window(&text, Some(100)).start, 0);
        assert_eq!(caret_window(&text, Some(1950)).end, 2000);
    }

    #[test]
    fn without_a_caret_the_window_is_the_end_of_the_field() {
        let text = format!("{}\nlast line", "x".repeat(900));
        let w = caret_window(&text, None);
        assert_eq!(w.text, "last line");
        // Characters, not bytes: multi-byte text is cut on character bounds.
        assert_eq!(caret_window("héllo wörld", Some(5)).text, "héllo wörld");
    }

    #[test]
    fn the_results_log_never_gets_the_text_or_the_title() {
        let dir = std::env::temp_dir().join(format!("glim-capture-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("capture-results.jsonl");
        let capture = sample();
        record_to(&path, &capture.meta).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains(SECRET) && !written.contains(TITLE), "{written}");
        assert!(written.contains("\"app\":\"notepad.exe\"") && written.contains("\"charCount\":29"), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn formatting_a_capture_never_shows_the_text_or_the_title() {
        let shown = format!("{:?} {:#?}", sample(), sample());
        assert!(!shown.contains(SECRET) && !shown.contains(TITLE), "{shown}");
    }

    #[test]
    fn nothing_in_the_capture_code_logs_or_writes_text() {
        // Every logging or file-writing call in the capture module, by source:
        // none may mention the text or the window title.
        for (file, src) in [("mod.rs", include_str!("mod.rs")), ("uia.rs", include_str!("uia.rs"))] {
            let code = src.split("#[cfg(test)]").next().unwrap();
            for (i, line) in code.lines().enumerate() {
                let writes = ["log::line", "eprintln!", "println!", "writeln!", "std::fs::write", "dbg!"]
                    .iter()
                    .any(|w| line.contains(w));
                if writes {
                    for banned in [".text", "excerpt", "window_title", "title"] {
                        assert!(!line.contains(banned), "{file}:{} writes {banned}: {}", i + 1, line.trim());
                    }
                }
            }
        }
    }

    #[test]
    fn control_types_have_names() {
        assert_eq!(control_type_name(50004), "Edit");
        assert_eq!(control_type_name(50030), "Document");
        assert_eq!(control_type_name(49999), "type 49999");
    }
}
