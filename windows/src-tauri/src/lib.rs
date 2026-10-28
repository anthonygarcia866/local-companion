// Glim for Windows — app wiring and the commands the island calls.

mod agent_hooks;
mod ai;
mod autostart;
mod agents;
mod capture;
mod chat;
mod config_file;
mod desktop;
mod files;
mod hooks;
mod i18n;
mod identity;
mod island;
mod local_chat;
mod log;
mod net;
mod pipe;
mod platform;
mod recap;
mod resume;
mod screen_capture;
mod learn;
mod memory;
mod session_window;
mod settings;
mod shortcuts;
mod placement;
mod pause;
mod phone;
mod presence;
mod tray;
#[cfg(windows)]
mod webview_drop;
mod webview_health;
mod relaunch;
#[cfg(windows)]
mod bubble_window;
mod writing;
mod voice;
mod calendar;
#[cfg(windows)]
mod outlook;
#[cfg(all(test, windows))]
mod outlook_live;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use chat::{Chat, ChatContext, ChatReply, ModelInfo};
use files::DroppedFile;
use hooks::{HookPreview, HookStatus};
use island::{PollGate, ScreenInfo};
use pipe::Pending;
use settings::Settings;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
    /// False where the OS has no global cursor (Wayland): the page then reports
    /// the cursor from its own mouse events.
    cursor_poll: bool,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    let mut settings = shared.settings.lock().unwrap().clone();
    // The real state of ~/.claude/settings.json wins over whatever we stored.
    let hooks_status = hooks::status();
    settings.hooks_installed = hooks_status.installed;
    settings.plan_relay_installed = hooks_status.plan_relay_installed;
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
        cursor_poll: platform::CURSOR_POLL,
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, settings: Settings) {
    let (screen_changed, autostart_changed, shortcuts_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        let shortcuts_changed = current.shortcuts != settings.shortcuts;
        // Where the character sits on the desktop is desktop.rs's to say, not a
        // webview's; so are the docks (island.rs, set by dragging the pill).
        let mut settings = settings.clone();
        settings.desktop_mochi = current.desktop_mochi.clone();
        settings.docks = current.docks.clone();
        settings.visibility = current.visibility.clone();
        // When each app was asked about is Rust's (writing_ask_shown/_answer).
        settings.writing_apps_asked = current.writing_apps_asked.clone();
        settings.writing_docs_tip_dismissed = current.writing_docs_tip_dismissed;
        // The draft card's remembered picks are Rust's (voice::draft_worker,
        // draft_forget).
        settings.draft_recipients = current.draft_recipients.clone();
        // The day the calendar's morning summary was last shown is Rust's.
        settings.calendar_summary_shown = current.calendar_summary_shown.clone();
        let calendar_changed = (current.calendar_on, current.calendar_hide_all_day, current.calendar_summary, &current.calendar_ids)
            != (settings.calendar_on, settings.calendar_hide_all_day, settings.calendar_summary, &settings.calendar_ids);
        *current = settings;
        if calendar_changed {
            calendar::worker::refresh(&app);
        }
        (screen_changed, autostart_changed, shortcuts_changed)
    };
    let settings = shared.settings.lock().unwrap().clone();
    shared.gate.drop_zone.store(settings.file_drop_zone, std::sync::atomic::Ordering::Relaxed);
    ai::rewrite::set_config(ai::rewrite::Config::from_settings(&settings));
    writing::prefs::set(writing::prefs::Prefs::from_settings(&settings));
    phone::apply_settings(&settings);
    screen_capture::apply(&app);
    if let Err(err) = settings::save(&settings) {
        log::line(format!("could not save settings: {err}"));
    }
    if autostart_changed {
        autostart::sync(settings.autostart, true);
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        island::apply_geometry(&app, &settings.screen, collapsed);
    }
    if shortcuts_changed {
        shortcuts::apply(&app, &settings.shortcuts);
    }
    if i18n::set_picked(&settings.language) {
        language_changed(&app);
    }
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
}

/// The island reports the system's languages at launch, for "System" in
/// Settings → Language (WebView2 and WebKitGTK know them best).
#[tauri::command]
fn set_system_languages(app: AppHandle, languages: Vec<String>) {
    if i18n::set_system(languages) {
        language_changed(&app);
    }
}

/// What Rust labels itself follows the new language: the tray menu and the
/// settings window's title. The webviews switch on their own.
fn language_changed(app: &AppHandle) {
    tray::retitle(app);
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_title(&i18n::t("Settings — Glim"));
    }
}

/// Hidden island → shrink the window to the invisible wake strip and park the
/// cursor poll; anything else → full panel and 60 Hz polling.
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, collapsed: bool) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
    // The wake strip must always take the mouse, and a resize invalidates the flag.
    island::refresh_click_through(&app, &shared.gate);
    shared.gate.set_active(!collapsed);
    platform::set_pointer_watch(!collapsed);
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(app: AppHandle, shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
    // Without the cursor poll the input region is the click-through: it follows the island.
    if !platform::CURSOR_POLL {
        island::refresh_click_through(&app, &shared.gate);
    }
}

#[tauri::command]
fn focus_window(app: AppHandle, caller: tauri::WebviewWindow, focused: bool) {
    // Only the island's own page asks for its chat field: no other window
    // (Settings, the desktop character) may hand it the keyboard.
    if caller.label() != island::WINDOW_LABEL {
        log::line(format!("focus_window refused from window {:?}", caller.label()));
        return;
    }
    let Some(win) = island::window(&app) else { return };
    // Through tao (FOCUSABLE): tao rewrites the extended styles on every flag
    // change (click-through toggles, show/hide) from its own flags, so a
    // WS_EX_NOACTIVATE set behind its back was wiped and the island took focus
    // when shown again. The raw toggle stays for Linux and the first frame.
    let _ = win.set_focusable(focused);
    platform::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
    }
}

/// The window a box on a card borrowed the keyboard from (lend_keyboard).
static LENT_FROM: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);

/// A box on a card (the question card's answer, the draft card's search)
/// borrows the keyboard on a click, and gives it back on Send, Cancel, a pick
/// or the card closing: to the window the user was typing in before, so their
/// next keys land there again (retest, 2026-10-17: they went to Outlook).
#[tauri::command]
fn lend_keyboard(app: AppHandle, caller: tauri::WebviewWindow, on: bool) {
    if caller.label() != island::WINDOW_LABEL {
        log::line(format!("lend_keyboard refused from window {:?}", caller.label()));
        return;
    }
    let Some(win) = island::window(&app) else { return };
    if on {
        // Asked again while the island already has it: keep the first one.
        if let Some(front) = platform::foreground_other_than(&win) {
            *LENT_FROM.lock().unwrap() = Some(front);
        }
        let _ = win.set_focusable(true);
        platform::set_activating(&win, true);
        let _ = win.set_focus();
    } else {
        let _ = win.set_focusable(false);
        platform::set_activating(&win, false);
        if let Some(to) = LENT_FROM.lock().unwrap().take() {
            platform::give_foreground_back(&win, to);
        }
    }
}

/// The pill was pressed and moved: the window follows the cursor from here.
#[tauri::command]
fn dock_drag_start(app: AppHandle) {
    island::start_dock_drag(&app);
}

/// Settings' position picker: the dock on the island's own display.
#[tauri::command]
fn set_dock(app: AppHandle, dock: String) {
    if let Some(dock) = placement::Dock::parse(&dock) {
        island::set_dock(&app, None, dock);
    }
}

/// The ember's click with writing suggestions waiting, and the island
/// closing after it: the pill without changing the saved mode.
#[tauri::command]
fn set_peek(app: AppHandle, on: bool) {
    presence::set_peek(&app, on);
}

/// Settings' visibility picker and the ember's click (back to "normal").
#[tauri::command]
fn set_visibility(app: AppHandle, visibility: String) {
    if let Some(v) = presence::Visibility::parse(&visibility) {
        presence::set_visibility(&app, v);
    }
}

/// What is on screen (pill, ember, hidden, recording indicator), for the page.
#[tauri::command]
fn presence_info(app: AppHandle) -> presence::PresencePayload {
    presence::payload(&app)
}

/// Where the island is docked on its display, for the page's layout.
#[tauri::command]
fn placement() -> island::PlacementPayload {
    let dock = island::current_dock();
    island::PlacementPayload { dock: dock.as_str(), vertical: dock.vertical() }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
}

/// The displays the island can be pinned to, for Settings.
#[tauri::command]
fn list_monitors(app: AppHandle) -> Vec<island::MonitorChoice> {
    island::monitor_choices(&app)
}

/// Opens a link in the browser — only a page on this machine (net/'s
/// allowlist), so the app never hands the browser an address off the machine.
#[tauri::command]
fn open_url(url: String) {
    // Loopback only: the allowlist's Anthropic host is for Rust's API calls,
    // not for pages.
    match net::check_str(&url) {
        Ok(url) if net::is_loopback_url(&url) => platform::open_url(url.as_str()),
        Ok(url) => log::line(net::local_refusal(&net::host_for_log(&url))),
        Err(err) => log::line(err),
    }
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to the file manager otherwise.
#[tauri::command]
fn open_in_vscode(path: Option<String>) -> bool {
    // No shell anywhere near this. The path is a project folder chosen by
    // whoever is using Claude Code, and a shell would happily read `&`, `^`, `%`
    // or `$` in a folder name as syntax. Finding the launcher ourselves and
    // handing the path over as a separate argument keeps it a path.
    let path = path.filter(|p| !p.is_empty());
    // It arrives in a hook payload: only an existing folder, given by its full
    // path, goes any further. `code` would read `--something` as an option, and
    // xdg-open would launch a file with whatever handles its type.
    if let Some(p) = path.as_deref() {
        let p = std::path::Path::new(p);
        if !(p.is_absolute() && p.is_dir()) {
            return false;
        }
    }
    if let Some(code) = platform::find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path.as_deref() {
            cmd.arg(p);
        }
        if platform::no_console(&mut cmd).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path.as_deref() {
        platform::reveal_folder(p);
    }
    false
}

// ── "Pick up from yesterday" (resume/, PR #21) ─────────────────────────────

/// The resume card's rows: each repo's HANDOFF.md, newest first. Logged as
/// names and counts only (resume::summary_line).
#[tauri::command]
fn resume_cards() -> Vec<resume::Card> {
    let scan = resume::read_all(&resume::repos(&[]));
    log::line(resume::summary_line(&scan));
    resume::cards(&scan)
}

/// A click on a repo's row: Claude Code opens in that repo, told to read
/// HANDOFF.md and continue (owner, 2026-10-13: only on that click). The
/// island sends the repo's name; the folder is looked up among the repos Glim
/// reads, never taken from the island.
#[tauri::command]
fn resume_open(repo: String) -> bool {
    let Some(folder) = resume::folder_for(&repo, &resume::repos(&[])) else {
        log::line("resume: open refused (not a repo Glim reads)");
        return false;
    };
    let opened = platform::open_claude_code_in(&folder, resume::PROMPT);
    log::line(format!("resume: {} Claude Code in {repo}", if opened { "opened" } else { "could not open" }));
    opened
}

/// "Open terminal": brings forward the terminal or editor window the session
/// runs in, when it was found (Windows, see session_window.rs); otherwise opens
/// the folder in VS Code, as before.
#[tauri::command]
fn open_session(session_id: Option<String>, path: Option<String>) -> bool {
    if let Some(owner) = session_id.as_deref().and_then(session_window::lookup) {
        let folder = path.as_deref().map(session_window::folder_name).unwrap_or_default();
        if platform::focus_process_window(owner, folder) {
            return true;
        }
    }
    open_in_vscode(path)
}

/// The Claude Desktop pill's target: the Claude app (Windows only — it has no
/// Linux build).
#[tauri::command]
fn open_claude_desktop() -> bool {
    platform::open_claude_desktop()
}

/// The file behind a live diff, if it may be handed to the editor: an existing
/// regular file given by its full path. Anything else — a relative path, a
/// folder, a path `code` could read as an option — goes no further.
fn diff_file(path: &str) -> Option<&std::path::Path> {
    let p = std::path::Path::new(path);
    (p.is_absolute() && p.is_file()).then_some(p)
}

/// The diff card's ↗: opens the edited file in VS Code when `code` is on PATH,
/// otherwise shows its folder. The file itself is never opened by its type —
/// xdg-open or Explorer would run a script that Claude just wrote.
#[tauri::command]
fn open_file_in_vscode(path: String) -> bool {
    let Some(file) = diff_file(&path) else { return false };
    if let Some(code) = platform::find_on_path("code") {
        let mut cmd = Command::new(code);
        cmd.arg(file);
        if platform::no_console(&mut cmd).spawn().is_ok() {
            return true;
        }
    }
    if let Some(folder) = file.parent().filter(|d| d.is_dir()) {
        platform::reveal_folder(&folder.to_string_lossy());
    }
    false
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    ai::budget::on_quit();
    #[cfg(windows)]
    voice::whisper::stop();
    app.exit(0);
}

// ── Claude Code hooks ─────────────────────────────────────────────────────────

#[tauri::command]
fn hooks_status() -> HookStatus {
    hooks::status()
}

/// Pill ID → whether that agent's hooks reach the app. Read-only.
#[tauri::command]
fn agent_hooks_status() -> std::collections::HashMap<String, bool> {
    agent_hooks::status()
}

/// Returns the diff the user has to look at before anything is written.
#[tauri::command]
fn hooks_preview(install: bool) -> Result<HookPreview, String> {
    hooks::preview(install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn hooks_apply(
    app: AppHandle,
    shared: State<Shared>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    // The fingerprint comes from the preview the user actually looked at, so a
    // settings.json that changed in between is refused rather than overwritten.
    let backup = hooks::write(install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.hooks_installed = install;
        if let Err(err) = settings::save(&current) {
            log::line(format!("could not save settings: {err}"));
        }
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

// ── Other agents' hooks and plugins ──────────────────────────────────────────

#[tauri::command]
fn agent_hooks_list() -> Vec<agents::AgentStatus> {
    agents::list()
}

/// The diff the user has to look at before anything is written.
#[tauri::command]
fn agent_hooks_preview(agent: String, install: bool) -> Result<config_file::Plan, String> {
    agents::preview(&agent, install)
}

/// Only ever called from an explicit click in the settings window, with the
/// fingerprint of the preview the user looked at.
#[tauri::command]
fn agent_hooks_apply(agent: String, install: bool, fingerprint: String) -> Result<String, String> {
    let backups = agents::apply(&agent, install, &fingerprint)?;
    let done = if install { "installed" } else { "removed" };
    log::line(format!("agent hooks {done} for {agent}"));
    Ok(backups)
}

// ── Plan usage ────────────────────────────────────────────────────────────────

/// The diff of putting the plan usage relay into (or taking it out of) the
/// status line, before anything is written.
#[tauri::command]
fn status_line_preview(install: bool) -> Result<HookPreview, String> {
    hooks::status_line_preview(install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn status_line_apply(
    app: AppHandle,
    shared: State<Shared>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    let backup = hooks::status_line_write(install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.plan_relay_installed = install;
        // As on the Mac: taking the relay out turns the pill off with it.
        if !install {
            current.show_plan_in_notch = false;
        }
        if let Err(err) = settings::save(&current) {
            log::line(format!("could not save settings: {err}"));
        }
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

#[tauri::command]
fn approval_decision(app: AppHandle, request_id: String, decision: String) {
    recap::record_decision(&app, &request_id, &decision);
    pipe::answer(&app, &request_id, &decision);
}

/// An option picked on the island for a question Claude Code asked.
#[tauri::command]
fn approval_answer(
    app: AppHandle,
    request_id: String,
    answers: std::collections::HashMap<String, serde_json::Value>,
) {
    // An answered question is not an Allow / Deny: nothing for the recap.
    recap::forget_request(&app, &request_id);
    pipe::answer_question(&app, &request_id, &answers);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(app: AppHandle, request_id: String) {
    pipe::acknowledge(&app, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Claude Code falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(app: AppHandle, request_id: String) {
    recap::forget_request(&app, &request_id);
    pipe::decline(&app, &request_id);
}

// ── Chat and files ────────────────────────────────────────────────────────────

/// One chat turn with the local server picked in the chat view. File bytes
/// stay on the Rust side.
#[tauri::command]
async fn chat_send(
    app: AppHandle,
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let settings = shared.settings.lock().unwrap().clone();
    chat::send(&app, &chat, &settings, query, context).await
}

/// The models a local server offers, for the picker in the chat view. Only
/// asked once the user picked that server, and only with its address.
#[tauri::command]
async fn chat_models(shared: State<'_, Shared>, provider: String) -> Result<Vec<ModelInfo>, String> {
    let settings = shared.settings.lock().unwrap().clone();
    chat::models(&settings, &provider).await
}

/// Settings → Local models → Connect: does the server answer, and with which models?
#[tauri::command]
async fn local_connect(provider: String, url: String) -> Result<local_chat::Connected, String> {
    local_chat::connect(&provider, &url).await
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Writing checker (1b): the island's clicks ─────────────────────────────────
// Ids only: which list, which row, which replacement. The words to type or
// copy are the checker's own, kept in the capture thread (writing::Session),
// so no webview message can make Glim type text of its own choosing.

/// A fix button: types that replacement into the field (docs/phase1-plan.md §7.3).
#[tauri::command]
fn writing_fix(check_id: u64, index: usize, choice: usize) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::Fix { check_id, index, choice });
    #[cfg(not(windows))]
    let _ = (check_id, index, choice);
}

/// Apply all: every suggestion's first replacement, last to first, each
/// re-verified; stops at the first that doesn't apply.
#[tauri::command]
fn writing_fix_all(check_id: u64) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::FixAll { check_id });
    #[cfg(not(windows))]
    let _ = check_id;
}

/// A Copy button (fields Glim can't type in, §7.7): only on the user's click.
#[tauri::command]
fn writing_copy(check_id: u64, index: usize, choice: usize) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::Copy { check_id, index, choice });
    #[cfg(not(windows))]
    let _ = (check_id, index, choice);
}

// ── Settings → AI (1c) ────────────────────────────────────────────────────────
// The key comes in once, on Save, and goes straight to Credential Manager;
// Settings only ever gets back {set, hint}. Nothing here logs it.

// ── Phone notifications (docs/phone-plan.md) ─────────────────────────────────

/// The island's event: a kind, the pill and its project name. phone.rs
/// decides the text and whether it goes at all.
#[tauri::command]
async fn phone_notify(shared: State<'_, Shared>, kind: phone::Kind, pill_id: String, project: String) -> Result<(), String> {
    if kind == phone::Kind::Test {
        return Err("use phone_test".into());
    }
    let settings = shared.settings.lock().unwrap().clone();
    phone::notify(&settings, kind, &pill_id, &project).await
}

/// Settings → Phone → Send test notification.
#[tauri::command]
async fn phone_test(shared: State<'_, Shared>) -> Result<(), String> {
    let settings = shared.settings.lock().unwrap().clone();
    phone::notify(&settings, phone::Kind::Test, "", "").await
}

/// Settings → Phone → Only when I'm away: ms since the last keyboard or
/// mouse input on this PC, or null.
#[tauri::command]
fn phone_idle_ms() -> Option<u64> {
    phone::idle_ms()
}

#[tauri::command]
fn phone_keys_status() -> Result<phone::KeysStatus, String> {
    phone::status(&phone::vault())
}

#[tauri::command]
fn phone_keys_save(user: String, token: String) -> Result<phone::KeysStatus, String> {
    let saved = phone::save(&phone::vault(), &user, &token);
    log::line(if saved.is_ok() { "phone: keys saved" } else { "phone: keys not saved" });
    saved
}

#[tauri::command]
fn phone_keys_remove() -> Result<phone::KeysStatus, String> {
    let removed = phone::remove(&phone::vault());
    if removed.is_ok() {
        log::line("phone: keys removed");
    }
    removed
}

#[tauri::command]
fn ai_key_status() -> Result<ai::key::KeyStatus, String> {
    ai::key::status(&ai::key::vault())
}

#[tauri::command]
fn ai_key_save(key: String) -> Result<ai::key::KeyStatus, String> {
    let saved = ai::key::save(&ai::key::vault(), &key);
    log::line(if saved.is_ok() { "ai: API key saved" } else { "ai: API key not saved" });
    saved
}

#[tauri::command]
fn ai_key_remove() -> Result<ai::key::KeyStatus, String> {
    let removed = ai::key::remove(&ai::key::vault());
    if removed.is_ok() {
        log::line("ai: API key removed");
    }
    removed
}

/// This month's spend estimate, and the cap, for Settings.
#[tauri::command]
fn ai_spend() -> ai::spend::SpendView {
    let spent = ai::spend::current(&ai::spend::path(), &ai::spend::month_now());
    ai::spend::view(&spent, ai::rewrite::config().cap_usd)
}

/// The models Settings offers, with their prices ($ per million tokens).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AiModel {
    id: &'static str,
    name: &'static str,
    input_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
    prices_checked: &'static str,
}

/// Changes the settings in Rust's hands, saves them, puts the writing
/// settings in force and tells both windows.
fn update_settings(app: &AppHandle, shared: &Shared, change: impl FnOnce(&mut Settings)) {
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        change(&mut current);
        if let Err(err) = settings::save(&current) {
            log::line(format!("could not save settings: {err}"));
        }
        current.clone()
    };
    writing::prefs::set(writing::prefs::Prefs::from_settings(&updated));
    phone::apply_settings(&updated);
    let _ = app.emit("settings-changed", updated);
}

/// The island showed (or queued) "Check writing in [app]?" (1d, §14.2):
/// today's date is kept for that app, so it is asked at most once a day.
#[tauri::command]
fn writing_ask_shown(app: AppHandle, shared: State<Shared>, exe: String) {
    if !writing::ask::askable(&exe, &writing::prefs::get(), &writing::ask::today()) {
        return;
    }
    update_settings(&app, &shared, |s| {
        s.writing_apps_asked.retain(|k, _| !k.eq_ignore_ascii_case(&exe));
        s.writing_apps_asked.insert(exe, writing::ask::today());
    });
}

/// The answer to "Check writing in [app]?": "yes" turns the checker on
/// there, "never" keeps it off for good (both listed in Settings → Writing →
/// Apps), "later" asks again the next day at the earliest.
#[tauri::command]
fn writing_ask_answer(app: AppHandle, shared: State<Shared>, exe: String, answer: String) {
    if !writing::ask::valid_exe(&exe) || writing::apps::is_known(&exe) {
        return;
    }
    let today = writing::ask::today();
    log::line(format!("writing: asked about {exe}: {}", if matches!(answer.as_str(), "yes" | "never") { answer.as_str() } else { "later" }));
    update_settings(&app, &shared, |s| {
        s.writing_apps_asked.retain(|k, _| !k.eq_ignore_ascii_case(&exe));
        let on = match answer.as_str() {
            "yes" => "on",
            "never" => "off",
            _ => {
                s.writing_apps_asked.insert(exe, today);
                return;
            }
        };
        s.writing_apps.retain(|k, _| !k.eq_ignore_ascii_case(&exe));
        s.writing_apps.insert(exe, on.to_string());
    });
}

/// "Got it" on the Google Docs tip (writing::docs_tip): it never shows again.
#[tauri::command]
fn writing_docs_tip_dismiss(app: AppHandle, shared: State<Shared>) {
    log::line("writing: Google Docs tip dismissed");
    update_settings(&app, &shared, |s| s.writing_docs_tip_dismissed = true);
}

/// Push-to-talk's card (voice 1.5b): "Copy" on what was heard or not typed.
/// The text came from Rust in the first place; it is only put on the
/// clipboard, never logged. False: another app held the clipboard.
#[tauri::command]
fn voice_copy(text: String) -> bool {
    #[cfg(windows)]
    let copied = capture::fix::copy_to_clipboard(&text);
    #[cfg(not(windows))]
    let copied = { let _ = text; false };
    log::line(format!("voice: copy {}", if copied { "ok" } else { "clipboard busy" }));
    copied
}

/// An answer typed into Glim's question card (Decision B, 2026-10-16). False:
/// no question waits any more (closed or timed out), or nothing was typed.
/// The text goes to the clean-up like a spoken answer; it is never logged.
#[tauri::command]
fn voice_answer(app: AppHandle, text: String) -> bool {
    #[cfg(windows)]
    let ok = voice::run::answer_typed(&app, text);
    #[cfg(not(windows))]
    let ok = { let _ = (app, text); false };
    ok
}

/// Cancel or Close on the question card (or its timeout): the voice keys go
/// back to dictating and asking.
#[tauri::command]
fn voice_cancel() {
    #[cfg(windows)]
    voice::run::cancel_question();
}

// ── Glim learns (docs/glim-learns-plan.md §1, §2) ─────────────────────────────

/// Try again on a learned shortcut's card: it is deleted and the ask goes to
/// Claude.
#[tauri::command]
fn voice_try_again(app: AppHandle) -> bool {
    #[cfg(windows)]
    let ok = voice::run::try_again(&app);
    #[cfg(not(windows))]
    let ok = { let _ = app; false };
    ok
}

/// Save on a routine's card.
#[tauri::command]
fn voice_routine_save(app: AppHandle) -> bool {
    #[cfg(windows)]
    let ok = voice::run::save_proposed(&app);
    #[cfg(not(windows))]
    let ok = { let _ = app; false };
    ok
}

/// Settings → Ask Glim: the learned shortcuts and the routines.
#[tauri::command]
fn learn_list() -> learn::LearnView {
    learn::with(|l| learn::view(l, learn::now()))
}

#[tauri::command]
fn learn_forget_shortcut(key: String, slot: bool) -> bool {
    let gone = learn::with(|l| {
        let gone = l.shortcuts.remove(&key, slot);
        if gone {
            l.save_shortcuts();
        }
        gone
    });
    if gone {
        log::line("learn: a shortcut forgotten (Settings)");
    }
    gone
}

#[tauri::command]
fn learn_forget_shortcuts() {
    let n = learn::with(|l| {
        let n = l.shortcuts.items.len();
        l.shortcuts.items.clear();
        l.save_shortcuts();
        n
    });
    log::line(format!("learn: {n} shortcuts forgotten (Settings)"));
}

/// Rename a routine: None when done, else why not (a code Settings words).
#[tauri::command]
fn learn_routine_rename(shared: State<Shared>, name: String, to: String) -> Option<String> {
    let own = voice::own_apps::names(&shared.settings.lock().unwrap().voice_apps);
    let done = learn::with(|l| {
        let r = l.routines.rename(&name, &to, &own);
        if r.is_ok() {
            l.save_routines();
        }
        r
    });
    match done {
        Ok(()) => {
            log::line("learn: a routine renamed (Settings)");
            None
        }
        Err(code) => Some(code.to_string()),
    }
}

#[tauri::command]
fn learn_routine_delete(name: String) -> bool {
    let gone = learn::with(|l| {
        let gone = l.routines.remove(&name);
        if gone {
            l.save_routines();
        }
        gone
    });
    if gone {
        log::line("learn: a routine deleted (Settings)");
    }
    gone
}

// ── Remembered rules and facts (docs/glim-learns-plan.md §3) ──────────────────

/// Settings → Memory: what is remembered (shown on this PC only).
#[tauri::command]
fn memory_list() -> memory::Memory {
    memory::with(|m| (m.clone(), false))
}

#[tauri::command]
fn memory_delete(kind: memory::Kind, text: String) -> bool {
    let gone = memory::with(|m| {
        let gone = m.remove(kind, &text);
        (gone, gone)
    });
    if gone {
        log::line(format!("memory: a {} deleted (Settings)", kind.as_str()));
    }
    gone
}

#[tauri::command]
fn memory_forget_all() {
    memory::with(|m| {
        *m = memory::Memory::default();
        ((), true)
    });
    log::line("memory: everything forgotten (Settings)");
}

/// Push-to-talk's card when Windows blocks the microphone (voice plan §6):
/// Windows' own microphone privacy page. A fixed settings link, nothing
/// from the island; no network.
#[tauri::command]
fn open_mic_settings() {
    log::line("voice: opened the microphone privacy settings");
    #[cfg(windows)]
    platform::open_url(MIC_SETTINGS);
}

/// Windows Settings → Privacy & security → Microphone.
const MIC_SETTINGS: &str = "ms-settings:privacy-microphone";

/// The mini lantern's popover opened or closed (1e, §7a.3): the window is
/// laid out again at once.
#[tauri::command]
fn bubble_popover(open: bool) {
    #[cfg(windows)]
    {
        bubble_window::set_open(open);
        capture::uia::request(capture::uia::Request::Wake);
    }
    #[cfg(not(windows))]
    let _ = open;
}

/// "Rewrite…" in the mini lantern's popover: the same as Ctrl+Alt+Shift+R
/// (1c's tones for this paragraph; nothing is sent until a tone is picked).
#[tauri::command]
fn bubble_rewrite(app: AppHandle) {
    shortcuts::dispatch(&app, "rewrite");
}

// ── The "Draft email" card (voice 1.5d) ───────────────────────────────────────
// Ids, indexes and a search's text only; voice::draft_worker owns the card and
// Outlook. Nothing here can send an email.

/// A chip: choice `choice` of recipient `index` on card `id`.
#[tauri::command]
fn draft_pick(app: AppHandle, id: u64, index: usize, choice: usize) {
    #[cfg(windows)]
    voice::draft_worker::pick(&app, id, index, choice);
    #[cfg(not(windows))]
    let _ = (app, id, index, choice);
}

/// "Change recipient" on recipient `index`: its address-book search opens.
#[tauri::command]
fn draft_change(app: AppHandle, id: u64, index: usize) {
    #[cfg(windows)]
    voice::draft_worker::change(&app, id, index);
    #[cfg(not(windows))]
    let _ = (app, id, index);
}

/// The search's text, as typed (Outlook's address book, on this PC).
#[tauri::command]
fn draft_search(app: AppHandle, id: u64, query: String) {
    #[cfg(windows)]
    voice::draft_worker::search(&app, id, query);
    #[cfg(not(windows))]
    let _ = (app, id, query);
}

/// "Create draft": saved in Outlook's Drafts and opened there. Never sent.
#[tauri::command]
fn draft_create(app: AppHandle, id: u64) {
    #[cfg(windows)]
    voice::draft_worker::create(&app, id);
    #[cfg(not(windows))]
    let _ = (app, id);
}

#[tauri::command]
fn draft_cancel(app: AppHandle, id: u64) {
    #[cfg(windows)]
    voice::draft_worker::cancel(&app, id);
    #[cfg(not(windows))]
    let _ = (app, id);
}

/// Settings → Email drafts → Forget: a remembered pick goes.
#[tauri::command]
fn draft_forget(app: AppHandle, shared: State<Shared>, name: String) {
    log::line("draft: a remembered name forgotten");
    update_settings(&app, &shared, |s| {
        s.draft_recipients.remove(&name);
    });
}

/// The card's picks, remembered once its draft is created (the spoken name,
/// lowercased → the address). Never logged.
/// The calendar glance's morning summary was shown today (local date).
pub(crate) fn calendar_summary_shown(app: &AppHandle, date: &str) {
    let Some(shared) = app.try_state::<Shared>() else { return };
    update_settings(app, &shared, |s| s.calendar_summary_shown = date.to_string());
}

/// Settings → Calendar: the calendars classic Outlook has open (empty when
/// it isn't running; it is never started for this).
#[tauri::command]
async fn calendar_calendars(app: AppHandle) -> Vec<calendar::worker::CalendarFolder> {
    calendar::worker::list(&app).await
}

/// The island's Join button: event `index` of read `generation`. Rust holds
/// the link and opens it only if it is a checked Teams/Zoom meeting link.
#[tauri::command]
fn calendar_join(app: AppHandle, caller: tauri::WebviewWindow, generation: u64, index: usize) {
    if caller.label() != island::WINDOW_LABEL {
        log::line(format!("calendar_join refused from window {:?}", caller.label()));
        return;
    }
    calendar::worker::join(&app, generation, index);
}

pub(crate) fn remember_draft_picks(app: &AppHandle, picks: &[(String, String)]) {
    if picks.is_empty() {
        return;
    }
    let Some(shared) = app.try_state::<Shared>() else { return };
    update_settings(app, &shared, |s| {
        for (name, address) in picks {
            s.draft_recipients.insert(name.clone(), address.clone());
        }
    });
}

/// The Rewrite card's default tone for the field being written in (1d, §14.3).
#[tauri::command]
fn writing_rewrite_default() -> &'static str {
    writing::tones::current_default().id()
}

/// Each category's built-in tone, for Settings → Writing → Rewrite tones.
#[tauri::command]
fn writing_tone_defaults() -> Vec<(&'static str, &'static str)> {
    writing::tones::Category::ALL.iter().map(|c| (c.id(), c.builtin().id())).collect()
}

/// The built-in pause entries (locked), for Settings → Privacy (1d, §14.5).
#[derive(serde::Serialize)]
struct PauseBuiltins {
    apps: Vec<&'static str>,
    sites: Vec<&'static str>,
}

#[tauri::command]
fn writing_pause_builtins() -> PauseBuiltins {
    PauseBuiltins { apps: capture::sensitive::PAUSED_APPS.to_vec(), sites: capture::sensitive::PAUSED_SITES.to_vec() }
}

/// The apps Settings → Writing → Apps lists (1d): the built-in ones.
#[tauri::command]
fn writing_known_apps() -> Vec<writing::apps::KnownApp> {
    writing::apps::known()
}

#[tauri::command]
fn ai_models() -> Vec<AiModel> {
    ai::claude::MODELS
        .iter()
        .map(|m| AiModel {
            id: m.id,
            name: m.name,
            input_usd_per_mtok: m.input_nanos as f64 / 1000.0,
            output_usd_per_mtok: m.output_nanos as f64 / 1000.0,
            prices_checked: ai::claude::PRICES_CHECKED,
        })
        .collect()
}

// ── The rewrite (1c): the island's clicks ─────────────────────────────────────
// Ids only, as for fixes: a tone id, or which rewrite. The paragraph is the
// capture thread's own, and so is the rewrite's text.

/// A tone on the Rewrite card: the paragraph the caret is in goes to the
/// Claude API (docs/phase1-plan.md §13). The click is the user's request.
#[tauri::command]
fn writing_rewrite(tone: String) {
    let Some(tone) = ai::claude::Tone::from_id(&tone) else { return };
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::Rewrite { tone });
    #[cfg(not(windows))]
    let _ = tone;
}

/// "Check with AI" (docs/phase1-plan.md §15.1): the paragraph the caret is
/// in goes to the Claude API (Haiku). The click is the user's request.
#[tauri::command]
fn writing_ai_check() {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::AiCheck { auto: false });
}

/// "Use this": types rewrite `id` in through 1b's apply path.
#[tauri::command]
fn writing_rewrite_apply(id: u64) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::RewriteApply { id });
    #[cfg(not(windows))]
    let _ = id;
}

/// Copy for rewrite `id`.
#[tauri::command]
fn writing_rewrite_copy(id: u64) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::RewriteCopy { id });
    #[cfg(not(windows))]
    let _ = id;
}

/// The Rewrite card closed: its rewrite and paragraph go.
#[tauri::command]
fn writing_rewrite_dismiss() {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::RewriteDismiss);
}

/// Ignore on a row (§7a.3): that word isn't flagged again this run. Ids only.
#[tauri::command]
fn writing_ignore(check_id: u64, index: usize) {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::Ignore { check_id, index });
    #[cfg(not(windows))]
    let _ = (check_id, index);
}

/// Dismiss: the list and its kept paragraph go until the text changes.
#[tauri::command]
fn writing_dismiss() {
    #[cfg(windows)]
    capture::uia::request(capture::uia::Request::Dismiss);
}

// ── Global shortcuts ──────────────────────────────────────────────────────────

/// How each global shortcut went: registered, taken by another app, and so on.
#[tauri::command]
fn shortcuts_status(app: AppHandle) -> shortcuts::Report {
    shortcuts::status(&app)
}

/// Settings is recording a new combination: let go of ours meanwhile, so the
/// keys reach the recorder instead of running an action. `false` takes them back.
#[tauri::command]
fn shortcuts_suspend(app: AppHandle, shared: State<Shared>, suspended: bool) {
    if suspended {
        shortcuts::suspend(&app);
    } else {
        let stored = shared.settings.lock().unwrap().shortcuts.clone();
        shortcuts::apply(&app, &stored);
    }
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere. A test below holds the two together.
///
/// Local-only (PRIVACY.md): left alone, the WebView2 runtime calls Microsoft on
/// its own — Edge's experiment config (config.edge.skype.com), the component
/// updater (edge.microsoft.com), network-error reports (ecs.nel.measure.office.net)
/// and a WPAD proxy lookup. Observed in a net-log of this app on 2026-10-09.
/// The switches after the first two turn those off, and the resolver rule makes
/// every host name but `localhost` fail to resolve inside the webview, so
/// anything not listed here cannot reach the network either.
pub(crate) const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-domain-reliability --no-pings --no-proxy-server \"--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE *.localhost\"";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
pub(crate) fn create_settings_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title(i18n::t("Settings — Glim"))
        .inner_size(780.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .visible(false)
        // Created hidden at launch, it still took the foreground without
        // this (seen 2026-10-09: the app being typed in lost focus to an
        // invisible window). It takes focus only when it is shown.
        .focused(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
            Some(win)
        }
        Err(err) => {
            log::line(format!("settings window failed: {err}"));
            None
        }
    }
}

/// Dev only (GLIM_DEV=1): the text-capture spike's debug panel. Created before
/// the island like the settings window (see create_settings_window), never
/// focused and non-activating, so it can't take focus from the field being
/// read. It shows captured text live and keeps none of it.
#[cfg(windows)]
fn create_capture_panel(app: &AppHandle) {
    #[cfg(dev)]
    let url = match app.config().build.dev_url.clone() {
        Some(mut base) => {
            base.set_path("/capture.html");
            WebviewUrl::External(base)
        }
        None => WebviewUrl::App("capture.html".into()),
    };
    #[cfg(not(dev))]
    let url = WebviewUrl::App("capture.html".into());
    match WebviewWindowBuilder::new(app, capture::uia::PANEL, url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Capture debug — Glim (dev)")
        .inner_size(460.0, 560.0)
        .position(40.0, 120.0)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        // tao keeps WS_EX_NOACTIVATE only through its own FOCUSABLE flag; set
        // behind its back it is wiped on the next restyle (as the island's was).
        .focusable(false)
        .visible(true)
        .build()
    {
        Ok(win) => platform::make_non_activating(&win),
        Err(err) => log::line(format!("capture panel failed: {err}")),
    }
}

/// `from` says who opened it, in glim.log (tray, island, command).
pub fn show_settings_window(app: &AppHandle, from: &str) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    webview_health::before_settings_shown(&win);
    let _ = win.unminimize();
    let _ = win.show();
    // Only now: never on the hidden window (see screen_capture::after_show).
    screen_capture::after_show(app, &win);
    let _ = win.set_focus();
    // Settings → Calendar asks Outlook for its calendars again (its list
    // may have been empty when the page was built at launch).
    let _ = app.emit_to("settings", "settings-opened", ());
    // The page must answer; a white one is reloaded, then built again.
    webview_health::settings_shown(app, from);
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app, "island");
}

/// The Settings page heard `settings-opened` or `settings-check`: it is alive
/// (webview_health.rs).
#[tauri::command]
fn settings_page_alive() {
    webview_health::settings_answered();
}

/// The Settings page has drawn its sections (`ok`) or failed to: one line in
/// glim.log either way (webview_health.rs).
#[tauri::command]
fn settings_page(ok: bool) {
    webview_health::settings_page(ok);
}

/// `glim.exe --mascot-state <idle|listen|think|suggest|record|paused|delegate|done|error|auto>`,
/// forwarded to the running Glim by the single-instance plugin: shows the
/// lantern in that state (`auto` follows the app again). A dev tool for
/// checking each state without touching the mouse or keyboard, so it only works
/// when the running Glim was started with `GLIM_DEV=1` in its environment.
fn dev_mascot_state(argv: &[String]) -> Option<String> {
    dev_arg(argv, "--mascot-state")
}

/// `glim.exe --dock <top-center|top-left|top-right|left-vertical|right-vertical>`:
/// docks the island on its display, as dragging the pill there would. Dev only.
fn dev_dock(argv: &[String]) -> Option<placement::Dock> {
    dev_arg(argv, "--dock").and_then(|d| placement::Dock::parse(&d))
}

/// `glim.exe --visibility <normal|ember|hidden>`: as Settings or the hotkey
/// would set it. `glim.exe --fullscreen <on|off|auto>`: pretends a fullscreen
/// app is (or isn't) in front, to check the auto-hide. Dev only.
fn dev_visibility(argv: &[String]) -> Option<presence::Visibility> {
    dev_arg(argv, "--visibility").and_then(|v| presence::Visibility::parse(&v))
}

fn dev_fullscreen(argv: &[String]) -> Option<Option<bool>> {
    match dev_arg(argv, "--fullscreen")?.as_str() {
        "on" => Some(Some(true)),
        "off" => Some(Some(false)),
        "auto" => Some(None),
        _ => None,
    }
}

/// `glim.exe --dev-chat "<prompt>"`: sends the prompt through the island's own
/// chat view (connecting Ollama first, as Settings → Connect does, if it isn't
/// yet), so the real chat path — chat view → `chat_send` → `local_chat` →
/// `net::request` — can be exercised without typing. Dev only, like
/// `--mascot-state`.
fn dev_chat_prompt(argv: &[String]) -> Option<String> {
    dev_arg(argv, "--dev-chat")
}

/// `glim.exe --dev-fix "<replacement>"`: opens the suggestion list and clicks
/// that fix button, inside the island's page, as a mouse click would
/// (scripts/verify-fix-notepad.ps1). A UI Automation Invoke can't stand in
/// for the click: Chromium activates the island window for it, and the fix
/// is then rightly refused (the field's app is no longer in front). Dev only.
fn dev_fix(argv: &[String]) -> Option<String> {
    dev_arg(argv, "--dev-fix")
}

/// The value after `flag`, only when the running Glim was started with
/// `GLIM_DEV=1`.
fn dev_arg(argv: &[String], flag: &str) -> Option<String> {
    if !dev_session() {
        return None;
    }
    let at = argv.iter().position(|a| a == flag)?;
    argv.get(at + 1).cloned()
}

/// `glim.exe --open-settings`: opens the Settings window directly, as the tray
/// menu's "Settings…" does, for when the island can't be reached (it can end
/// up on another display; see PROJECT_STATUS.md, open issues). Dev only.
fn dev_open_settings(argv: &[String]) -> bool {
    dev_session() && argv.iter().any(|a| a == "--open-settings")
}

/// The running Glim was started with `GLIM_DEV=1`.
fn dev_session() -> bool {
    std::env::var_os("GLIM_DEV").is_some_and(|v| v == "1")
}

pub fn run() {
    // A Glim started by relaunch.rs waits for the old one to exit, or the
    // single-instance plugin below would hand it straight back to it.
    let relaunched = relaunch::wait_for_previous(&std::env::args().collect::<Vec<_>>());
    // Before anything reads or creates the data folders (settings, log, relay).
    let migrated = platform::migrate_data_dirs();
    for note in &migrated {
        log::line(format!("data folder migration: {note}"));
    }
    if relaunched {
        log::line("relaunch: started after the window engine stopped");
    }
    platform::prepare_environment();
    let loaded = settings::load();
    i18n::set_picked(&loaded.language);
    ai::rewrite::set_config(ai::rewrite::Config::from_settings(&loaded));
    writing::prefs::set(writing::prefs::Prefs::from_settings(&loaded));
    phone::apply_settings(&loaded);
    let gate = Arc::new(PollGate::new());
    gate.drop_zone.store(loaded.file_drop_zone, std::sync::atomic::Ordering::Relaxed);

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // `glim --shortcut <action>`: what a desktop's own keyboard
            // settings run where we can't listen for keys ourselves (Wayland).
            // `glim.exe --resume-card` (GLIM_DEV=1): shows the resume card, as the
            // tray item does (decided 2026-10-15).
            if dev_session() && argv.iter().any(|a| a == "--resume-card") {
                let _ = app.emit_to(island::WINDOW_LABEL, "resume-show", ());
                return;
            }
            if let Some(state) = dev_mascot_state(&argv) {
                let _ = app.emit_to(island::WINDOW_LABEL, "mascot-force", state);
                return;
            }
            if let Some(v) = dev_visibility(&argv) {
                presence::set_visibility(app, v);
                return;
            }
            if let Some(f) = dev_fullscreen(&argv) {
                presence::set_dev_fullscreen(app, f);
                return;
            }
            if let Some(dock) = dev_dock(&argv) {
                island::set_dock(app, None, dock);
                return;
            }
            if dev_open_settings(&argv) {
                show_settings_window(app, "command");
                return;
            }
            if let Some(fix) = dev_fix(&argv) {
                let _ = app.emit_to(island::WINDOW_LABEL, "dev-fix", fix);
                return;
            }
            if let Some(prompt) = dev_chat_prompt(&argv) {
                let _ = app.emit_to(island::WINDOW_LABEL, "dev-chat", prompt);
                return;
            }
            match shortcuts::from_args(&argv) {
                Some(action) => shortcuts::dispatch(app, action),
                None => {
                    let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
                }
            }
        }));
    // Where no global shortcut can work, the plugin isn't even started.
    if platform::global_shortcuts_blocked().is_none() {
        builder = builder.plugin(shortcuts::plugin());
    }

    builder
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Pending::default())
        .manage(Chat::default())
        .manage(shortcuts::Registry::default())
        .manage(recap::load())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_system_languages,
            set_collapsed,
            set_island_rect,
            focus_window,
            lend_keyboard,
            calendar_calendars,
            calendar_join,
            reposition,
            dock_drag_start,
            set_visibility,
            set_peek,
            presence_info,
            set_dock,
            placement,
            list_monitors,
            open_url,
            open_in_vscode,
            open_session,
            open_claude_desktop,
            open_file_in_vscode,
            quit_app,
            hooks_status,
            agent_hooks_status,
            hooks_preview,
            hooks_apply,
            agent_hooks_list,
            agent_hooks_preview,
            agent_hooks_apply,
            status_line_preview,
            status_line_apply,
            approval_decision,
            approval_answer,
            approval_ack,
            approval_decline,
            log_line,
            writing_fix,
            writing_fix_all,
            writing_rewrite,
            ai_key_status,
            ai_key_save,
            ai_key_remove,
            phone_notify,
            phone_test,
            phone_idle_ms,
            phone_keys_status,
            phone_keys_save,
            phone_keys_remove,
            ai_spend,
            ai_models,
            writing_known_apps,
            writing_pause_builtins,
            writing_rewrite_default,
            writing_tone_defaults,
            writing_ask_shown,
            writing_ask_answer,
            writing_docs_tip_dismiss,
            voice_copy,
            voice_answer,
            voice_cancel,
            voice_try_again,
            voice_routine_save,
            learn_list,
            learn_forget_shortcut,
            learn_forget_shortcuts,
            learn_routine_rename,
            learn_routine_delete,
            memory_list,
            memory_delete,
            memory_forget_all,
            open_mic_settings,
            draft_pick,
            draft_change,
            draft_search,
            draft_create,
            draft_cancel,
            draft_forget,
            writing_ai_check,
            resume_cards,
            resume_open,
            bubble_popover,
            bubble_rewrite,
            writing_rewrite_apply,
            writing_rewrite_copy,
            writing_rewrite_dismiss,
            writing_copy,
            writing_dismiss,
            writing_ignore,
            chat_send,
            chat_models,
            local_connect,
            chat_reset,
            ingest_file,
            open_settings_window,
            settings_page,
            settings_page_alive,
            shortcuts_status,
            shortcuts_suspend,
            recap::recap_history,
            recap::recap_prefs,
            recap::recap_set_enabled,
            recap::recap_set_hide_projects,
            recap::recap_mark_shown,
            recap::recap_clear,
            recap::recap_save_png,
            recap::recap_reveal_saved,
            desktop::desktop_mochi_info,
            desktop::desktop_mochi_pick_up,
            desktop::desktop_mochi_carry,
            desktop::desktop_mochi_carry_end,
            desktop::desktop_mochi_drag_begin,
            desktop::desktop_mochi_drag_move,
            desktop::desktop_mochi_drag_end,
            desktop::desktop_mochi_fly_out,
            desktop::desktop_mochi_fly_home,
            desktop::desktop_mochi_set_asleep,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // What Glim learned (shortcuts, routines): loaded once, counts logged.
            learn::with(|_| ());
            // What is remembered (Settings → Memory): loaded once, counts logged.
            memory::with(|_| ((), false));
            // Launch at startup follows the setting at every launch, not only
            // when it changes (the entry may be missing, or point at an old copy).
            autostart::sync(loaded.autostart, false);
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);
            // The capture thread (src/capture/): the writing checker in every
            // session, for the apps it is on in (src/writing/apps.rs); in dev
            // sessions also the live debug panel, for every app.
            #[cfg(windows)]
            {
                if dev_session() {
                    create_capture_panel(&handle);
                }
                capture::uia::start(handle.clone(), dev_session());
            }
            // Same rule for the character's desktop window.
            desktop::setup(&handle);
            // Every window built so far, before the island is first shown;
            // the hidden ones (Settings) get it when shown.
            screen_capture::apply(&handle);
            // Every window built so far keeps its page through a WebView2
            // process failure; Settings is reloaded after a wake.
            webview_health::watch_all(&handle);
            webview_health::start_wake_watch(&handle);
            webview_health::start_ui_watch(&handle);

            if let Some(win) = island::window(&handle) {
                // Where Tauri takes the drop itself (Linux), its paths are the
                // ones ingest_file may copy (files.rs).
                win.on_window_event(|event| {
                    if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                        files::allow_dropped(paths.iter().map(|p| p.to_string_lossy().to_string()));
                    }
                });
                platform::make_non_activating(&win);
                #[cfg(windows)]
                webview_drop::install(&handle);
                island::apply_geometry(&handle, &loaded.screen, false);
                let _ = win.show();
                screen_capture::after_show(&handle, &win);
            }
            gate.collapsed.store(false, Ordering::Relaxed);
            // Nothing drawn yet, so nothing takes the mouse until the page
            // reports the island's shape.
            if !platform::CURSOR_POLL {
                island::refresh_click_through(&handle, &gate);
            }
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());
            island::spawn_display_watch(handle.clone());
            // The visibility hotkey works in Rust: it has to reach a hidden window.
            let cycle_handle = handle.clone();
            shortcuts::on_cycle_visibility(move || presence::cycle(&cycle_handle));
            // So does the pause key (1d): it works with the island hidden.
            let pause_handle = handle.clone();
            shortcuts::on_pause_toggle(move || pause::toggle(&pause_handle));
            // And push-to-talk (voice 1.5b): Rust polls the key for its release.
            #[cfg(windows)]
            {
                let voice_handle = handle.clone();
                shortcuts::on_push_to_talk(move |ask| voice::run::on_key_down(&voice_handle, if ask { voice::Mode::Ask } else { voice::Mode::Dictate }));
            }

            log::line(format!("--- Glim {} started ---", env!("CARGO_PKG_VERSION")));
            hooks::ensure_hook_exe(&handle);
            // Every Claude config folder in sync? Logged only; Settings fixes it.
            hooks::log_sync_state();
            pipe::start(handle.clone());
            shortcuts::apply(&handle, &loaded.shortcuts);
            // The calendar glance: reads only while its setting is on, and
            // only an Outlook that is already running.
            calendar::worker::start(&handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Glim");
}

#[cfg(test)]
mod tests {
    use super::{diff_file, BROWSER_ARGS};

    /// The island never takes focus by being shown: it is declared unfocused,
    /// made WS_EX_NOACTIVATE before it is first shown, and only the chat view
    /// (or the island's own shortcuts) asks for focus, through focus_window.
    /// The capture debug panel is built unfocused and non-activating too.
    #[test]
    fn showing_the_island_or_the_capture_panel_never_takes_focus() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let island = conf["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["label"] == "island")
            .expect("the island window");
        assert_eq!(island["focus"], false);
        // tao owns WS_EX_NOACTIVATE only through its FOCUSABLE flag: anything
        // set behind its back is wiped on its next restyle (seen 2026-10-10:
        // Hidden → Normal took the foreground).
        assert_eq!(island["focusable"], false);
        let src_lf = include_str!("lib.rs").replace("\r\n", "\n");
        let focus = &src_lf[src_lf.find("fn focus_window").unwrap()..];
        assert!(focus[..focus.find("\n}\n").unwrap()].contains("let _ = win.set_focusable(focused);"));

        // CI checks the sources out with CRLF (core.autocrlf): compare as LF.
        let src = include_str!("lib.rs").replace("\r\n", "\n");
        let desktop = include_str!("desktop.rs").replace("\r\n", "\n");
        let setup = &src[src.find(".setup(move |app|").unwrap()..];
        let non_activating = setup.find("platform::make_non_activating(&win);").expect("island made non-activating");
        let shown = setup.find("let _ = win.show();").expect("island shown");
        assert!(non_activating < shown, "WS_EX_NOACTIVATE must be set before the island is first shown");

        let panel = &src[src.find("fn create_capture_panel").unwrap()..];
        let panel = &panel[..panel.find("\n}\n").unwrap()];
        assert!(panel.contains(".focused(false)") && panel.contains("platform::make_non_activating(&win)"));

        // Every window Glim builds in code is built unfocused: a hidden one
        // created at launch (Settings) used to take the foreground anyway.
        for (file, code) in [("lib.rs", src.as_str()), ("desktop.rs", desktop.as_str())] {
            let code = code.split("#[cfg(test)]").next().unwrap();
            for (at, _) in code.match_indices("WebviewWindowBuilder::new(") {
                let chain = &code[at..at + code[at..].find(".build()").expect("a builder chain")];
                assert!(chain.contains(".focused(false)"), "{file}: a window is built without .focused(false)");
            }
        }
    }

    #[test]
    fn every_window_asks_webview2_for_the_same_locked_down_arguments() {
        for conf in ["tauri.conf.json", "tauri.linux.conf.json"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(conf);
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let island = json.pointer("/app/windows/0/additionalBrowserArgs").and_then(|v| v.as_str());
            assert_eq!(island, Some(BROWSER_ARGS), "{conf}");
        }
        for switch in [
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-domain-reliability",
            "--no-pings",
            "--no-proxy-server",
            "\"--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE *.localhost\"",
        ] {
            assert!(BROWSER_ARGS.contains(switch), "{switch}");
        }
    }

    #[test]
    fn only_an_existing_file_by_its_full_path_reaches_the_editor() {
        let dir = std::env::temp_dir().join(format!("glim-diff-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edited.ts");
        std::fs::write(&file, "x").unwrap();

        assert!(diff_file(&file.to_string_lossy()).is_some());
        // A folder, a missing file, a relative path or an option never pass.
        assert!(diff_file(&dir.to_string_lossy()).is_none());
        assert!(diff_file(&dir.join("missing.ts").to_string_lossy()).is_none());
        assert!(diff_file("edited.ts").is_none());
        assert!(diff_file("--help").is_none());
        assert!(diff_file("").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
