// The Windows side of the text-capture spike: UI Automation, read-only.
//
// One thread owns every UIA object. It subscribes to focus-changed events (the
// handler only wakes the thread up, so no COM object crosses threads) and also
// re-reads the focused field every POLL so the debug panel follows typing.
// For each field: skip it entirely if its app is paused, if IsPassword, if
// its labels read like payment or ID data, or if it sits on a paused site;
// then skip it if it isn't editable (read-only page content, e.g. a whole web
// page in Chrome); only then try TextPattern2 (text + caret), then
// TextPattern (text + selection start as the caret), then ValuePattern (text
// only). A reading holding a card number, an SSN or an IBAN is dropped
// before it leaves this thread (super::finish_reading).

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::OnceLock;
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
    UIA_ValuePatternId, UIA_DocumentControlTypeId, UIA_FullDescriptionPropertyId,
};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR};
use windows::Win32::UI::WindowsAndMessaging::STATE_SYSTEM_READONLY;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

use super::sensitive::{is_browser, is_sensitive_field, FieldLabels, PauseList};
use super::{
    control_type_name, finish, is_editable, record_to, results_path, withheld, Capture, CaptureMeta, EditSignals,
    Reading, MAX_FIELD_CHARS, SKIPPED_OFF, SKIPPED_READ_ONLY,
};

/// How often the focused field is re-read between focus events: the fallback
/// that keeps capture working if focus events are late or missing, and what
/// follows typing within a field.
const POLL: Duration = Duration::from_millis(300);
/// How often the counters go to the log (counts and app names only).
const REPORT_EVERY: Duration = Duration::from_secs(30);
/// The window label of the debug panel.
pub const PANEL: &str = "capture-debug";

/// What wakes the capture thread.
enum Msg {
    /// A focus change.
    Focus,
    /// Something the island asked for.
    Request(Request),
}

/// What the island can ask the capture thread for (1b). Ids only: the words
/// to type or copy are the checker's own, kept in this thread.
pub enum Request {
    /// Type replacement `choice` of row `index` of list `check_id`.
    Fix { check_id: u64, index: usize, choice: usize },
    /// Copy it to the clipboard (fields Glim can't type in).
    Copy { check_id: u64, index: usize, choice: usize },
    /// Apply all: every row of list `check_id`, each with its first
    /// replacement, from the end backward (writing::Session::jobs_all).
    FixAll { check_id: u64 },
    /// Dismiss the list.
    Dismiss,
    /// 1c: rewrite the paragraph the caret is in, in this tone (a click).
    Rewrite { tone: crate::ai::claude::Tone },
    /// 1c: the API's answer for rewrite `id` (from the async runtime).
    RewriteDone { id: u64, reply: crate::ai::claude::Reply, elapsed: std::time::Duration },
    /// 1c: "Use this": type rewrite `id` in, through 1b's apply path.
    RewriteApply { id: u64 },
    /// 1c: copy rewrite `id`.
    RewriteCopy { id: u64 },
    /// 1c: the Rewrite card was closed.
    RewriteDismiss,
    /// "Check with AI" (§15.1): a click, or the automatic check.
    AiCheck { auto: bool },
    /// Its answer, from the async runtime.
    AiCheckDone { id: u64, reply: crate::ai::check::CheckReply, elapsed: std::time::Duration },
    /// Just look again now (Pause changed).
    Wake,
}

/// The capture thread's queue, for the island's requests.
static REQUESTS: OnceLock<Sender<Msg>> = OnceLock::new();

/// Hands a request to the capture thread. False if it isn't running.
pub fn request(r: Request) -> bool {
    REQUESTS.get().is_some_and(|tx| tx.send(Msg::Request(r)).is_ok())
}

/// Wakes the capture thread on every focus change.
#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler(Sender<Msg>);

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(&self, _sender: Ref<'_, IUIAutomationElement>) -> WinResult<()> {
        let _ = self.0.send(Msg::Focus);
        Ok(())
    }
}

/// Starts the capture thread. In every session it feeds the writing checker
/// for apps that have it on (crate::writing); in a dev session (`dev`) it
/// also reads every other app for the debug panel and keeps the results log.
pub fn start(app: AppHandle, dev: bool) {
    std::thread::Builder::new()
        .name("glim-capture".into())
        .spawn(move || {
            if let Err(err) = run(&app, dev) {
                crate::log::line(format!("capture: stopped: {err}"));
            }
        })
        .ok();
}

fn run(app: &AppHandle, dev: bool) -> WinResult<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)? };
    let (tx, rx) = channel();
    let _ = REQUESTS.set(tx.clone());
    let handler: IUIAutomationFocusChangedEventHandler = FocusHandler(tx).into();
    unsafe { uia.AddFocusChangedEventHandler(None, &handler)? };
    crate::log::line(if dev {
        "capture: subscribed to UIA focus changes (dev session: all apps, debug panel)"
    } else {
        "capture: subscribed to UIA focus changes (writing checker apps only)"
    });

    // Glim's own process tree (glim.exe and its msedgewebview2.exe children):
    // nothing in it is ever read (super::OwnGuard).
    let mut guard = super::OwnGuard::default();
    // Apps and sites never captured in: the built-ins and the user's own
    // (1d), rebuilt whenever the settings change.
    let prefs = crate::writing::prefs::get();
    let mut pause = PauseList::with_user(&prefs.pause_apps, &prefs.pause_sites);
    // The writing checker: the field's text stays in this thread.
    let mut writer = crate::writing::Session::default();
    let results = results_path();
    // The field being watched, and its last reading's metadata: recorded to
    // the results log (dev sessions only) when focus leaves it, so its char
    // count is the final one.
    let mut current: Option<(IUIAutomationElement, CaptureMeta)> = None;
    // The field whose paragraph the checker keeps for a fix (1b): its element,
    // its process, and the list's check id.
    let mut fix_field: Option<(IUIAutomationElement, u32)> = None;
    // The last field the checker read text from, and the one a rewrite (1c)
    // was asked in: "Use this" types only there.
    let mut last_field: Option<(IUIAutomationElement, u32)> = None;
    let mut rewrite_field: Option<(IUIAutomationElement, u32)> = None;
    let send_rewrite = |e: crate::ai::rewrite::RewriteEvent| {
        let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::ai::rewrite::EVENT, &e);
    };
    let send_ai = |e: crate::ai::check::CheckEvent| {
        let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::ai::check::EVENT, &e);
    };
    // The automatic check's last refusal before sending (no key, cap…):
    // logged once, not at every pause in typing.
    let mut auto_refused: Option<String> = None;
    let send_list = |s: &crate::writing::Suggestions| {
        let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::writing::EVENT, s);
    };
    let send_result = |outcome: crate::writing::Outcome, app_name: String, done: usize, total: usize, skipped: usize| {
        if total > 0 || skipped > 0 {
            crate::log::line(format!("writing: fix all {done}/{total} {}, {skipped} unclear skipped ({app_name})", outcome.code()));
        } else {
            crate::log::line(format!("writing: fix {} ({app_name})", outcome.code()));
        }
        let r = crate::writing::FixResult { outcome, app: app_name, done, total, skipped };
        let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::writing::RESULT_EVENT, &r);
    };
    // Diagnostics for the log: how many focus events and polls arrived, how
    // many readings went to the panel and by which path, and which apps were
    // seen. Never any text or title.
    let (mut events, mut polls, mut by_event, mut by_poll) = (0u32, 0u32, 0u32, 0u32);
    let mut apps: Vec<String> = Vec::new();
    let mut report_at = std::time::Instant::now() + REPORT_EVERY;
    // The writing settings in force (1d): a change drops what the checker
    // shows and keeps, and the next reading starts afresh.
    let mut prefs_version = crate::writing::prefs::version();
    // The one-time "Check writing in [app]?" prompt (1d, §14.2).
    let mut asker = crate::writing::ask::Asker::default();
    let mut docs_tip = crate::writing::docs_tip::DocsTip::default();

    loop {
        let via = match rx.recv_timeout(POLL) {
            Ok(Msg::Focus) => {
                events += 1;
                "event"
            }
            Ok(Msg::Request(r)) => {
                use crate::writing::Outcome;
                match r {
                    Request::Fix { check_id, index, choice } => {
                        let outcome = match writer.job(check_id, index, choice) {
                            Err(o) => o,
                            Ok(_) if crate::writing::paused() => Outcome::Paused,
                            Ok(job) if !crate::writing::offerable(&job.replacement) => Outcome::Blocked,
                            Ok(job) => match &fix_field {
                                Some((element, pid)) => {
                                    let mut field = super::fix::UiaField::new(&uia, element.clone(), *pid, &pause);
                                    crate::writing::apply::run(&mut field, &job)
                                }
                                None => Outcome::Away,
                            },
                        };
                        let name = writer.app();
                        // Whatever happened, the field is checked again at once.
                        writer.after_fix();
                        fix_field = None;
                        send_result(outcome, name, 0, 0, 0);
                    }
                    Request::FixAll { check_id } => {
                        let (done, total, skipped, outcome) = match writer.jobs_all(check_id) {
                            Err(o) => (0, 0, 0, o),
                            Ok((jobs, skipped)) if crate::writing::paused() => (0, jobs.len(), skipped, Outcome::Paused),
                            Ok((jobs, skipped)) => match &fix_field {
                                Some((element, pid)) => {
                                    let mut field = super::fix::UiaField::new(&uia, element.clone(), *pid, &pause);
                                    let (done, outcome) = crate::writing::apply::run_all(&mut field, &jobs);
                                    (done, jobs.len(), skipped, outcome)
                                }
                                None => (0, jobs.len(), skipped, Outcome::Away),
                            },
                        };
                        let name = writer.app();
                        writer.after_fix();
                        fix_field = None;
                        send_result(outcome, name, done, total, skipped);
                    }
                    Request::Copy { check_id, index, choice } => {
                        let outcome = match writer.copy_text(check_id, index, choice) {
                            Ok(t) if super::fix::copy_to_clipboard(&t) => Outcome::Copied,
                            Ok(_) => Outcome::ClipboardBusy,
                            Err(o) => o,
                        };
                        send_result(outcome, writer.app(), 0, 0, 0);
                    }
                    Request::Dismiss => {
                        writer.dismiss(send_list);
                        fix_field = None;
                    }
                    Request::Rewrite { tone } => {
                        use crate::ai::{claude, key, rewrite, spend};
                        let config = rewrite::config();
                        let saved = key::load(&key::vault());
                        let has_key = saved.as_ref().map(Option::is_some).map_err(Clone::clone);
                        let spent = spend::current(&spend::path(), &spend::month_now());
                        let begun = rewrite::precheck(crate::writing::paused(), &config, &has_key, &spent)
                            .map_err(String::from)
                            .and_then(|()| {
                                writer.rewrite_begin(std::time::Instant::now()).map_err(|o| {
                                    // Withheld as sensitive: nothing of it was read or sent.
                                    if o == crate::writing::Outcome::Blocked { "sensitive" } else { "no-paragraph" }.to_string()
                                })
                            });
                        match (begun, saved) {
                            (Ok(src), Ok(Some(api_key))) => {
                                rewrite_field = last_field.clone();
                                send_rewrite(rewrite::RewriteEvent::sending(src.id, tone, config.model));
                                let (id, model, paragraph) = (src.id, config.model, src.paragraph);
                                tauri::async_runtime::spawn(async move {
                                    let started = std::time::Instant::now();
                                    let reply = match crate::net::Url::parse(claude::API_BASE) {
                                        Ok(base) => claude::rewrite(&base, &api_key, model, tone, &paragraph).await,
                                        Err(_) => claude::Reply { result: Err(claude::Failure::Network), usage: None },
                                    };
                                    request(Request::RewriteDone { id, reply, elapsed: started.elapsed() });
                                });
                            }
                            (Err(code), _) => {
                                crate::log::line(format!("rewrite: not sent ({code})"));
                                send_rewrite(rewrite::RewriteEvent::failed(0, code));
                            }
                            // precheck passed, so the key was there a moment ago.
                            (Ok(src), _) => {
                                writer.rewrite_clear();
                                send_rewrite(rewrite::RewriteEvent::failed(src.id, "no-key"));
                            }
                        }
                    }
                    Request::RewriteDone { id, reply, elapsed } => {
                        use crate::ai::{claude, rewrite, spend};
                        crate::log::line(claude::log_line(&reply, elapsed));
                        if let Some(usage) = &reply.usage {
                            if let Err(err) = spend::record(&spend::path(), &spend::month_now(), usage) {
                                crate::log::line(format!("rewrite: could not record the spend: {err}"));
                            }
                        }
                        let shown = match &reply.result {
                            Ok(text) => writer.rewrite_answer(id, text.clone()),
                            Err(_) => {
                                if writer.rewrite_is(id) {
                                    writer.rewrite_clear();
                                }
                                None
                            }
                        };
                        send_rewrite(rewrite::RewriteEvent::answered(id, &reply, shown.as_ref()));
                    }
                    Request::RewriteApply { id } => {
                        let outcome = match writer.rewrite_job(id) {
                            Err(o) => o,
                            Ok(_) if crate::writing::paused() => Outcome::Paused,
                            Ok(job) => match &rewrite_field {
                                Some((element, pid)) => {
                                    let mut field = super::fix::UiaField::new(&uia, element.clone(), *pid, &pause);
                                    crate::writing::apply::run(&mut field, &job)
                                }
                                None => Outcome::Away,
                            },
                        };
                        let name = writer.rewrite_app();
                        crate::log::line(format!("writing: rewrite {} ({name})", outcome.code()));
                        if outcome == Outcome::Applied {
                            // The paragraph changed: checked again at once.
                            writer.rewrite_clear();
                            writer.after_fix();
                            rewrite_field = None;
                            fix_field = None;
                        }
                        send_rewrite(crate::ai::rewrite::RewriteEvent::result(id, outcome, name));
                    }
                    Request::RewriteCopy { id } => {
                        let outcome = match writer.rewrite_copy(id) {
                            Ok(t) if super::fix::copy_to_clipboard(&t) => Outcome::Copied,
                            Ok(_) => Outcome::ClipboardBusy,
                            Err(o) => o,
                        };
                        send_rewrite(crate::ai::rewrite::RewriteEvent::result(id, outcome, writer.rewrite_app()));
                    }
                    Request::RewriteDismiss => {
                        writer.rewrite_clear();
                        rewrite_field = None;
                    }
                    Request::AiCheck { auto } => {
                        use crate::ai::{check, claude, key, rewrite, spend};
                        let config = rewrite::config();
                        let saved = key::load(&key::vault());
                        let has_key = saved.as_ref().map(Option::is_some).map_err(Clone::clone);
                        let spent = spend::current(&spend::path(), &spend::month_now());
                        let begun = rewrite::precheck(crate::writing::paused(), &config, &has_key, &spent)
                            .map_err(String::from)
                            .and_then(|()| writer.ai_begin(auto).map_err(|o| if o == crate::writing::Outcome::Blocked { "sensitive".to_string() } else { "no-paragraph".to_string() }));
                        match (begun, saved) {
                            (Ok(src), Ok(Some(api_key))) => {
                                auto_refused = None;
                                send_ai(check::CheckEvent { phase: "sending", id: src.id, auto, ..Default::default() });
                                let (id, paragraph) = (src.id, src.paragraph);
                                tauri::async_runtime::spawn(async move {
                                    let started = std::time::Instant::now();
                                    let reply = match crate::net::Url::parse(claude::API_BASE) {
                                        Ok(base) => check::check(&base, &api_key, &paragraph).await,
                                        Err(_) => check::CheckReply { result: Err(claude::Failure::Network), usage: None, dropped: 0 },
                                    };
                                    request(Request::AiCheckDone { id, reply, elapsed: started.elapsed() });
                                });
                            }
                            (Err(code), _) if auto => {
                                if auto_refused.as_deref() != Some(code.as_str()) {
                                    crate::log::line(format!("ai-check: not sent ({code}) (auto)"));
                                    auto_refused = Some(code);
                                }
                            }
                            (Err(code), _) => {
                                crate::log::line(format!("ai-check: not sent ({code}) (click)"));
                                send_ai(check::CheckEvent { phase: "failed", code, ..Default::default() });
                            }
                            (Ok(src), _) => {
                                writer.ai_clear(src.id);
                                send_ai(check::CheckEvent { phase: "failed", id: src.id, auto, code: "no-key".into(), ..Default::default() });
                            }
                        }
                    }
                    Request::AiCheckDone { id, reply, elapsed } => {
                        use crate::ai::{check, rewrite, spend};
                        let auto = writer.ai_pending(id).unwrap_or(false);
                        crate::log::line(check::log_line(&reply, elapsed, auto));
                        if let Some(usage) = &reply.usage {
                            if let Err(err) = spend::record(&spend::path(), &spend::month_now(), usage) {
                                crate::log::line(format!("ai-check: could not record the spend: {err}"));
                            }
                        }
                        let event = match &reply.result {
                            Ok(issues) => match writer.ai_answer(id, issues, send_list) {
                                Some(added) => check::CheckEvent { phase: "done", id, auto, added, ..Default::default() },
                                // The paragraph changed meanwhile: nothing to show.
                                None => check::CheckEvent { phase: "failed", id, auto, code: "changed".into(), ..Default::default() },
                            },
                            Err(f) => {
                                writer.ai_clear(id);
                                let retry_after = match f {
                                    crate::ai::claude::Failure::Api { retry_after, .. } => *retry_after,
                                    _ => None,
                                };
                                check::CheckEvent { phase: "failed", id, auto, code: rewrite::failure_code(f), retry_after, ..Default::default() }
                            }
                        };
                        send_ai(event);
                    }
                    Request::Wake => {}
                }
                "request"
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
            if let Some(r) = writer.take_report() {
                crate::log::line(r);
            }
            (events, polls, by_event, by_poll) = (0, 0, 0, 0);
            apps.clear();
            report_at = std::time::Instant::now() + REPORT_EVERY;
        }
        if crate::writing::prefs::version() != prefs_version {
            prefs_version = crate::writing::prefs::version();
            let prefs = crate::writing::prefs::get();
            pause = PauseList::with_user(&prefs.pause_apps, &prefs.pause_sites);
            writer.settings_changed(send_list);
            fix_field = None;
            last_field = None;
            rewrite_field = None;
        }
        // Paused: nothing is read at all, and the checker drops what it kept.
        if crate::writing::paused() {
            let r = crate::writing::Observed { same: false, text: None, caret: None, rules: crate::writing::Rules::Full, mode: crate::writing::FixMode::View, app: "", paused: true, sensitive: false };
            writer.observe_in(&r, std::time::Instant::now(), send_list);
            fix_field = None;
            continue;
        }
        let Ok(element) = (unsafe { uia.GetFocusedElement() }) else { continue };
        let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0);
        // Glim's own windows (the island, the panel, Settings, its webviews):
        // skipped by the element's process and by the foreground window's,
        // before anything about the element is read. The island keeps its
        // suggestions while the user clicks it.
        if guard.is_own(pid as u32, foreground_pid(), std::time::Instant::now(), own_tree) {
            continue;
        }
        let reading = read(&uia, &element, pid, &pause, dev);
        let ask = reading.ask;
        let mail_site = reading.mail_site;
        // Google Docs with no text to check (canvas rendering, screen reader
        // support off): one metadata line per stay, and the one-time tip.
        let docs_unreadable = reading.docs_site
            && crate::writing::apps::enabled(&reading.capture.meta.app)
            && crate::writing::docs_tip::unreadable(&reading.capture.meta.pattern, reading.capture.meta.read_only, reading.text.as_deref());
        let step = docs_tip.feed(docs_unreadable, crate::writing::prefs::get().docs_tip_dismissed, std::time::Instant::now());
        if step.log {
            crate::log::line(format!(
                "writing: Google Docs field not readable (pattern {}, read-only {})",
                reading.capture.meta.pattern, reading.capture.meta.read_only
            ));
        }
        if step.offer {
            crate::log::line("writing: Google Docs tip offered");
            let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::writing::docs_tip::EVENT, ());
        }
        let mut capture = reading.capture;
        capture.via = via.into();
        let same = current
            .as_ref()
            .is_some_and(|(cur, _)| unsafe { uia.CompareElements(cur, &element) }.is_ok_and(|b| b.as_bool()));
        if !same {
            if let Some((_, meta)) = current.take() {
                if dev {
                    let _ = record_to(&results, &meta);
                }
            }
        }
        let askable = ask.then_some(capture.meta.app.as_str());
        if let Some(exe) = asker.feed(askable, same, std::time::Instant::now()) {
            let name = app_description(pid).unwrap_or_else(|| crate::writing::apps::display_name(&exe));
            crate::log::line(format!("writing: asking about {exe}"));
            let _ = app.emit_to(crate::island::WINDOW_LABEL, crate::writing::ask::EVENT, &crate::writing::ask::Ask { exe, name });
        }
        if !same && !apps.contains(&capture.meta.app) {
            apps.push(capture.meta.app.clone());
        }
        // The checker only ever sees apps that have it on.
        let checked = reading.text.as_deref().filter(|_| crate::writing::apps::enabled(&capture.meta.app));
        let name = crate::writing::apps::display_name(&capture.meta.app);
        let r = crate::writing::Observed {
            same,
            text: checked,
            caret: capture.meta.caret,
            rules: crate::writing::apps::rules(&capture.meta.app),
            mode: crate::writing::fix_mode(&capture.meta.app, &capture.meta.pattern, capture.meta.chat_site),
            app: &name,
            paused: false,
            // Withheld for a card, SSN or IBAN (or a payment field), in an
            // app the checker is on for.
            sensitive: capture.meta.sensitive && crate::writing::apps::enabled(&capture.meta.app),
        };
        writer.observe_in(&r, std::time::Instant::now(), send_list);
        // "Check with AI automatically when I pause typing" (§15.1, off by
        // default): once per paragraph version, the same path as a click.
        if checked.is_some() && crate::writing::prefs::get().ai_auto_check && writer.ai_auto_take(std::time::Instant::now()) {
            request(Request::AiCheck { auto: true });
        }
        // A kept paragraph always belongs to the field just read (moving
        // anywhere else drops it).
        fix_field = writer.kept_check().map(|_| (element.clone(), pid as u32));
        if checked.is_some() {
            last_field = Some((element.clone(), pid as u32));
            let cat = crate::writing::tones::category(&capture.meta.app, capture.meta.chat_site, mail_site);
            crate::writing::tones::set_focus(&capture.meta.app, cat);
        }
        current = Some((element, capture.meta.clone()));
        if dev && app.emit_to(PANEL, "capture-debug", &capture).is_ok() {
            if via == "event" { by_event += 1 } else { by_poll += 1 }
        }
    }
    Ok(())
}

/// Everything the panel shows about the focused element, and its text for
/// the writing checker. Outside dev sessions, apps the checker is off for
/// are not read at all.
fn read(uia: &IUIAutomation, element: &IUIAutomationElement, pid: i32, pause: &PauseList, dev: bool) -> Reading {
    let control = unsafe { element.CurrentControlType() }.map(|t| t.0).unwrap_or(0);
    let app = process_name(pid);
    if pause.pauses_app(&app) {
        // A paused app (a password manager): nothing about the field is read.
        return withheld(app, control_type_name(control), true).into();
    }
    let mut meta = CaptureMeta { app, control_type: control_type_name(control), pattern: "none".into(), ..Default::default() };
    // An app on neither list (1d, §14.2): whether its field is one to ask
    // about, from the password flag and the editable signals only.
    let ask = !crate::writing::apps::enabled(&meta.app)
        && crate::writing::ask::askable(&meta.app, &crate::writing::prefs::get(), &crate::writing::ask::today())
        && !unsafe { element.CurrentIsPassword() }.is_ok_and(|b| b.as_bool())
        && is_editable(&edit_signals(element, control));
    if !dev && !crate::writing::apps::enabled(&meta.app) {
        meta.pattern = SKIPPED_OFF.into();
        let mut off: Reading = Capture { meta, window_title: String::new(), excerpt: None, via: String::new() }.into();
        off.ask = ask;
        return off;
    }

    if unsafe { element.CurrentIsPassword() }.is_ok_and(|b| b.as_bool()) {
        // Ignored completely: no pattern is asked for, nothing is read.
        meta.password = true;
        meta.pattern = "skipped (password)".into();
        return Capture { meta, window_title: String::new(), excerpt: None, via: String::new() }.into();
    }

    if is_sensitive_field(&field_labels(element)) {
        // A payment or ID field, by its labels: nothing is read.
        return withheld(meta.app, meta.control_type, false).into();
    }
    let urls = if is_browser(&meta.app) { page_urls(uia, element) } else { Vec::new() };
    if urls.iter().any(|url| pause.pauses_url(url)) {
        // A field on a paused site, or in a frame from one (a card form from
        // a payment provider inside a shop's page).
        return withheld(meta.app, meta.control_type, true).into();
    }
    // A web chat app (claude.ai in Chrome): read and checked like the desktop
    // chat apps, and view-only like them.
    meta.chat_site = urls.iter().any(|url| crate::writing::apps::chat_site(url));
    // The window title is for the dev panel only.
    let window_title = if dev { foreground_title() } else { String::new() };

    let docs_site = urls.iter().any(|url| super::sensitive::url_on_sites(url, &crate::writing::docs_tip::DOCS_SITES));
    if !is_editable(&edit_signals(element, control)) {
        // Read-only content (a web page, a document view): the writing layer
        // never reads it. Like a password field, nothing is read.
        meta.read_only = true;
        meta.pattern = SKIPPED_READ_ONLY.into();
        let mut skipped: Reading = Capture { meta, window_title, excerpt: None, via: String::new() }.into();
        skipped.docs_site = docs_site;
        return skipped;
    }

    let mail_site = urls.iter().any(|url| super::sensitive::url_on_sites(url, &crate::writing::tones::MAIL_SITES));
    let (pattern, text, caret) = read_text(element);
    meta.pattern = pattern.into();
    // Card numbers, SSNs and IBANs in the text are caught here, before the
    // reading goes anywhere.
    let mut reading = finish(meta, window_title, text, caret);
    reading.mail_site = mail_site;
    reading.docs_site = docs_site;
    reading
}

/// The field's labels from UI Automation, never its text.
pub(super) fn field_labels(element: &IUIAutomationElement) -> FieldLabels {
    unsafe {
        let s = |r: WinResult<windows::core::BSTR>| r.map(|b| b.to_string()).unwrap_or_default();
        FieldLabels {
            name: s(element.CurrentName()),
            labeled_by: element.CurrentLabeledBy().map(|l| s(l.CurrentName())).unwrap_or_default(),
            help_text: s(element.CurrentHelpText()),
            description: element
                .GetCurrentPropertyValue(UIA_FullDescriptionPropertyId)
                .ok()
                .and_then(|v| variant_string(&v))
                .unwrap_or_default(),
            automation_id: s(element.CurrentAutomationId()),
        }
    }
}

/// The URLs of the pages and frames a browser field sits in: browsers give
/// each Document (the page, and each frame) its URL as its Value. Read only to
/// match against the pause list; never sent anywhere.
pub(super) fn page_urls(uia: &IUIAutomation, element: &IUIAutomationElement) -> Vec<String> {
    let mut urls = Vec::new();
    let Ok(walker) = (unsafe { uia.ControlViewWalker() }) else { return urls };
    let mut cur = element.clone();
    for _ in 0..64 {
        unsafe {
            if cur.CurrentControlType().is_ok_and(|t| t == UIA_DocumentControlTypeId) {
                if let Ok(value) = cur
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .and_then(|v| v.CurrentValue())
                {
                    urls.push(value.to_string());
                }
            }
            match walker.GetParentElement(&cur) {
                Ok(parent) => cur = parent,
                Err(_) => break,
            }
        }
    }
    urls
}

/// Whether the element can be typed in, from its patterns and states only:
/// ValuePattern.IsReadOnly, the IsReadOnly attribute of its text (an attribute
/// query, no text is read), the legacy read-only state, and its control type.
pub(super) fn edit_signals(element: &IUIAutomationElement, control: i32) -> EditSignals {
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

/// A VARIANT's string, or None for anything else.
fn variant_string(v: &VARIANT) -> Option<String> {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        (inner.vt == VT_BSTR).then(|| inner.Anonymous.bstrVal.to_string())
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
pub(super) fn offset_of(doc: &IUIAutomationTextRange, at: &IUIAutomationTextRange) -> Option<usize> {
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
pub(super) fn foreground_pid() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid
}

/// The executable's file name for a process id.
pub(super) fn process_name(pid: i32) -> String {
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

/// The app's name from its executable's version info (FileDescription,
/// "Adobe Acrobat"), for the one-time prompt. None when it has none.
fn app_description(pid: i32) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let path = process_path(pid)?;
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;
        // The first language and code page the file lists.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let query: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
        let key = format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription\0", pair[0], pair[1]);
        let query: Vec<u16> = key.encode_utf16().collect();
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let chars = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
        let name = String::from_utf16_lossy(&chars[..end]).trim().to_string();
        (!name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)).then_some(name)
    }
}

/// The executable's full path for a process id.
fn process_path(pid: i32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_FORMAT(0), windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
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
