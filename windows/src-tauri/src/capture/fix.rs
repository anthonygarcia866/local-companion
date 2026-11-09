// Phase 1b: the field a fix is typed into, through UI Automation, and the
// Copy button's clipboard (docs/phase1-plan.md §7). The steps themselves, and
// their order, are in writing/apply.rs; this is only how each one is done on
// Windows. Runs on the capture thread, which owns every UIA object.
//
// Privacy: nothing here logs, stores or sends the field's text.

use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::DataExchange::{
    CloseClipboard, CountClipboardFormats, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2,
    IUIAutomationTextRange, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
    TextUnit_Paragraph, UIA_TextPattern2Id, UIA_TextPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LBUTTON, VK_LWIN, VK_MBUTTON, VK_MENU, VK_RBUTTON,
    VK_RWIN, VK_SHIFT, VK_V,
};

use super::sensitive::{is_browser, is_sensitive_field, PauseList};
use super::uia::{edit_signals, field_labels, foreground_pid, offset_of, page_urls, process_name};
use super::{is_editable, MAX_FIELD_CHARS};
use crate::writing::apply::Field;
use crate::writing::Outcome;

/// The field a fix came from: the element focused when it was checked, and
/// its process.
pub struct UiaField<'a> {
    uia: &'a IUIAutomation,
    element: IUIAutomationElement,
    pid: u32,
    pause: &'a PauseList,
    /// The user's clipboard while Glim pastes a one-step job: put back
    /// after, or when this field is dropped (any way out, a panic too).
    saved_clip: Option<ClipGuard>,
}

impl<'a> UiaField<'a> {
    pub fn new(uia: &'a IUIAutomation, element: IUIAutomationElement, pid: u32, pause: &'a PauseList) -> Self {
        UiaField { uia, element, pid, pause, saved_clip: None }
    }

    fn pattern2(&self) -> Option<IUIAutomationTextPattern2> {
        unsafe { self.element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) }.ok()
    }

    fn pattern(&self) -> Option<IUIAutomationTextPattern> {
        unsafe { self.element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }.ok()
    }

    /// The caret as a range: TextPattern2's caret, else the selection's start.
    fn caret_range(&self) -> Option<IUIAutomationTextRange> {
        unsafe {
            if let Some(p2) = self.pattern2() {
                let mut active = BOOL::default();
                if let Ok(c) = p2.GetCaretRange(&mut active) {
                    return Some(c);
                }
            }
            let sel = self.pattern()?.GetSelection().ok()?;
            if sel.Length().ok()? < 1 {
                return None;
            }
            let r = sel.GetElement(0).ok()?;
            r.MoveEndpointByRange(TextPatternRangeEndpoint_End, &r, TextPatternRangeEndpoint_Start).ok()?;
            Some(r)
        }
    }
}

impl Field for UiaField<'_> {
    type Range = IUIAutomationTextRange;

    fn gate(&mut self) -> Result<(), Outcome> {
        if crate::writing::paused() {
            return Err(Outcome::Paused);
        }
        unsafe {
            // Still the field it came from, and still in front: Glim never
            // moves focus to apply a fix.
            let focused = self.uia.GetFocusedElement().map_err(|_| Outcome::Away)?;
            if !self.uia.CompareElements(&focused, &self.element).is_ok_and(|b| b.as_bool()) || foreground_pid() != self.pid {
                return Err(Outcome::Away);
            }
            // Every PR #8 rule, on the field as it is now.
            let app = process_name(self.pid as i32);
            if !crate::writing::apps::enabled(&app) || !crate::writing::apps::fixes(&app) || self.pause.pauses_app(&app) {
                return Err(Outcome::Blocked);
            }
            if self.element.CurrentIsPassword().is_ok_and(|b| b.as_bool()) || is_sensitive_field(&field_labels(&self.element)) {
                return Err(Outcome::Blocked);
            }
            if is_browser(&app)
                && page_urls(self.uia, &self.element)
                    .iter()
                    .any(|u| self.pause.pauses_url(u) || crate::writing::apps::chat_site(u))
            {
                // A paused site, or a web chat app (view-only).
                return Err(Outcome::Blocked);
            }
            let control = self.element.CurrentControlType().map(|t| t.0).unwrap_or(0);
            if !is_editable(&edit_signals(&self.element, control)) {
                return Err(Outcome::Blocked);
            }
            // The text as it is now: a card, SSN or IBAN anywhere in it, no fix.
            let doc = self.document().ok_or(Outcome::Changed)?;
            let text = self.text(&doc).ok_or(Outcome::Changed)?;
            if super::sensitive::contains_sensitive_number(&text) {
                return Err(Outcome::Blocked);
            }
        }
        // Windows drops a normal app's keys into an elevated one (UIPI); an
        // elevated Glim (an admin session, the CI runner) can type there.
        if elevated(self.pid) && !self_elevated() {
            return Err(Outcome::Elevated);
        }
        Ok(())
    }

    fn caret_paragraph(&mut self) -> Option<IUIAutomationTextRange> {
        let r = self.caret_range()?;
        unsafe { r.ExpandToEnclosingUnit(TextUnit_Paragraph).ok()? };
        Some(r)
    }

    fn document(&mut self) -> Option<IUIAutomationTextRange> {
        unsafe {
            if let Some(p2) = self.pattern2() {
                if let Ok(d) = p2.DocumentRange() {
                    return Some(d);
                }
            }
            self.pattern()?.DocumentRange().ok()
        }
    }

    fn text(&mut self, r: &IUIAutomationTextRange) -> Option<String> {
        unsafe { r.GetText(MAX_FIELD_CHARS) }.ok().map(|b| b.to_string())
    }

    fn offset(&mut self, r: &IUIAutomationTextRange) -> Option<usize> {
        let doc = self.document()?;
        offset_of(&doc, r)
    }

    fn caret(&mut self) -> Option<usize> {
        let c = self.caret_range()?;
        let doc = self.document()?;
        offset_of(&doc, &c)
    }

    fn sub(&mut self, r: &IUIAutomationTextRange, from: usize, len: usize) -> Option<IUIAutomationTextRange> {
        unsafe {
            let x = r.Clone().ok()?;
            x.MoveEndpointByRange(TextPatternRangeEndpoint_End, r, TextPatternRangeEndpoint_Start).ok()?;
            if from > 0 && x.Move(TextUnit_Character, from as i32).ok()? != from as i32 {
                return None;
            }
            if len > 0 && x.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, len as i32).ok()? != len as i32 {
                return None;
            }
            Some(x)
        }
    }

    fn select(&mut self, r: &IUIAutomationTextRange) -> bool {
        unsafe { r.Select() }.is_ok()
    }

    fn selection(&mut self) -> Option<String> {
        unsafe {
            let sel = self.pattern()?.GetSelection().ok()?;
            if sel.Length().ok()? != 1 {
                return None;
            }
            sel.GetElement(0).ok()?.GetText(MAX_FIELD_CHARS).ok().map(|b| b.to_string())
        }
    }

    fn keys_down(&mut self) -> bool {
        keys_held()
    }

    fn key_pause(&mut self) {
        std::thread::sleep(Duration::from_millis(crate::writing::apply::KEYS_POLL_MS));
    }

    fn type_over(&mut self, text: &str) -> bool {
        let inputs: Vec<INPUT> = key_events(text)
            .into_iter()
            .map(|k| {
                let (vk, scan, flags) = match k {
                    Key::Unicode(unit, up) => (VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
                    Key::Backspace(up) => (VK_BACK, 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
                    Key::Vk(code, up) => (VIRTUAL_KEY(code), 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
                };
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
                }
            })
            .collect();
        // One call: Windows doesn't interleave the user's own input with it.
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        sent as usize == inputs.len()
    }

    fn replace_in_document(&mut self, expect: &str, text: &str) -> bool {
        super::word_om::replace_selection(&process_name(self.pid as i32), expect, text)
    }

    fn paste_over(&mut self, text: &str) -> bool {
        // The user's whole clipboard first, every format byte for byte; if
        // any of it can't be put back exactly, the text is typed instead.
        let Some(saved) = save_clipboard() else {
            set_paste_refusal("unrestorable");
            return false;
        };
        set_paste_refusal("");
        if !open_clipboard() {
            set_paste_refusal("busy");
            return false;
        }
        // From here on, the guard puts it back however this ends.
        let guard = ClipGuard(saved);
        let set = unsafe { EmptyClipboard() }.is_ok() && set_text(text, true).is_some();
        unsafe {
            let _ = CloseClipboard();
        }
        if !set {
            set_paste_refusal("set-failed");
            return false;
        }
        self.saved_clip = Some(guard);
        // Ctrl+V, in one call (Windows doesn't interleave the user's input).
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            self.after_paste();
            set_paste_refusal("keys");
            return false;
        }
        true
    }

    fn after_paste(&mut self) {
        // Dropping the guard puts the clipboard back.
        self.saved_clip = None;
    }

    fn pause(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
    }
}

thread_local! {
    /// Why the last paste_over on this thread didn't paste ("" = it did).
    static PASTE_REFUSED: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}

fn set_paste_refusal(code: &'static str) {
    PASTE_REFUSED.with(|c| c.set(code));
}

/// The code for the last paste_over's refusal, "" when it pasted (or none ran).
pub fn last_paste_refusal() -> &'static str {
    PASTE_REFUSED.with(std::cell::Cell::get)
}

/// How the last one-step job went in, for the log: codes only, never text.
/// "via word-om", or e.g. "via typed (word-om: no-selection; paste: unrestorable)".
pub fn one_step_note() -> String {
    one_step_note_from(crate::writing::apply::last_path(), super::word_om::last_refusal(), last_paste_refusal())
}

pub fn one_step_note_from(path: &str, om: &str, paste: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    if path.starts_with("refused") {
        return format!(" via {path}");
    }
    let mut why = Vec::new();
    if path != "word-om" && !om.is_empty() && om != "not-word" {
        why.push(format!("word-om: {om}"));
    }
    if path == "typed" && !paste.is_empty() {
        why.push(format!("paste: {paste}"));
    }
    if why.is_empty() { format!(" via {path}") } else { format!(" via {path} ({})", why.join("; ")) }
}

/// One keyboard event to send.
#[derive(Debug, PartialEq)]
pub enum Key {
    /// A UTF-16 unit, typed as Unicode (no layout, no Caps Lock); true = up.
    Unicode(u16, bool),
    Backspace(bool),
    /// A virtual key (Shift, Enter), for a line break; true = up.
    Vk(u16, bool),
}

const VK_SHIFT_CODE: u16 = 0x10;
const VK_RETURN_CODE: u16 = 0x0D;

/// The events that type `text` over a selection: each UTF-16 unit down and up
/// (an emoji is two units), or one Backspace for a removal.
pub fn key_events(text: &str) -> Vec<Key> {
    if text.is_empty() {
        return vec![Key::Backspace(false), Key::Backspace(true)];
    }
    // A line break is Shift+Enter: a new line in Word, Notepad and chat
    // boxes alike, where a plain Enter would send the message. "\r" goes.
    let mut out = Vec::new();
    for u in text.encode_utf16() {
        match u {
            0x0D => {}
            0x0A => out.extend([Key::Vk(VK_SHIFT_CODE, false), Key::Vk(VK_RETURN_CODE, false), Key::Vk(VK_RETURN_CODE, true), Key::Vk(VK_SHIFT_CODE, true)]),
            u => out.extend([Key::Unicode(u, false), Key::Unicode(u, true)]),
        }
    }
    out
}

/// A modifier key or a mouse button is down right now.
fn keys_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
        .iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
}

/// Whether the process runs elevated (as administrator). A token Glim can't
/// even read counts as elevated: Glim couldn't type into it either.
/// Whether `pid` runs elevated; true when Glim can't tell (as if it did).
fn elevated(pid: u32) -> bool {
    token_elevated(pid).unwrap_or(true)
}

/// Whether Glim itself runs elevated; false when it can't tell.
fn self_elevated() -> bool {
    token_elevated(std::process::id()).unwrap_or(false)
}

fn token_elevated(pid: u32) -> Option<bool> {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return None };
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok();
        let _ = CloseHandle(process);
        if !opened {
            return None;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut e as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok.then_some(e.TokenIsElevated != 0)
    }
}

/// Opens the clipboard (a few tries: another app may hold it a moment).
fn open_clipboard() -> bool {
    for _ in 0..5 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Hands `bytes` to the clipboard as `format` (it must be open and emptied).
fn set_data(format: u32, bytes: &[u8]) -> Option<()> {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let dst = GlobalLock(mem) as *mut u8;
        if dst.is_null() {
            let _ = GlobalFree(Some(mem));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        let _ = GlobalUnlock(mem);
        // The clipboard owns the memory once this succeeds.
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
            return None;
        }
    }
    Some(())
}

/// Glim's own mark on a clipboard it fills for a paste: the text there is
/// only passing through. Restoring checks for it (a clipboard the user
/// changed meanwhile is left alone), and at startup any left over from a
/// paste Glim never finished (killed mid-paste) is cleared
/// (`clear_stale_paste`).
const GLIM_MARK: windows::core::PCWSTR = windows::core::w!("Glim.PasteInProgress");

/// Windows' markers that keep a clipboard out of clipboard history (Win+V),
/// the cloud clipboard and clipboard monitors.
/// ExcludeClipboard...'s presence is the signal (no value); the other two
/// hold a DWORD 0.
const PRIVATE_MARKS: [(windows::core::PCWSTR, &[u8]); 3] = [
    (windows::core::w!("ExcludeClipboardContentFromMonitorProcessing"), &[]),
    (windows::core::w!("CanIncludeInClipboardHistory"), &[0, 0, 0, 0]),
    (windows::core::w!("CanUploadToCloudClipboard"), &[0, 0, 0, 0]),
];

fn registered(name: windows::core::PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

/// Puts `text` on the clipboard (it must be open and emptied). `private`:
/// Glim's own paste of the user's paragraph, which is only passing through:
/// marked so Windows keeps it out of clipboard history (Win+V) and the cloud
/// clipboard, and with Glim's mark (`GLIM_MARK`).
fn set_text(text: &str, private: bool) -> Option<()> {
    let bytes: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    if private {
        // Glim's mark first (whatever fails after it, the user's clipboard
        // still goes back), then Windows' marks, before the text.
        set_data(registered(GLIM_MARK), &[])?;
        for (name, data) in PRIVATE_MARKS {
            set_data(registered(name), data)?;
        }
    }
    set_data(CF_UNICODETEXT.0 as u32, &bytes)
}

/// Puts `text` on the clipboard (a Copy button the user clicked). False if
/// another app holds the clipboard.
pub fn copy_to_clipboard(text: &str) -> bool {
    if !open_clipboard() {
        return false;
    }
    let done = unsafe { EmptyClipboard() }.is_ok() && set_text(text, false).is_some();
    unsafe {
        let _ = CloseClipboard();
    }
    done
}

/// At startup: a clipboard that still holds Glim's mark is a paste Glim
/// never finished (it was stopped between the paste and putting the user's
/// clipboard back). It is emptied, so Glim's text never stays there. True
/// when one was found.
pub fn clear_stale_paste() -> bool {
    if !open_clipboard() {
        return false;
    }
    let stale = unsafe { IsClipboardFormatAvailable(registered(GLIM_MARK)) }.is_ok();
    if stale {
        unsafe {
            let _ = EmptyClipboard();
        }
    }
    unsafe {
        let _ = CloseClipboard();
    }
    stale
}

/// The user's clipboard, saved whole for a paste: every format, byte for byte.
#[derive(Clone, Debug, PartialEq)]
pub enum Saved {
    Empty,
    Formats(Vec<(u32, Vec<u8>)>),
}

/// The most Glim saves (all formats together); more and it types instead.
const MAX_SAVED_BYTES: usize = 64 << 20;

/// Whether a clipboard format can be put back exactly from its bytes. Only
/// memory formats can: a picture or drawing handle (bitmap, metafile,
/// palette), an owner-drawn or private handle can't, and neither can the
/// OLE clipboard ("DataObject", "Ole Private Data": the data stays in the
/// app that copied it). `name` is the registered name ("" for Windows' own).
pub fn restorable_format(format: u32, name: &str) -> bool {
    const GDI: [u32; 8] = [
        2,    // CF_BITMAP
        3,    // CF_METAFILEPICT
        9,    // CF_PALETTE
        14,   // CF_ENHMETAFILE
        0x80, // CF_OWNERDISPLAY
        0x82, // CF_DSPBITMAP
        0x83, // CF_DSPMETAFILEPICT
        0x8E, // CF_DSPENHMETAFILE
    ];
    if GDI.contains(&format) || (0x0200..=0x03FF).contains(&format) {
        // CF_PRIVATEFIRST..CF_GDIOBJLAST: handles Glim can't copy.
        return false;
    }
    !["DataObject", "Ole Private Data"].iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// The registered name of a format ("" for Windows' own numbered ones).
fn format_name(format: u32) -> String {
    if format < 0xC000 {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = unsafe { GetClipboardFormatNameW(format, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

/// What the clipboard holds now, if Glim can put it back exactly: every
/// format's bytes. None when it can't (a picture, a drawing, the OLE
/// clipboard, a format whose data the copying app no longer gives
/// (delayed rendering), too big, or the clipboard is busy): then the text
/// is typed and the clipboard is never touched. A clipboard holding only
/// Glim's own leftover paste (its mark) counts as empty.
fn save_clipboard() -> Option<Saved> {
    if !open_clipboard() {
        return None;
    }
    unsafe {
        let saved = (|| {
            if IsClipboardFormatAvailable(registered(GLIM_MARK)).is_ok() {
                return Some(Saved::Empty);
            }
            let mut formats = Vec::new();
            let mut total = 0usize;
            let mut format = 0u32;
            loop {
                format = EnumClipboardFormats(format);
                if format == 0 {
                    break;
                }
                if !restorable_format(format, &format_name(format)) {
                    return None;
                }
                // A delayed-rendered format is rendered now; one its app
                // won't give is not restorable.
                let h = GetClipboardData(format).ok()?;
                let mem = HGLOBAL(h.0);
                let size = GlobalSize(mem);
                total += size;
                if size == 0 || total > MAX_SAVED_BYTES {
                    return None;
                }
                let src = GlobalLock(mem) as *const u8;
                if src.is_null() {
                    return None;
                }
                let bytes = std::slice::from_raw_parts(src, size).to_vec();
                let _ = GlobalUnlock(mem);
                formats.push((format, bytes));
            }
            Some(if formats.is_empty() { Saved::Empty } else { Saved::Formats(formats) })
        })();
        let _ = CloseClipboard();
        saved
    }
}

/// Puts the saved clipboard back exactly (every format, in its order), but
/// only while the clipboard still holds Glim's paste (or nothing, if the
/// paste failed after emptying it): if the user copied something
/// meanwhile, theirs stays.
fn restore_clipboard(saved: &Saved) {
    if !open_clipboard() {
        return;
    }
    unsafe {
        let ours = IsClipboardFormatAvailable(registered(GLIM_MARK)).is_ok() || CountClipboardFormats() == 0;
        if ours && EmptyClipboard().is_ok() {
            if let Saved::Formats(formats) = saved {
                for (format, bytes) in formats {
                    let _ = set_data(*format, bytes);
                }
            }
        }
        let _ = CloseClipboard();
    }
}

/// The user's clipboard while Glim pastes: put back when dropped, so it
/// comes back on every path out of a paste (the `finally`), a panic too.
struct ClipGuard(Saved);

impl Drop for ClipGuard {
    fn drop(&mut self) {
        restore_clipboard(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_is_typed_as_unicode_units_down_and_up_and_a_removal_is_one_backspace() {
        assert_eq!(key_events("ok"), [Key::Unicode(111, false), Key::Unicode(111, true), Key::Unicode(107, false), Key::Unicode(107, true)]);
        assert_eq!(key_events(""), [Key::Backspace(false), Key::Backspace(true)]);
        // Dictation's line break: Shift+Enter (a plain Enter sends a chat
        // message); a "\r" is dropped.
        assert_eq!(
            key_events("a\r\nb"),
            [Key::Unicode(97, false), Key::Unicode(97, true), Key::Vk(0x10, false), Key::Vk(0x0D, false), Key::Vk(0x0D, true), Key::Vk(0x10, true), Key::Unicode(98, false), Key::Unicode(98, true)]
        );
        // An emoji is two UTF-16 units: four events.
        assert_eq!(key_events("\u{1F600}").len(), 4);
        // Never Enter or Tab: those are refused before this (writing::offerable).
        assert!(key_events("it's").iter().all(|k| !matches!(k, Key::Unicode(13 | 10 | 9, _))));
    }

    #[test]
    fn a_browser_copy_is_restorable_and_pictures_drawings_and_the_ole_clipboard_are_not() {
        // What Chrome or Edge puts there for copied rich text, plus Windows'
        // own text forms and Glim's marks: memory formats, put back byte for byte.
        for (format, name) in [
            (13, ""),
            (1, ""),
            (7, ""),
            (16, ""),
            (15, ""), // CF_HDROP: a file list, memory too
            (8, ""),  // CF_DIB alone (its synthesized CF_BITMAP is refused below)
            (0xC0A1, "HTML Format"),
            (0xC0A2, "Chromium internal source URL"),
            (0xC0A3, "Rich Text Format"),
            (0xC0A4, "ExcludeClipboardContentFromMonitorProcessing"),
            (0xC0A5, "CanIncludeInClipboardHistory"),
            (0xC0A6, "CanUploadToCloudClipboard"),
            (0xC0A7, "Glim.PasteInProgress"),
        ] {
            assert!(restorable_format(format, name), "{format:#x} {name}");
        }
        // Handles, not memory: a picture, a drawing, a palette, owner-drawn
        // or private handles; and the OLE clipboard, whose data stays in the
        // app that copied it. Any of these and Glim types instead.
        for (format, name) in [
            (2, ""),
            (3, ""),
            (9, ""),
            (14, ""),
            (0x80, ""),
            (0x82, ""),
            (0x83, ""),
            (0x8E, ""),
            (0x200, ""),
            (0x2FF, ""),
            (0x300, ""),
            (0x3FF, ""),
            (0xC0B1, "DataObject"),
            (0xC0B2, "Ole Private Data"),
        ] {
            assert!(!restorable_format(format, name), "{format:#x} {name}");
        }
    }

    #[test]
    fn glims_paste_is_marked_private_and_the_clipboard_comes_back_on_every_path() {
        let src = include_str!("fix.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        // Glim's mark first, then Windows' three, then the text.
        let set = &code[code.find("fn set_text(").unwrap()..code.find("pub fn copy_to_clipboard").unwrap()];
        let mark = set.find("set_data(registered(GLIM_MARK)").unwrap();
        let marks = set.find("for (name, data) in PRIVATE_MARKS").unwrap();
        let text = set.find("set_data(CF_UNICODETEXT").unwrap();
        assert!(mark < marks && marks < text);
        for m in ["ExcludeClipboardContentFromMonitorProcessing", "CanIncludeInClipboardHistory\"), &[0, 0, 0, 0]", "CanUploadToCloudClipboard\"), &[0, 0, 0, 0]"] {
            assert!(code.contains(m), "{m}");
        }
        // The paste: the guard owns the saved clipboard before it is emptied,
        // and dropping it (any way out of the paste) puts it back.
        let paste = &code[code.find("fn paste_over(").unwrap()..code.find("fn after_paste(").unwrap()];
        assert!(paste.find("ClipGuard(saved)").unwrap() < paste.find("EmptyClipboard()").unwrap());
        assert!(code.contains("impl Drop for ClipGuard {
    fn drop(&mut self) {
        restore_clipboard(&self.0);"));
        // Restoring never overwrites what the user copied meanwhile.
        let restore = &code[code.find("fn restore_clipboard(").unwrap()..code.find("struct ClipGuard").unwrap()];
        assert!(restore.contains("IsClipboardFormatAvailable(registered(GLIM_MARK)).is_ok() || CountClipboardFormats() == 0"));
    }

    /// On the real clipboard (skipped when the session has none, e.g. a
    /// service): a browser-like rich copy is saved whole, Glim's private
    /// paste replaces it with its marks, and it comes back byte for byte; a
    /// leftover Glim paste is cleared at startup.
    #[test]
    fn the_whole_clipboard_round_trips_and_a_stale_paste_is_cleared() {
        let html = b"Version:0.9
StartHTML:0
<b>rich</b> text ".to_vec();
        let text: Vec<u8> = "rich text".encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        let html_format = registered(windows::core::w!("HTML Format"));
        // The user's clipboard as it is now, to leave it as found.
        let before = save_clipboard();
        if !open_clipboard() {
            eprintln!("no clipboard in this session: skipped");
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
        }
        set_data(html_format, &html).unwrap();
        set_data(CF_UNICODETEXT.0 as u32, &text).unwrap();
        unsafe {
            let _ = CloseClipboard();
        }
        let saved = save_clipboard().expect("a rich text copy is restorable");
        let Saved::Formats(formats) = &saved else { panic!("not empty") };
        assert!(formats.iter().any(|(f, b)| *f == html_format && b.starts_with(&html)));
        // Glim's paste: guarded, private, marked.
        {
            let _guard = ClipGuard(saved.clone());
            assert!(open_clipboard());
            unsafe {
                let _ = EmptyClipboard();
            }
            set_text("Glim's paragraph", true).unwrap();
            unsafe {
                let _ = CloseClipboard();
                for name in [GLIM_MARK, PRIVATE_MARKS[0].0, PRIVATE_MARKS[1].0, PRIVATE_MARKS[2].0] {
                    assert!(IsClipboardFormatAvailable(registered(name)).is_ok());
                }
                assert!(IsClipboardFormatAvailable(html_format).is_err());
            }
        } // dropped: put back
        let back = save_clipboard().expect("restorable");
        let Saved::Formats(after) = &back else { panic!("restored") };
        for (f, b) in formats {
            let got = after.iter().find(|(g, _)| g == f).map(|(_, b)| b).expect("format back");
            // GlobalSize may round up: the original bytes, then padding.
            assert!(got.starts_with(b), "{f:#x}");
        }
        unsafe {
            assert!(IsClipboardFormatAvailable(registered(GLIM_MARK)).is_err(), "no mark left");
        }
        // A Glim paste left behind (Glim killed mid-paste) is cleared at startup.
        assert!(open_clipboard());
        unsafe {
            let _ = EmptyClipboard();
        }
        set_text("left behind", true).unwrap();
        unsafe {
            let _ = CloseClipboard();
        }
        assert!(clear_stale_paste());
        assert!(!clear_stale_paste(), "once");
        // The clipboard as it was before the test.
        if let Some(b) = before {
            assert!(open_clipboard());
            unsafe {
                let _ = EmptyClipboard();
            }
            set_data(registered(GLIM_MARK), &[]).unwrap();
            unsafe {
                let _ = CloseClipboard();
            }
            restore_clipboard(&b);
        }
    }

    /// A web chat app is view-only, and the gate checks it again at click
    /// time (a stale list, or a tab switched to claude.ai under it).
    #[test]
    fn the_gate_refuses_web_chat_apps() {
        let src = include_str!("fix.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let gate = &code[code.find("Every PR #8 rule").unwrap()..code.find("fn caret_paragraph").unwrap()];
        assert!(gate.contains("chat_site(u)") && gate.contains("pauses_url(u)"));
    }

    #[test]
    fn the_one_step_note_is_codes_only() {
        assert_eq!(one_step_note_from("", "", ""), "");
        assert_eq!(one_step_note_from("word-om", "", ""), " via word-om");
        assert_eq!(one_step_note_from("paste", "not-word", ""), " via paste");
        assert_eq!(one_step_note_from("paste", "no-selection", ""), " via paste (word-om: no-selection)");
        assert_eq!(one_step_note_from("typed", "selection-differs", "unrestorable"), " via typed (word-om: selection-differs; paste: unrestorable)");
        assert_eq!(one_step_note_from("retyped", "", ""), " via retyped");
        assert_eq!(one_step_note_from("refused (selection-differs)", "no-selection", "busy"), " via refused (selection-differs)");
    }

    #[test]
    fn this_module_never_logs_or_writes_text() {
        let src = include_str!("fix.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for w in ["log::line", "eprintln!", "println!", "std::fs::write", "dbg!"] {
            assert!(!code.contains(w), "{w}");
        }
    }
}
