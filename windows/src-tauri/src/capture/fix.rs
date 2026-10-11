// Phase 1b: the field a fix is typed into, through UI Automation, and the
// Copy button's clipboard (docs/phase1-plan.md §7). The steps themselves, and
// their order, are in writing/apply.rs; this is only how each one is done on
// Windows. Runs on the capture thread, which owns every UIA object.
//
// Privacy: nothing here logs, stores or sends the field's text.

use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
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
    VK_RWIN, VK_SHIFT,
};

use super::sensitive::{is_browser, is_sensitive_field, PauseList};
use super::uia::{edit_signals, field_labels, foreground_pid, offset_of, page_urls, process_name};
use super::{is_editable, MAX_FIELD_CHARS};
use crate::writing::apply::Field;
use crate::writing::Outcome;

/// How long a held key or mouse button is waited for before the fix is refused.
const KEYS_WAIT: Duration = Duration::from_millis(500);

/// The field a fix came from: the element focused when it was checked, and
/// its process.
pub struct UiaField<'a> {
    uia: &'a IUIAutomation,
    element: IUIAutomationElement,
    pid: u32,
    pause: &'a PauseList,
}

impl<'a> UiaField<'a> {
    pub fn new(uia: &'a IUIAutomation, element: IUIAutomationElement, pid: u32, pause: &'a PauseList) -> Self {
        UiaField { uia, element, pid, pause }
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
        // Windows drops a normal app's keys into an elevated one (UIPI).
        if elevated(self.pid) {
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

    fn keys_ready(&mut self) -> Result<(), Outcome> {
        let until = Instant::now() + KEYS_WAIT;
        loop {
            if !keys_held() {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Outcome::KeysHeld);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn type_over(&mut self, text: &str) -> bool {
        let inputs: Vec<INPUT> = key_events(text)
            .into_iter()
            .map(|k| {
                let (vk, scan, flags) = match k {
                    Key::Unicode(unit, up) => (VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
                    Key::Backspace(up) => (VK_BACK, 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
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

    fn pause(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// One keyboard event to send.
#[derive(Debug, PartialEq)]
pub enum Key {
    /// A UTF-16 unit, typed as Unicode (no layout, no Caps Lock); true = up.
    Unicode(u16, bool),
    Backspace(bool),
}

/// The events that type `text` over a selection: each UTF-16 unit down and up
/// (an emoji is two units), or one Backspace for a removal.
pub fn key_events(text: &str) -> Vec<Key> {
    if text.is_empty() {
        return vec![Key::Backspace(false), Key::Backspace(true)];
    }
    text.encode_utf16().flat_map(|u| [Key::Unicode(u, false), Key::Unicode(u, true)]).collect()
}

/// A modifier key or a mouse button is down right now.
fn keys_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
        .iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
}

/// Whether the process runs elevated (as administrator). A token Glim can't
/// even read counts as elevated: Glim couldn't type into it either.
fn elevated(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok();
        let _ = CloseHandle(process);
        if !opened {
            return true;
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
        !ok || e.TokenIsElevated != 0
    }
}

/// Puts `text` on the clipboard (a Copy button the user clicked). False if
/// another app holds the clipboard.
pub fn copy_to_clipboard(text: &str) -> bool {
    let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut open = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                open = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !open {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).ok()?;
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), dst, units.len());
            let _ = GlobalUnlock(mem);
            // The clipboard owns the memory once this succeeds.
            if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0))).is_err() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_is_typed_as_unicode_units_down_and_up_and_a_removal_is_one_backspace() {
        assert_eq!(key_events("ok"), [Key::Unicode(111, false), Key::Unicode(111, true), Key::Unicode(107, false), Key::Unicode(107, true)]);
        assert_eq!(key_events(""), [Key::Backspace(false), Key::Backspace(true)]);
        // An emoji is two UTF-16 units: four events.
        assert_eq!(key_events("\u{1F600}").len(), 4);
        // Never Enter or Tab: those are refused before this (writing::offerable).
        assert!(key_events("it's").iter().all(|k| !matches!(k, Key::Unicode(13 | 10 | 9, _))));
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
    fn this_module_never_logs_or_writes_text() {
        let src = include_str!("fix.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for w in ["log::line", "eprintln!", "println!", "std::fs::write", "dbg!"] {
            assert!(!code.contains(w), "{w}");
        }
    }
}
