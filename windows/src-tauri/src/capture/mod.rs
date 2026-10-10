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
    /// Skipped because it isn't editable (read-only page content).
    pub read_only: bool,
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

/// What UI Automation says about whether a field can be typed in, gathered
/// before any text is read. The writing layer reads only editable fields:
/// Chrome, for one, exposes a whole web page as a focusable read-only
/// Document (seen 2026-10-10: an AppFolio dashboard, 6,316 characters of page
/// content captured with no text box focused).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditSignals {
    /// ValuePattern.IsReadOnly, when the element has a ValuePattern.
    pub value_read_only: Option<bool>,
    /// The IsReadOnly text attribute of the whole document range, when the
    /// element has a TextPattern and the attribute is not mixed.
    pub text_read_only: Option<bool>,
    /// The legacy (MSAA/IA2) state has STATE_SYSTEM_READONLY.
    pub legacy_read_only: bool,
    /// The control type is Edit.
    pub edit_control: bool,
}

/// Whether the writing layer may read this field: the most specific signal
/// decides — ValuePattern, then the text's own IsReadOnly attribute, then the
/// legacy read-only state — and an element with none of them counts as
/// editable only if it is an Edit control. Anything unclear is skipped.
pub fn is_editable(s: &EditSignals) -> bool {
    if let Some(read_only) = s.value_read_only {
        return !read_only;
    }
    if let Some(read_only) = s.text_read_only {
        return !read_only;
    }
    if s.legacy_read_only {
        return false;
    }
    s.edit_control
}

/// Glim's own process tree: `root` and every process descended from it, from a
/// snapshot of (pid, parent pid) pairs. Glim's webviews run as separate
/// msedgewebview2.exe children of glim.exe, so skipping only Glim's own pid
/// let the capture layer evaluate Glim's own windows (seen 2026-10-10).
pub fn process_tree(root: u32, pairs: &[(u32, u32)]) -> std::collections::HashSet<u32> {
    let mut tree = std::collections::HashSet::from([root]);
    loop {
        let before = tree.len();
        for &(pid, parent) in pairs {
            if pid != parent && tree.contains(&parent) {
                tree.insert(pid);
            }
        }
        if tree.len() == before {
            return tree;
        }
    }
}

/// The `pattern` recorded for a field that was skipped as read-only.
pub const SKIPPED_READ_ONLY: &str = "skipped (read-only content)";

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
                read_only: false,
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

    fn signals(value: Option<bool>, text: Option<bool>, legacy: bool, edit: bool) -> EditSignals {
        EditSignals { value_read_only: value, text_read_only: text, legacy_read_only: legacy, edit_control: edit }
    }

    #[test]
    fn read_only_page_content_is_not_editable() {
        // A web page in Chrome: a Document whose text is read-only, with the
        // legacy read-only state; with or without a read-only ValuePattern.
        assert!(!is_editable(&signals(None, Some(true), true, false)));
        assert!(!is_editable(&signals(Some(true), Some(true), true, false)));
        // A Document nothing says anything about: skipped, not guessed.
        assert!(!is_editable(&signals(None, None, false, false)));
        // A read-only Edit (a log view, a disabled box).
        assert!(!is_editable(&signals(Some(true), None, false, true)));
        assert!(!is_editable(&signals(None, None, true, true)));
    }

    #[test]
    fn editable_fields_are_editable() {
        // Notepad / Word / Outlook compose: a Document with editable text.
        assert!(is_editable(&signals(None, Some(false), false, false)));
        // Gmail compose (contenteditable) inside a read-only page state.
        assert!(is_editable(&signals(None, Some(false), true, false)));
        // An ordinary text box, with or without a ValuePattern.
        assert!(is_editable(&signals(Some(false), None, false, true)));
        assert!(is_editable(&signals(None, None, false, true)));
        // ValuePattern wins over the text attribute when they disagree.
        assert!(is_editable(&signals(Some(false), Some(true), false, true)));
    }

    #[test]
    fn editability_is_decided_before_any_text_is_read() {
        // In uia.rs read(): the password and read-only checks both come before
        // read_text, the only function that asks for text.
        let src = include_str!("uia.rs").replace("\r\n", "\n");
        let read = &src[src.find("fn read(").unwrap()..];
        let read = &read[..read.find("\n}\n").unwrap()];
        let password = read.find("CurrentIsPassword").expect("password check");
        let editable = read.find("is_editable(").expect("editability check");
        let text = read.find("read_text(").expect("text read");
        assert!(password < text && editable < text && password < editable);
    }

    #[test]
    fn glims_own_process_tree_includes_its_webview_children() {
        // glim 100 → msedgewebview2 200 → 201, 202 (renderers); 300 is another
        // app's webview; 400 claims to be its own parent (pid reuse) and 500 is
        // unrelated.
        let pairs = [(200, 100), (201, 200), (202, 200), (300, 999), (400, 400), (500, 1)];
        let tree = process_tree(100, &pairs);
        assert_eq!(tree, std::collections::HashSet::from([100, 200, 201, 202]));
    }

    #[test]
    fn glims_own_windows_are_skipped_before_anything_is_read() {
        // In uia.rs run(): the own-tree check comes before read().
        let src = include_str!("uia.rs").replace("\r\n", "\n");
        let run = &src[src.find("fn run(").unwrap()..];
        let skip = run.find("own.contains(").expect("own-process check");
        let read = run.find("read(&element").expect("read call");
        assert!(skip < read);
    }

    #[test]
    fn control_types_have_names() {
        assert_eq!(control_type_name(50004), "Edit");
        assert_eq!(control_type_name(50030), "Document");
        assert_eq!(control_type_name(49999), "type 49999");
    }
}
