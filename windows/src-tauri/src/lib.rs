// Glim for Windows — app wiring and the commands the island calls.

mod agent_hooks;
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
mod session_window;
mod settings;
mod shortcuts;
mod placement;
mod tray;
#[cfg(windows)]
mod webview_drop;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

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
        *current = settings;
        (screen_changed, autostart_changed, shortcuts_changed)
    };
    let settings = shared.settings.lock().unwrap().clone();
    if let Err(err) = settings::save(&settings) {
        log::line(format!("could not save settings: {err}"));
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[glim] autostart: {err}");
        }
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
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else { return };
    platform::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
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
    match net::check_str(&url) {
        Ok(url) => platform::open_url(url.as_str()),
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
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title(i18n::t("Settings — Glim"))
        .inner_size(560.0, 680.0)
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
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
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
        .visible(true)
        .build()
    {
        Ok(win) => platform::make_non_activating(&win),
        Err(err) => log::line(format!("capture panel failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
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

/// `glim.exe --dev-chat "<prompt>"`: sends the prompt through the island's own
/// chat view (connecting Ollama first, as Settings → Connect does, if it isn't
/// yet), so the real chat path — chat view → `chat_send` → `local_chat` →
/// `net::request` — can be exercised without typing. Dev only, like
/// `--mascot-state`.
fn dev_chat_prompt(argv: &[String]) -> Option<String> {
    dev_arg(argv, "--dev-chat")
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
    // Before anything reads or creates the data folders (settings, log, relay).
    let migrated = platform::migrate_data_dirs();
    for note in &migrated {
        log::line(format!("data folder migration: {note}"));
    }
    platform::prepare_environment();
    let loaded = settings::load();
    i18n::set_picked(&loaded.language);
    let gate = Arc::new(PollGate::new());

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // `glim --shortcut <action>`: what a desktop's own keyboard
            // settings run where we can't listen for keys ourselves (Wayland).
            if let Some(state) = dev_mascot_state(&argv) {
                let _ = app.emit_to(island::WINDOW_LABEL, "mascot-force", state);
                return;
            }
            if let Some(dock) = dev_dock(&argv) {
                island::set_dock(app, None, dock);
                return;
            }
            if dev_open_settings(&argv) {
                show_settings_window(app);
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
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None));
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
            reposition,
            dock_drag_start,
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
            chat_send,
            chat_models,
            local_connect,
            chat_reset,
            ingest_file,
            open_settings_window,
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
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);
            // Dev only: the text-capture spike's live debug panel, and the
            // capture thread that feeds it (src/capture/).
            #[cfg(windows)]
            if dev_session() {
                create_capture_panel(&handle);
                capture::uia::start(handle.clone());
            }
            // Same rule for the character's desktop window.
            desktop::setup(&handle);

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

            log::line(format!("--- Glim {} started ---", env!("CARGO_PKG_VERSION")));
            hooks::ensure_hook_exe(&handle);
            pipe::start(handle.clone());
            shortcuts::apply(&handle, &loaded.shortcuts);
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
