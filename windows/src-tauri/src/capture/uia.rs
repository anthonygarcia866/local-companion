// The Windows side of the text-capture spike: UI Automation, read-only.
//
// One thread owns every UIA object. It subscribes to focus-changed events (the
// handler only wakes the thread up, so no COM object crosses threads) and also
// re-reads the focused field every POLL so the debug panel follows typing.
// For each field: skip it entirely if IsPassword, then skip it entirely if it
// isn't editable (read-only page content, e.g. a whole web page in Chrome);
// only then try TextPattern2 (text + caret), then TextPattern (text +
// selection start as the caret), then ValuePattern (text only).

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use windows::core::{implement, Ref, Result as WinResult, BOOL};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationFocusChangedEventHandler,
    IUIAutomationFocusChangedEventHandler_Impl, IUIAutomationTextPattern, IUIAutomationTextPattern2,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationTextRange, IUIAutomationValuePattern,
    TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, UIA_EditControlTypeId,
    UIA_IsReadOnlyAttributeId, UIA_LegacyIAccessiblePatternId, UIA_TextPattern2Id, UIA_TextPatternId,
    UIA_ValuePatternId,
};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL};
use windows::Win32::UI::WindowsAndMessaging::STATE_SYSTEM_READONLY;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

use super::{
    caret_window, control_type_name, is_editable, record_to, results_path, Capture, CaptureMeta, EditSignals,
    MAX_FIELD_CHARS, SKIPPED_READ_ONLY,
};

/// How often the focused field is re-read between focus events: the fallback
/// that keeps capture working if focus events are late or missing, and what
/// follows typing within a field.
const POLL: Duration = Duration::from_millis(300);
/// How often the counters go to the log (counts and app names only).
const REPORT_EVERY: Duration = Duration::from_secs(30);
/// The window label of the debug panel.
pub const PANEL: &str = "capture-debug";

/// Wakes the capture thread on every focus change.
#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler(Sender<()>);

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(&self, _sender: Ref<'_, IUIAutomationElement>) -> WinResult<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

/// Starts the capture thread. Dev sessions only (the caller checks).
pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("glim-capture".into())
        .spawn(move || {
            if let Err(err) = run(&app) {
                crate::log::line(format!("capture: stopped: {err}"));
            }
        })
        .ok();
}

fn run(app: &AppHandle) -> WinResult<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)? };
    let (tx, rx) = channel();
    let handler: IUIAutomationFocusChangedEventHandler = FocusHandler(tx).into();
    unsafe { uia.AddFocusChangedEventHandler(None, &handler)? };
    crate::log::line("capture: subscribed to UIA focus changes (dev session)");

    // Glim's own process tree (glim.exe and its msedgewebview2.exe children):
    // nothing in it is ever read (super::OwnGuard).
    let mut guard = super::OwnGuard::default();
    let results = results_path();
    // The field being watched, and its last reading's metadata: recorded to
    // the results log when focus leaves it, so its char count is the final one.
    let mut current: Option<(IUIAutomationElement, CaptureMeta)> = None;
    // Diagnostics for the log: how many focus events and polls arrived, how
    // many readings went to the panel and by which path, and which apps were
    // seen. Never any text or title.
    let (mut events, mut polls, mut by_event, mut by_poll) = (0u32, 0u32, 0u32, 0u32);
    let mut apps: Vec<String> = Vec::new();
    let mut report_at = std::time::Instant::now() + REPORT_EVERY;

    loop {
        let via = match rx.recv_timeout(POLL) {
            Ok(()) => {
                events += 1;
                "event"
            }
            Err(RecvTimeoutError::Timeout) => {
                polls += 1;
                "poll"
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if std::time::Instant::now() >= report_at {
            crate::log::line(format!(
                "capture: last {}s: {events} focus events, {polls} polls; panel updates {by_event} by event, {by_poll} by poll; apps: {}",
                REPORT_EVERY.as_secs(),
                if apps.is_empty() { "none".to_string() } else { apps.join(", ") }
            ));
            (events, polls, by_event, by_poll) = (0, 0, 0, 0);
            apps.clear();
            report_at = std::time::Instant::now() + REPORT_EVERY;
        }
        let Ok(element) = (unsafe { uia.GetFocusedElement() }) else { continue };
        let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0);
        // Glim's own windows (the island, the panel, Settings, its webviews):
        // skipped by the element's process and by the foreground window's,
        // before anything about the element is read.
        if guard.is_own(pid as u32, foreground_pid(), std::time::Instant::now(), own_tree) {
            continue;
        }
        let mut capture = read(&element, pid);
        capture.via = via.into();
        let same = current
            .as_ref()
            .is_some_and(|(cur, _)| unsafe { uia.CompareElements(cur, &element) }.is_ok_and(|b| b.as_bool()));
        if !same {
            if let Some((_, meta)) = current.take() {
                let _ = record_to(&results, &meta);
            }
        }
        if !same && !apps.contains(&capture.meta.app) {
            apps.push(capture.meta.app.clone());
        }
        current = Some((element, capture.meta.clone()));
        if app.emit_to(PANEL, "capture-debug", &capture).is_ok() {
            if via == "event" { by_event += 1 } else { by_poll += 1 }
        }
    }
    Ok(())
}

/// Everything the panel shows about the focused element.
fn read(element: &IUIAutomationElement, pid: i32) -> Capture {
    let control = unsafe { element.CurrentControlType() }.map(|t| t.0).unwrap_or(0);
    let mut meta = CaptureMeta {
        app: process_name(pid),
        control_type: control_type_name(control),
        pattern: "none".into(),
        ..Default::default()
    };
    let window_title = foreground_title();

    if unsafe { element.CurrentIsPassword() }.is_ok_and(|b| b.as_bool()) {
        // Ignored completely: no pattern is asked for, nothing is read.
        meta.password = true;
        meta.pattern = "skipped (password)".into();
        return Capture { meta, window_title, excerpt: None, via: String::new() };
    }

    if !is_editable(&edit_signals(element, control)) {
        // Read-only content (a web page, a document view): the writing layer
        // never reads it. Like a password field, nothing is read.
        meta.read_only = true;
        meta.pattern = SKIPPED_READ_ONLY.into();
        return Capture { meta, window_title, excerpt: None, via: String::new() };
    }

    let (pattern, text, caret) = read_text(element);
    meta.pattern = pattern.into();
    meta.readable = text.is_some();
    meta.char_count = text.as_ref().map(|t| t.chars().count()).unwrap_or(0);
    meta.caret = caret;
    // The full text stays here and is dropped when this returns; only the
    // window around the caret goes to the panel.
    let excerpt = text.as_deref().map(|t| caret_window(t, caret));
    Capture { meta, window_title, excerpt, via: String::new() }
}

/// Whether the element can be typed in, from its patterns and states only:
/// ValuePattern.IsReadOnly, the IsReadOnly attribute of its text (an attribute
/// query, no text is read), the legacy read-only state, and its control type.
fn edit_signals(element: &IUIAutomationElement, control: i32) -> EditSignals {
    unsafe {
        let value_read_only = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .and_then(|v| v.CurrentIsReadOnly())
            .ok()
            .map(|b| b.as_bool());
        let text_read_only = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .and_then(|p| p.DocumentRange())
            .and_then(|r| r.GetAttributeValue(UIA_IsReadOnlyAttributeId))
            .ok()
            .and_then(|v| variant_bool(&v));
        let legacy_read_only = element
            .GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
            .and_then(|l| l.CurrentState())
            .is_ok_and(|state| state & STATE_SYSTEM_READONLY != 0);
        EditSignals { value_read_only, text_read_only, legacy_read_only, edit_control: control == UIA_EditControlTypeId.0 }
    }
}

/// A VARIANT's boolean, or None for anything else (UIA returns a special
/// "mixed" object when the range's text is partly read-only).
fn variant_bool(v: &VARIANT) -> Option<bool> {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        (inner.vt == VT_BOOL).then(|| inner.Anonymous.boolVal.as_bool())
    }
}

/// (pattern that worked, text, caret) — TextPattern2, then TextPattern, then
/// ValuePattern.
fn read_text(element: &IUIAutomationElement) -> (&'static str, Option<String>, Option<usize>) {
    unsafe {
        if let Ok(p2) = element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
            if let Ok(doc) = p2.DocumentRange() {
                if let Ok(text) = range_text(&doc) {
                    let mut active = BOOL::default();
                    let caret = p2.GetCaretRange(&mut active).ok().and_then(|c| offset_of(&doc, &c));
                    return ("TextPattern2", Some(text), caret);
                }
            }
        }
        if let Ok(p) = element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            if let Ok(doc) = p.DocumentRange() {
                if let Ok(text) = range_text(&doc) {
                    let caret = p
                        .GetSelection()
                        .ok()
                        .filter(|s| s.Length().unwrap_or(0) > 0)
                        .and_then(|s| s.GetElement(0).ok())
                        .and_then(|sel| offset_of(&doc, &sel));
                    return ("TextPattern", Some(text), caret);
                }
            }
        }
        if let Ok(v) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            if let Ok(value) = v.CurrentValue() {
                return ("ValuePattern", Some(value.to_string()), None);
            }
        }
    }
    ("none", None, None)
}

fn range_text(range: &IUIAutomationTextRange) -> WinResult<String> {
    Ok(unsafe { range.GetText(MAX_FIELD_CHARS)? }.to_string())
}

/// Characters from the start of `doc` to the start of `at`.
fn offset_of(doc: &IUIAutomationTextRange, at: &IUIAutomationTextRange) -> Option<usize> {
    unsafe {
        let before = doc.Clone().ok()?;
        before
            .MoveEndpointByRange(TextPatternRangeEndpoint_End, at, TextPatternRangeEndpoint_Start)
            .ok()?;
        Some(before.GetText(MAX_FIELD_CHARS).ok()?.to_string().chars().count())
    }
}

/// Glim's own process tree, from a Toolhelp snapshot of every process; only
/// processes whose image is one of OWN_IMAGES can join it. `None` if the
/// snapshot can't be taken (retried: ERROR_BAD_LENGTH is transient).
fn own_tree() -> Option<std::collections::HashSet<u32>> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    for _ in 0..3 {
        let mut pairs = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { continue };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let first = Process32FirstW(snap, &mut e).is_ok();
            let mut ok = first;
            while ok {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase();
                if super::OWN_IMAGES.contains(&name.as_str()) {
                    pairs.push((e.th32ProcessID, e.th32ParentProcessID));
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
            if !first {
                continue;
            }
        }
        return Some(super::process_tree(std::process::id(), &pairs));
    }
    crate::log::line("capture: process snapshot failed; skipping this reading");
    None
}

/// The process owning the foreground window.
fn foreground_pid() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid
}

/// The executable's file name for a process id.
fn process_name(pid: i32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32) else {
            return format!("pid {pid}");
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_FORMAT(0), windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return format!("pid {pid}");
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string()
    }
}

/// The foreground window's title (live panel only — never recorded).
fn foreground_title() -> String {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}
