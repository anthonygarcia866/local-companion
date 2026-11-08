// One-step jobs in Outlook and Word (a live revision accepted, a rewrite's
// "Use this"): the paragraph goes in through Word's object model, one
// Range.Text replacement, which Word records as ONE undo step. No clipboard,
// no keys. Outlook's compose box is a Word document too: its window's
// WordEditor (a message window), or the reading pane's inline reply.
//
// It is used only after apply.rs has selected the paragraph through UI
// Automation and read the selection back; here Word's own selection must
// hold the same text again before anything is written, so the write lands
// on exactly what was checked. Anything unexpected (no running app, another
// editor, a selection that differs, a protected document) is a refusal, and
// apply.rs pastes or types instead.
//
// Only attaches to an app already running (GetActiveObject; Outlook is one
// process, so asking COM for it returns the running one). Late-bound, on the
// capture thread. Privacy: nothing here logs, stores or sends text.

use windows::core::{Interface, IUnknown, PCWSTR};
use windows::Win32::System::Com::{CLSIDFromProgID, CoCreateInstance, IDispatch, CLSCTX_LOCAL_SERVER};
use windows::Win32::System::Ole::GetActiveObject;
use windows::Win32::System::Variant::VARIANT;

use crate::outlook::Disp;

/// Outlook's window classes (OlObjectClass): the main window, a message window.
const OL_EXPLORER: i32 = 34;
const OL_INSPECTOR: i32 = 35;

/// An app whose text box is a Word document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Outlook,
    Word,
}

/// The host for a process name (any case); None for every other app.
pub fn host(exe: &str) -> Option<Host> {
    match exe.to_ascii_lowercase().as_str() {
        "outlook.exe" => Some(Host::Outlook),
        "winword.exe" => Some(Host::Word),
        _ => None,
    }
}

/// `text` as Word's Range.Text takes it: a line break inside the paragraph
/// is Word's manual line break (U+000B), never a new paragraph.
pub fn word_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\u{b}")
}

/// The running app for `progid`. `one_process`: the app is never started
/// twice, so COM hands back the running one if the table has no entry.
fn running(progid: PCWSTR, one_process: bool) -> Option<Disp> {
    let clsid = unsafe { CLSIDFromProgID(progid) }.ok()?;
    let mut unknown: Option<IUnknown> = None;
    if unsafe { GetActiveObject(&clsid, None, &mut unknown) }.is_ok() {
        if let Some(d) = unknown.and_then(|u| u.cast::<IDispatch>().ok()) {
            return Some(Disp::from(d));
        }
    }
    if !one_process {
        return None;
    }
    let app: IDispatch = unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }.ok()?;
    Some(Disp::from(app))
}

/// The Word document the user is writing in: Outlook's active message
/// window or inline reply; Word's active document.
fn document(host: Host) -> Option<Disp> {
    match host {
        Host::Outlook => {
            let app = running(windows::core::w!("Outlook.Application"), true)?;
            let window = app.call_obj("ActiveWindow", &[]).ok()?;
            match window.int("Class") {
                OL_INSPECTOR => window.obj("WordEditor").ok(),
                OL_EXPLORER => window.obj("ActiveInlineResponseWordEditor").ok(),
                _ => None,
            }
        }
        Host::Word => running(windows::core::w!("Word.Application"), false)?.obj("ActiveDocument").ok(),
    }
}

/// Word's selection in `doc`: its ActiveWindow's, else its first window's.
/// Outlook's message window hands out a WordEditor whose ActiveWindow is
/// empty (probe on a test draft, 2026-11-07: every one-step job fell back to
/// typing); Windows(1) is that same document's own window.
fn selection(doc: &Disp) -> Option<Disp> {
    doc.obj("ActiveWindow")
        .and_then(|w| w.obj("Selection"))
        .or_else(|_| doc.obj("Windows").and_then(|ws| ws.item(1)).and_then(|w| w.obj("Selection")))
        .ok()
}

thread_local! {
    /// Why the last replace_selection on this thread refused ("" = it wrote).
    static REFUSED: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}

/// The code for the last replace_selection's refusal, "" when it wrote.
pub fn last_refusal() -> &'static str {
    REFUSED.with(std::cell::Cell::get)
}

/// Replaces Word's selection with `text` in one Range.Text assignment (one
/// undo step), if the app is Outlook or Word and its selection holds exactly
/// `expect` (but for the kind of quote marks). False, with nothing written,
/// otherwise (last_refusal says which step refused, as a code).
pub fn replace_selection(exe: &str, expect: &str, text: &str) -> bool {
    let result = try_replace(exe, expect, text);
    REFUSED.with(|r| r.set(result.err().unwrap_or("")));
    result.is_ok()
}

fn try_replace(exe: &str, expect: &str, text: &str) -> Result<(), &'static str> {
    let host = host(exe).ok_or("not-word")?;
    // Outlook (2026-11-07, run 16 + harness probe): it ignores every
    // simulated paste (injected Ctrl+V keys with or without scan codes, or a
    // WM_PASTE message) and its Ctrl+Z ignores object-model writes. The write here at
    // least applies (one write, Undo via Word's own Undo command); the paste
    // (#96) applied nothing. One keyboard Ctrl+Z in Outlook is still open.
    let doc = document(host).ok_or("no-document")?;
    let range = selection(&doc).ok_or("no-selection")?.obj("Range").map_err(|_| "no-range")?;
    // The selection read back through Word itself, right before the write.
    if range.int("End") <= range.int("Start") {
        return Err("empty-selection");
    }
    if !crate::writing::apply::same(&range.text("Text"), expect) {
        return Err("selection-differs");
    }
    range.put("Text", VARIANT::from(word_text(text).as_str())).map_err(|_| "write-failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_outlook_and_word_are_word_documents() {
        assert_eq!(host("OUTLOOK.EXE"), Some(Host::Outlook));
        assert_eq!(host("WINWORD.EXE"), Some(Host::Word));
        assert_eq!(host("winword.exe"), Some(Host::Word));
        // New Outlook (olk.exe) is a web view: no Word object model.
        for other in ["olk.exe", "ms-teams.exe", "notepad.exe", "chrome.exe", ""] {
            assert_eq!(host(other), None, "{other}");
        }
    }

    /// Outlook's message window: the WordEditor's ActiveWindow is empty, so
    /// the selection comes from the document's first window instead.
    #[test]
    fn the_selection_falls_back_to_the_documents_own_window() {
        let src = include_str!("word_om.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let f = &code[code.find("fn selection(").unwrap()..code.find("thread_local!").unwrap()];
        let active = f.find("obj(\"ActiveWindow\")").unwrap();
        let first = f.find("ws.item(1)").unwrap();
        assert!(active < first && f.contains(".or_else("));
        assert!(code.contains("selection(&doc).ok_or(\"no-selection\")"));
    }

    #[test]
    fn a_refusal_is_a_code_and_the_app_check_comes_first() {
        assert!(!replace_selection("notepad.exe", "a", "b"));
        assert_eq!(last_refusal(), "not-word");
        // Outlook keeps this path: no simulated paste lands there.
        let code = include_str!("word_om.rs");
        let code = code.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("return Err(\"outlook-uses-paste\")"));
    }

    #[test]
    fn a_line_break_stays_inside_the_paragraph() {
        assert_eq!(word_text("one\ntwo"), "one\u{b}two");
        assert_eq!(word_text("one\r\ntwo\rthree"), "one\u{b}two\u{b}three");
        assert_eq!(word_text("plain"), "plain");
    }

    /// One assignment, after Word's own selection is read back; no
    /// clipboard, no keys, never a new app started for Word, never a log.
    #[test]
    fn the_write_is_one_range_text_after_a_read_back() {
        let src = include_str!("word_om.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let body = &code[code.find("fn try_replace").unwrap()..];
        let check = body.find("same(&range.text(\"Text\"), expect)").expect("read back");
        let write = body.find("range.put(\"Text\"").expect("one Range.Text");
        assert!(check < write);
        assert_eq!(body.matches(".put(").count(), 1, "one write");
        for w in ["Clipboard", "SendInput", "TypeText", "log::line", "println!", "eprintln!"] {
            assert!(!code.contains(w), "{w}");
        }
        assert!(code.contains("running(windows::core::w!(\"Word.Application\"), false)"), "Word is never started");
    }
}
