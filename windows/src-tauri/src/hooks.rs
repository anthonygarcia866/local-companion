// Claude Code hook installation.
//
// The rule from CLAUDE.md is strict and is followed to the letter:
// read settings.json, take a dated backup, merge without touching anybody
// else's hooks, show the diff, and write only after an explicit click.
// Uninstall removes Glim's entries and nothing else.
//
// Every Claude config folder gets the hooks (`config_dirs`): ~/.claude, the
// folder CLAUDE_CONFIG_DIR names, and any ~/.claude<digits> folder (the owner
// runs ~/.claude and ~/.claude2), each only if Claude Code has really used it
// (`looks_like_claude_config`). Other tools' ~/.claude-mem or ~/.claude-flow
// hold a settings.json of their own and are never written. They are found
// afresh each time, so a folder that appears later is picked up by the next
// install or update, and one preview covers them all.
//
// The command is only the quoted exe path in forward slashes plus the event name:
// on Windows Claude Code runs hook commands through Git Bash, and anything with
// PowerShell or cmd in it breaks.
//
// Reading, the diff, the backup and the write itself live in config_file.rs,
// shared with every other agent's installer (agents.rs).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use crate::agents::{self, Shell};
use crate::config_file::{self, FileEdit};
use crate::{platform, settings};

/// Every event the island reacts to, with the hook timeout written to settings.json.
/// PermissionRequest waits for a human, so it gets the decision timeout + 10 s.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

// Glim's entries in settings.json are told apart by the relay they run:
// `agents::MARKER` (glim-hook) now, `agents::LEGACY_MARKER` (coucou-hook)
// before the rename. Installing replaces both and removing removes both, but
// only the current name counts as installed: the old relay path is gone.
use crate::agents::{names_relay, MARKER};

/// One Claude config folder, as Settings shows it.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderStatus {
    /// The folder's settings.json.
    pub settings_path: String,
    /// Glim's hooks (the current relay) are in it.
    pub installed: bool,
    /// Installing would change nothing: every hook is there and current.
    pub up_to_date: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    /// Glim's hooks are in at least one folder.
    pub installed: bool,
    /// Every folder has them, current: nothing to update.
    pub all_up_to_date: bool,
    pub folders: Vec<FolderStatus>,
    /// Coucou's status line relay (plan usage) is the one in settings.json.
    pub plan_relay_installed: bool,
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to `write`
    /// so we only ever apply what the user actually looked at.
    pub fingerprint: String,
}

/// ~/.claude/settings.json: where the status line relay (plan usage) goes.
pub fn settings_path() -> PathBuf {
    platform::home_dir().join(".claude").join("settings.json")
}

/// Every Claude config folder, in a fixed order: ~/.claude, the folder
/// CLAUDE_CONFIG_DIR names, then the ~/.claude<digits> folders, by name; each
/// only if it looks like a Claude Code config. Each folder once, whatever its
/// spelling.
pub fn config_dirs() -> Vec<PathBuf> {
    config_dirs_in(&platform::home_dir(), std::env::var_os(CONFIG_DIR_VAR).map(PathBuf::from))
}

/// Claude Code's own variable. Tests read another one: the developer's real
/// CLAUDE_CONFIG_DIR (the owner's ~/.claude2) must never be written by a test.
#[cfg(not(test))]
const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";
#[cfg(test)]
const CONFIG_DIR_VAR: &str = "GLIM_TEST_CLAUDE_CONFIG_DIR";

fn config_dirs_in(home: &Path, env: Option<PathBuf>) -> Vec<PathBuf> {
    find_config_dirs(home, env).0
}

/// The config folders, and the names of the ~/.claude* (and CLAUDE_CONFIG_DIR)
/// folders left out, for the launch log.
fn find_config_dirs(home: &Path, env: Option<PathBuf>) -> (Vec<PathBuf>, Vec<String>) {
    let mut candidates = vec![home.join(".claude")];
    if let Some(dir) = env.filter(|d| !d.as_os_str().is_empty()) {
        let dir = if dir.is_absolute() { dir } else { home.join(dir) };
        // A folder that isn't there isn't created: Claude Code makes its own.
        if dir.is_dir() {
            candidates.push(dir);
        }
    }
    let mut skipped = Vec::new();
    let mut others = Vec::new();
    for e in std::fs::read_dir(home).into_iter().flatten().filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        // The entry's own type: a path to "~/.claude2 " would be read as
        // ~/.claude2 on Windows (trailing space dropped).
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir();
        if !name.to_ascii_lowercase().starts_with(".claude") || !is_dir || name.eq_ignore_ascii_case(".claude") {
            continue;
        }
        if numbered_claude_name(&name) {
            others.push(e.path());
        } else {
            skipped.push(name);
        }
    }
    others.sort();
    candidates.extend(others);
    let mut seen: Vec<String> = Vec::new();
    candidates.retain(|d| {
        let key = same_folder_key(d);
        let fresh = !seen.contains(&key);
        seen.push(key);
        fresh
    });
    let (dirs, unlike): (Vec<PathBuf>, Vec<PathBuf>) = candidates.into_iter().partition(|d| looks_like_claude_config(d));
    skipped.extend(unlike.iter().map(|d| d.file_name().map_or_else(|| d.to_string_lossy().to_string(), |n| n.to_string_lossy().to_string())));
    skipped.sort();
    skipped.dedup();
    (dirs, skipped)
}

/// `.claude` followed by one or more digits and nothing else (`.claude2`,
/// `.claude10`): the other folders the owner names by hand. `.claude-mem`,
/// `.claude-flow` (other tools' own folders) and `.claude2 ` don't count.
fn numbered_claude_name(name: &str) -> bool {
    match (name.get(..7), name.get(7..)) {
        (Some(head), Some(digits)) => {
            head.eq_ignore_ascii_case(".claude") && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

/// Claude Code has run with this folder as its config: it keeps `projects/`
/// there, and `history.jsonl` or `.claude.json` (both checked in the owner's
/// ~/.claude and ~/.claude2, 2026-10-13). Existence only: nothing is read.
fn looks_like_claude_config(dir: &Path) -> bool {
    dir.join("projects").is_dir() && (dir.join("history.jsonl").is_file() || dir.join(".claude.json").is_file())
}

/// Two spellings of one folder compare equal (case on Windows, a trailing
/// separator, `.` parts, links where the folder exists).
fn same_folder_key(dir: &Path) -> String {
    let p = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.components().collect());
    let s = p.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// A settings.json's contents, or None when it isn't there or isn't JSON
/// we can read (left to the strict writer to report).
fn read_value(path: &Path) -> Option<Value> {
    config_file::read(path).ok().flatten().and_then(|bytes| config_file::parse_json(Some(&bytes), "settings.json").ok())
}

/// The settings as they are, or an empty object when we cannot tell. Only for
/// read-only paths like `status()`, which must never fail loudly; anything that
/// writes goes through `config_file`, which surfaces the error instead.
fn read_settings_lossy() -> Value {
    config_file::read(&settings_path())
        .ok()
        .and_then(|bytes| config_file::parse_json(bytes.as_deref(), "settings.json").ok())
        .unwrap_or_else(|| json!({}))
}

/// On Windows Claude Code runs hook commands through Git Bash; on Linux through
/// `sh`. Either way the relay path is one quoted shell word.
fn hook_command(event: &str) -> String {
    agents::relay_command(Shell::Sh, event)
}

/// An entry Glim wrote, under the relay's current or old name.
fn entry_is_ours(entry: &Value) -> bool {
    entry_command_matches(entry, names_relay)
}

/// An entry that runs the current relay.
fn entry_is_current(entry: &Value) -> bool {
    entry_command_matches(entry, |c| c.contains(MARKER))
}

fn entry_command_matches(entry: &Value, test: impl Fn(&str) -> bool) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| hooks.iter().any(|h| h.get("command").and_then(Value::as_str).is_some_and(&test)))
}

/// The status line in settings.json is Glim's relay, under either name (old
/// installs wrote `coucou-hook StatusLine`, newer ones `… --statusline`).
fn status_line_is_ours(v: &Value) -> bool {
    v.get("command").and_then(Value::as_str).is_some_and(names_relay)
}

/// Settings with Coucou's hooks added; everything else is left untouched. A
/// `hooks` (or one of its events) that is not what Claude Code documents is
/// refused rather than replaced.
fn merged(existing: &Value) -> Result<Value, String> {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let mut hooks = match root.get("hooks") {
        None => Map::new(),
        Some(Value::Object(h)) => h.clone(),
        Some(_) => return Err(unexpected("\"hooks\"")),
    };

    for (event, timeout) in HOOK_EVENTS {
        let mut list = match hooks.get(*event) {
            None => Vec::new(),
            Some(Value::Array(list)) => list.clone(),
            Some(_) => return Err(unexpected(&format!("\"hooks\".\"{event}\""))),
        };
        list.retain(|entry| !entry_is_ours(entry));
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": hook_command(event),
                "timeout": timeout,
            }]
        }));
        hooks.insert((*event).to_string(), Value::Array(list));
    }

    root.insert("hooks".into(), Value::Object(hooks));
    Ok(Value::Object(root))
}

fn unexpected(what: &str) -> String {
    crate::i18n::tf("settings.json: {what} has an unexpected type — Glim has not touched it.", &[("what", what)])
}

/// Settings with every Coucou entry removed, and nothing else changed.
fn without_ours(existing: &Value) -> Result<Value, String> {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let hooks = match root.get("hooks") {
        None => return Ok(Value::Object(root)),
        Some(Value::Object(h)) => h.clone(),
        Some(_) => return Err(unexpected("\"hooks\"")),
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> =
                    list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Ok(Value::Object(root))
}

/// The files an install (or uninstall) changes, one per folder. A folder it
/// would leave as it is isn't touched at all (no rewrite, no backup), so a
/// folder already in sync keeps its file byte for byte. Uninstalling never
/// creates a file; installing creates one only where none is.
fn edits(install: bool) -> Vec<FileEdit<'static>> {
    edits_for(&config_dirs(), install)
}

fn edits_for(dirs: &[PathBuf], install: bool) -> Vec<FileEdit<'static>> {
    dirs.iter()
        .map(|d| d.join("settings.json"))
        .filter(|path| {
            let exists = path.is_file();
            match read_value(path) {
                Some(current) => {
                    let after = if install { merged(&current) } else { without_ours(&current) };
                    // A refusal (an odd "hooks") is kept, so the preview says so.
                    after.map_or(true, |a| a != current)
                }
                // Unreadable: kept, so the strict writer reports it. Missing:
                // only an install has something to write.
                None => exists || install,
            }
        })
        .map(|path| FileEdit {
            path,
            edit: config_file::json_edit("settings.json".into(), move |current| {
                if install { merged(current).map(Some) } else { without_ours(current).map(Some) }
            }),
        })
        .collect()
}

/// Whether Glim's current hooks are anywhere in `settings`.
fn has_current_hooks(settings: &Value) -> bool {
    settings
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|hooks| hooks.values().filter_map(Value::as_array).flatten().any(entry_is_current))
}

/// Each folder: hooks in it or not, and in sync or not.
fn folder_statuses(dirs: &[PathBuf]) -> Vec<FolderStatus> {
    dirs.iter()
        .map(|d| {
            let path = d.join("settings.json");
            let current = read_value(&path);
            let installed = current.as_ref().is_some_and(has_current_hooks);
            let up_to_date = current.as_ref().is_some_and(|c| merged(c).is_ok_and(|m| &m == c));
            FolderStatus { settings_path: path.to_string_lossy().to_string(), installed, up_to_date }
        })
        .collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn status() -> HookStatus {
    let current = read_settings_lossy();
    let folders = folder_statuses(&config_dirs());
    let installed = folders.iter().any(|f| f.installed);
    let hook_path = settings::hook_exe_path();
    HookStatus {
        installed,
        all_up_to_date: folders.iter().all(|f| f.up_to_date),
        folders,
        plan_relay_installed: plan_relay_installed(&current),
        settings_path: settings_path().to_string_lossy().to_string(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    }
}

pub fn preview(install: bool) -> Result<HookPreview, String> {
    let edits = edits(install);
    if edits.is_empty() {
        return Ok(HookPreview { diff: crate::i18n::t("No change."), backup: String::new(), settings_path: String::new(), fingerprint: NOTHING.into() });
    }
    Ok(plan_to_preview(config_file::preview(&edits)?))
}

/// The fingerprint of a preview with nothing to write.
const NOTHING: &str = "nothing";

/// At launch: says in the log when the folders aren't in sync (a folder
/// without Glim's hooks, or an older relay command). Writes nothing: the
/// fix is Settings → Claude Code → Update hooks, after the diff.
pub fn log_sync_state() {
    let (dirs, skipped) = find_config_dirs(&platform::home_dir(), std::env::var_os(CONFIG_DIR_VAR).map(PathBuf::from));
    if !skipped.is_empty() {
        // Quoted, so a trailing space shows: ".claude2 " is not ".claude2".
        let names: Vec<String> = skipped.iter().map(|n| format!("{n:?}")).collect();
        crate::log::line(format!("hooks: not a Claude Code config folder, skipped: {}", names.join(", ")));
    }
    let folders = folder_statuses(&dirs);
    if folders.iter().any(|f| f.installed) && !folders.iter().all(|f| f.up_to_date) {
        let behind: Vec<&str> = folders.iter().filter(|f| !f.up_to_date).map(|f| f.settings_path.as_str()).collect();
        crate::log::line(format!("hooks: not in sync in {} (Settings → Claude Code → Update hooks)", behind.join(", ")));
    }
}

fn plan_to_preview(plan: config_file::Plan) -> HookPreview {
    HookPreview {
        diff: plan.diff,
        backup: plan.backup,
        settings_path: plan.path,
        fingerprint: plan.fingerprint,
    }
}

/// Writes the merged (or cleaned) settings after taking a dated backup, and
/// returns where the backup went ("" when there was no file to back up).
///
/// `fingerprint` is the one the preview was computed from: a settings.json that
/// changed in between is refused rather than overwritten (see config_file).
pub fn write(install: bool, fingerprint: &str) -> Result<String, String> {
    let edits = edits(install);
    if edits.is_empty() && fingerprint == NOTHING {
        return Ok(String::new());
    }
    let backups = config_file::apply(&edits, fingerprint)?;
    let done = if install { "installed" } else { "removed" };
    crate::log::line(format!("hooks {done} in {} folder(s)", edits.len()));
    Ok(backups.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"))
}

// ── Status line (plan usage) ──────────────────────────────────────────────────
//
// Claude Code runs one `statusLine` command and hands it the plan limits. Coucou
// puts its relay there; a status line the user already had is kept in
// statusline-previous.json beside the relay, and the relay still runs it, so it
// keeps working. Installing and removing it is separate from the hooks, and
// goes through the same hardened writer (config_file.rs): strict read, diff,
// fingerprint, dated backup, then the write.

/// The command patterns of the user's Claude Code "ask" rules, from every
/// config folder's settings.json (`permissions.ask`: "Bash(git merge*)",
/// "PowerShell(*rm -rf *)"), the part inside the parentheses. Read only, on
/// each new approval card: a command matching one is never allowed by the
/// Allow key (fix e, 2026-10-17), only by a click.
pub fn ask_patterns() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in config_dirs() {
        let Ok(bytes) = std::fs::read(dir.join("settings.json")) else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { continue };
        for p in ask_patterns_in(&v) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// `permissions.ask`'s command rules: "Bash(x)" / "PowerShell(x)" → "x".
/// Rules for other tools (Edit, WebFetch…) have no command to match.
pub fn ask_patterns_in(settings: &Value) -> Vec<String> {
    let rules = settings.pointer("/permissions/ask").and_then(Value::as_array).cloned().unwrap_or_default();
    rules
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|r| ["Bash(", "PowerShell("].iter().find_map(|t| r.strip_prefix(t)))
        .filter_map(|r| r.strip_suffix(')'))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Where the user's own status line waits while the relay stands in for it.
pub fn status_line_previous_path() -> PathBuf {
    settings::hook_exe_path().with_file_name("statusline-previous.json")
}

fn read_status_line_previous() -> Option<Value> {
    let bytes = std::fs::read(status_line_previous_path()).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok().filter(Value::is_object)
}

/// True when the `statusLine` in settings.json is the current relay.
pub fn plan_relay_installed(settings: &Value) -> bool {
    settings
        .get("statusLine")
        .and_then(|v| v.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(MARKER))
}

/// `statusLine` as it reads after installing or removing the relay. `None`: the
/// key goes. Installing swaps only `command`, so `padding`, `refreshInterval`
/// and the rest of the user's status line stay as they were.
fn status_line_after(existing: Option<&Value>, install: bool, previous: Option<&Value>) -> Option<Value> {
    if install {
        let mut sl = existing.filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
        let obj = sl.as_object_mut().expect("an object");
        obj.entry("type").or_insert_with(|| json!("command"));
        obj.insert("command".into(), json!(hook_command("--statusline")));
        Some(sl)
    } else if existing.is_some_and(status_line_is_ours) {
        previous.cloned()
    } else {
        existing.cloned() // not ours any more (the user changed it): leave it alone
    }
}

fn status_line_settings(current: &Value, install: bool, previous: Option<&Value>) -> Result<Value, String> {
    let mut root = current.as_object().cloned().unwrap_or_default();
    // A statusLine that is not an object is something we do not understand:
    // refuse rather than replace it.
    if install && root.get("statusLine").is_some_and(|v| !v.is_object()) {
        return Err(unexpected("\"statusLine\""));
    }
    match status_line_after(root.get("statusLine"), install, previous) {
        Some(sl) => root.insert("statusLine".into(), sl),
        None => root.remove("statusLine"),
    };
    Ok(Value::Object(root))
}

fn status_line_edits(install: bool, previous: Option<Value>) -> Vec<FileEdit<'static>> {
    vec![FileEdit {
        path: settings_path(),
        edit: config_file::json_edit("settings.json".into(), move |current| {
            Ok(Some(status_line_settings(current, install, previous.as_ref())?))
        }),
    }]
}

/// The diff the user has to look at before the relay goes in or out.
pub fn status_line_preview(install: bool) -> Result<HookPreview, String> {
    Ok(plan_to_preview(config_file::preview(&status_line_edits(install, read_status_line_previous()))?))
}

/// Installs or removes the relay, after the same backup and fingerprint checks as
/// the hooks. Installing first saves a status line of the user's own, removing
/// puts it back (or removes the key if there was none).
pub fn status_line_write(install: bool, fingerprint: &str) -> Result<String, String> {
    let before = config_file::read(&settings_path())
        .and_then(|bytes| config_file::parse_json(bytes.as_deref(), "settings.json"))?;
    let previous = read_status_line_previous();
    let own = before.get("statusLine").filter(|v| !status_line_is_ours(v)).cloned();
    let saved = match (install, own.as_ref()) {
        (true, Some(own)) => {
            save_status_line_previous(own).map_err(|_| {
                crate::i18n::t("Could not save your current status line next to the relay; nothing was changed.")
            })?;
            true
        }
        _ => false,
    };
    let result = config_file::apply(&status_line_edits(install, previous), fingerprint);
    match &result {
        // Back in settings.json: the saved copy has done its job.
        Ok(_) if !install => {
            let _ = std::fs::remove_file(status_line_previous_path());
        }
        // Nothing was written: do not leave a stale copy behind.
        Err(_) if saved => {
            let _ = std::fs::remove_file(status_line_previous_path());
        }
        _ => {}
    }
    let backups = result?;
    Ok(backups.first().map(|p| p.to_string_lossy().to_string()).unwrap_or_default())
}

fn save_status_line_previous(status_line: &Value) -> std::io::Result<()> {
    let path = status_line_previous_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    config_file::write_like(&path, &path, config_file::pretty(status_line).as_bytes())
}

/// Copies the relay (glim-hook.exe / glim-hook) into the local data dir's
/// bin/ on launch. In a bundled install it comes from the app resources; in
/// `tauri dev` it sits next to the app binary in the workspace target directory.
///
/// Every candidate is tried rather than just the first, because getting this
/// wrong is silent and fatal: `resources` used to be a glob, which made NSIS
/// mirror the source path into `_up_\target\release\`, no candidate matched, and
/// the relay was simply never installed. It only looked healthy on a developer
/// machine, where a leftover copy from `tauri dev` was already sitting in bin/.
pub fn ensure_hook_exe(app: &AppHandle) {
    let dest = settings::hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    // Nobody else may swap the relay Claude Code runs: its folder is ours only.
    if platform::ensure_private_dir(&settings::local_dir()).is_err()
        || std::fs::create_dir_all(dir).is_err()
    {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app.path().resolve(platform::HOOK_EXE, tauri::path::BaseDirectory::Resource) {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Installed build, then `tauri dev` (target/debug) next to the
            // release hook the pre-build step produces.
            candidates.push(parent.join(platform::HOOK_EXE));
            candidates.push(parent.join("../release").join(platform::HOOK_EXE));
            // Belt and braces: where the old glob form used to land it.
            candidates.push(parent.join("_up_/target/release").join(platform::HOOK_EXE));
        }
    }

    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    let Some(src) = candidates.into_iter().find(|p| p.exists()) else {
        crate::log::line(format!(
            "{} not found — Claude Code hooks cannot work. Looked in: {}",
            platform::HOOK_EXE,
            tried.join(", ")
        ));
        return;
    };
    install_relay(&src, &dest);
}

#[cfg(windows)]
fn install_relay(src: &Path, dest: &Path) {
    let same = match (std::fs::metadata(src), std::fs::metadata(dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine, it is the same relay.
    if let Err(err) = std::fs::copy(src, dest) {
        if !dest.exists() {
            crate::log::line(format!("could not install {}: {err}", platform::HOOK_EXE));
        }
    }
}

/// Linux does not keep the modification time on copy, so the contents decide.
/// The new relay is written beside the old one and renamed over it: a hook
/// starting at that moment runs either the old relay or the new one, never half
/// of one, and a relay that is running right now does not block the update.
#[cfg(unix)]
fn install_relay(src: &Path, dest: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if matches!((std::fs::read(src), std::fs::read(dest)), (Ok(a), Ok(b)) if a == b) {
        return;
    }
    let temp = dest.with_extension(format!("new-{}", std::process::id()));
    let result = std::fs::copy(src, &temp)
        .and_then(|_| std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&temp, dest));
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        crate::log::line(format!("could not install {}: {err}", platform::HOOK_EXE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fix e: the Allow key never allows what an "ask" rule names.
    #[test]
    fn the_ask_rules_command_patterns_are_read_as_written() {
        let s = json!({ "permissions": {
            "ask": ["Bash(git merge*)", "PowerShell(*Remove-Item* -Recurse*-Force*)", "Bash(*DROP TABLE*)", "Edit(*.env)", "WebFetch", 7, "Bash()"],
            "allow": ["Bash(ls *)"],
        } });
        assert_eq!(ask_patterns_in(&s), ["git merge*", "*Remove-Item* -Recurse*-Force*", "*DROP TABLE*"]);
        assert!(ask_patterns_in(&json!({})).is_empty());
        assert!(ask_patterns_in(&json!({ "permissions": { "ask": "Bash(x)" } })).is_empty());
    }

    #[test]
    fn the_hooks_leave_the_status_line_alone() {
        let theirs = json!({ "statusLine": { "type": "command", "command": "~/bin/my-line" } });
        assert_eq!(merged(&theirs).unwrap()["statusLine"], theirs["statusLine"]);
        assert_eq!(without_ours(&theirs).unwrap()["statusLine"], theirs["statusLine"]);
        assert!(merged(&json!({})).unwrap().get("statusLine").is_none());
    }

    #[test]
    fn the_relay_takes_the_status_line_and_keeps_the_users_other_fields() {
        // None yet: ours is added.
        let fresh = status_line_settings(&json!({ "model": "opus" }), true, None).unwrap();
        assert!(status_line_is_ours(&fresh["statusLine"]));
        assert_eq!(fresh["statusLine"]["type"], "command");
        assert_eq!(fresh["model"], "opus");

        // Their own: only the command is swapped; padding and refresh stay.
        let own = json!({ "statusLine": { "type": "command", "command": "~/bin/my-line", "padding": 2, "refreshInterval": 5 } });
        let taken = status_line_settings(&own, true, None).unwrap();
        assert!(status_line_is_ours(&taken["statusLine"]));
        assert_eq!(taken["statusLine"]["padding"], 2);
        assert_eq!(taken["statusLine"]["refreshInterval"], 5);

        // Installing again keeps ours and its extra fields.
        let again = status_line_settings(&taken, true, None).unwrap();
        assert_eq!(again["statusLine"]["padding"], 2);
        assert!(status_line_is_ours(&again["statusLine"]));
    }

    #[test]
    fn removing_the_relay_restores_what_was_there_and_never_touches_anything_else() {
        let previous = json!({ "type": "command", "command": "~/bin/my-line", "padding": 2 });
        let ours = status_line_settings(&json!({}), true, None).unwrap();

        // There was one before: it comes back exactly.
        assert_eq!(status_line_settings(&ours, false, Some(&previous)).unwrap()["statusLine"], previous);
        // There was none: the key goes.
        assert!(status_line_settings(&ours, false, None).unwrap().get("statusLine").is_none());
        // The user changed it since: not ours, so untouched.
        let theirs = json!({ "statusLine": { "type": "command", "command": "~/bin/other" } });
        assert_eq!(status_line_settings(&theirs, false, Some(&previous)).unwrap()["statusLine"], theirs["statusLine"]);
        // A statusLine we do not understand is refused, never replaced.
        assert!(status_line_settings(&json!({ "statusLine": "echo hi" }), true, None).is_err());
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = serde_json::json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                ]
            }
        });

        let after = merged(&existing).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["enabledPlugins"], serde_json::json!(["a", "b"]));

        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("someone-elses-tool.exe")),
            "another tool's hook was dropped"
        );
        assert!(pre.iter().any(entry_is_ours), "our own hook was not added");
        assert!(after["hooks"]["SomeEventWeDoNotTouch"].is_array());

        // Installing twice still leaves one entry of ours per event. (Compared
        // by count: another test points HOME elsewhere meanwhile, which moves
        // the relay path.)
        let twice = merged(&after).unwrap();
        assert_eq!(twice["hooks"]["PreToolUse"].as_array().unwrap().iter().filter(|e| entry_is_ours(e)).count(), 1);

        // And removing ours puts it back exactly as it was.
        let cleaned = without_ours(&after).unwrap();
        assert_eq!(cleaned, existing);
    }

    #[test]
    fn values_of_an_unexpected_type_are_refused_not_replaced() {
        for odd in [
            json!({ "hooks": "a string" }),
            json!({ "hooks": [1, 2] }),
            json!({ "hooks": { "PreToolUse": { "not": "a list" } } }),
        ] {
            assert!(merged(&odd).is_err(), "{odd}");
        }
        assert!(without_ours(&json!({ "hooks": 3 })).is_err());
        // An event we do not install stays as it is, whatever its shape.
        let kept = json!({ "hooks": { "Custom": "anything" } });
        assert_eq!(without_ours(&kept).unwrap(), kept);
    }

    /// Everything filesystem-shaped lives in one test on purpose: it points
    /// the home directory at a temp directory, and that is process-wide.
    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let tmp = std::env::temp_dir().join(format!("glim-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        used_by_claude(&tmp.join(".claude"), "history.jsonl");
        std::env::set_var(platform::HOME_VAR, &tmp);

        let path = settings_path();
        assert!(path.starts_with(&tmp), "the test must not touch the real home");

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();

        // Install.
        let plan = preview(true).expect("a BOM must not stop the preview");
        assert!(plan.diff.contains("glim-hook"), "the diff must show what changes");
        let backup = write(true, &plan.fingerprint).expect("install should succeed");

        // The backup holds the original bytes, BOM and all.
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        // Everything else survived, and so did the other tool's hook.
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["tui"]["x"], 1);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool.exe")));
        assert!(status().installed);

        // A file that moved since the preview is refused, and left alone.
        let stale = preview(false).unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = write(false, &stale.fingerprint).unwrap_err();
        // Either it changed, or (here) there's nothing of Glim's left in it:
        // the set of files differs from the preview's. Nothing is written.
        assert!(err.contains("Nothing was written"), "got: {err}");
        let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview(true).is_err());
        assert!(write(true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Every Claude config folder (owner, 2026-10-12) ──────────────────────
    //
    // These tests pass folders explicitly and never read CLAUDE_CONFIG_DIR or
    // the real home: a test must never write the owner's ~/.claude2.

    fn scratch_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("glim-hooks-dirs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn write_json(path: &Path, v: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(v).unwrap()).unwrap();
    }

    fn json_at(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn ours_per_event(v: &Value) -> Vec<usize> {
        HOOK_EVENTS
            .iter()
            .map(|(e, _)| v["hooks"][*e].as_array().map_or(0, |l| l.iter().filter(|x| entry_is_ours(x)).count()))
            .collect()
    }

    /// What Claude Code leaves in a config folder it has used (the markers
    /// `looks_like_claude_config` checks), with or without a settings.json.
    fn used_by_claude(dir: &Path, marker: &str) {
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        std::fs::write(dir.join(marker), b"").unwrap();
    }

    #[test]
    fn every_claude_folder_is_found_once() {
        let home = scratch_home("find");
        used_by_claude(&home.join(".claude"), "history.jsonl"); // no settings.json yet: still listed
        used_by_claude(&home.join(".claude2"), ".claude.json");
        write_json(&home.join(".claude2").join("settings.json"), &json!({}));
        used_by_claude(&home.join(".claude10"), "history.jsonl");
        std::fs::write(home.join(".claude.json"), b"{}").unwrap(); // a file, not a folder
        let elsewhere = home.join("elsewhere");
        used_by_claude(&elsewhere, "history.jsonl");

        let none = config_dirs_in(&home, None);
        assert_eq!(none, [home.join(".claude"), home.join(".claude10"), home.join(".claude2")]);
        // CLAUDE_CONFIG_DIR naming a folder already found: once only.
        let spelled = PathBuf::from(format!("{}{}", home.join(".claude2").display(), std::path::MAIN_SEPARATOR));
        assert_eq!(config_dirs_in(&home, Some(spelled)).len(), 3);
        #[cfg(windows)]
        assert_eq!(config_dirs_in(&home, Some(PathBuf::from(home.join(".CLAUDE2").to_string_lossy().to_string()))).len(), 3);
        // A folder elsewhere comes second, whatever its name.
        assert_eq!(config_dirs_in(&home, Some(elsewhere.clone()))[1], elsewhere);
        // One that isn't there is not created or listed; nor is an empty value.
        assert_eq!(config_dirs_in(&home, Some(home.join("missing"))).len(), 3);
        assert_eq!(config_dirs_in(&home, Some(PathBuf::new())).len(), 3);
        assert!(!home.join("missing").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn other_tools_folders_and_odd_names_are_never_config_folders() {
        let home = scratch_home("others");
        used_by_claude(&home.join(".claude"), "history.jsonl");
        // The owner's real neighbours (2026-10-13): each holds a settings.json
        // or a projects folder, and none is Claude Code's.
        for name in [".claude-mem", ".claude-flow", ".claude2 ", ".claude-something", ".claude2b", ".claude_3", ".claude-2"] {
            // Windows drops a trailing space from an ordinary path (".claude2 "
            // would land in .claude2); a \\?\ path makes the folder as named.
            let dir = if cfg!(windows) { PathBuf::from(format!(r"\\?\{}", home.join(name).display())) } else { home.join(name) };
            std::fs::create_dir_all(&dir).unwrap();
            write_json(&dir.join("settings.json"), &json!({ "tool": name }));
            used_by_claude(&dir, "history.jsonl"); // even looking like a config doesn't make the name count
        }
        assert!(!home.join(".claude2").exists(), "the trailing-space folder is its own folder");
        let (dirs, skipped) = find_config_dirs(&home, None);
        assert_eq!(dirs, [home.join(".claude")]);
        for name in [".claude-mem", ".claude-flow", ".claude2 ", ".claude-something", ".claude2b", ".claude_3", ".claude-2"] {
            assert!(skipped.contains(&name.to_string()), "{name} skipped and logged: {skipped:?}");
        }
        // Logged by folder name only, never a path.
        assert!(skipped.iter().all(|n| !n.contains('/') && !n.contains('\\')), "{skipped:?}");
        assert!(edits_for(&dirs, true).iter().all(|e| e.path.starts_with(home.join(".claude"))));
        assert!(numbered_claude_name(".claude2") && numbered_claude_name(".CLAUDE3"));
        for name in [".claude2 ", ".claude", ".claude-mem", ".claude 2", ".claude2\u{a0}", ".claude²", ".claudé2", "claude2"] {
            assert!(!numbered_claude_name(name), "{name:?}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_owners_real_claude2_counts_and_its_trailing_space_twin_does_not() {
        // Shaped like the owner's home (2026-10-13): .claude and .claude2 as
        // Claude Code leaves them, and ".claude2 " beside .claude2 holding
        // only a projects folder.
        let home = scratch_home("owner");
        for real in [".claude", ".claude2"] {
            let dir = home.join(real);
            used_by_claude(&dir, "history.jsonl");
            std::fs::write(dir.join(".claude.json"), b"{}").unwrap();
            std::fs::write(dir.join(".credentials.json"), b"").unwrap();
            write_json(&dir.join("settings.json"), &json!({}));
            for sub in ["sessions", "shell-snapshots", "file-history", "plugins"] {
                std::fs::create_dir_all(dir.join(sub)).unwrap();
            }
        }
        let twin = if cfg!(windows) { PathBuf::from(format!(r"\\?\{}", home.join(".claude2 ").display())) } else { home.join(".claude2 ") };
        std::fs::create_dir_all(twin.join("projects")).unwrap();
        let (dirs, skipped) = find_config_dirs(&home, None);
        assert_eq!(dirs, [home.join(".claude"), home.join(".claude2")]);
        assert_eq!(dirs[1].file_name().unwrap().len(), 8, "the real .claude2, no trailing space");
        assert_eq!(skipped, [".claude2 "]);
        assert_eq!(skipped[0].len(), 9);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_claude_code_never_used_is_left_out_even_when_named_right() {
        let home = scratch_home("unused");
        // A bare ~/.claude, a .claude3 with only settings.json, a .claude4 with
        // projects but no history, and CLAUDE_CONFIG_DIR naming an empty folder.
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        write_json(&home.join(".claude3").join("settings.json"), &json!({}));
        std::fs::create_dir_all(home.join(".claude4").join("projects")).unwrap();
        let named = home.join("named");
        std::fs::create_dir_all(&named).unwrap();
        let (dirs, skipped) = find_config_dirs(&home, Some(named));
        assert!(dirs.is_empty(), "{dirs:?}");
        assert_eq!(skipped, [".claude", ".claude3", ".claude4", "named"]);
        // Nothing is created anywhere by looking.
        assert!(!home.join(".claude").join("settings.json").exists());
        assert!(!home.join("named").join("projects").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_reaches_every_folder_backs_each_up_and_never_duplicates_hand_copied_hooks() {
        let home = scratch_home("install");
        let (main, two, work) = (home.join(".claude"), home.join(".claude2"), home.join(".claude-work"));
        std::fs::create_dir_all(&main).unwrap(); // no settings.json: created
        // .claude2: the owner's hand copy, exactly what Glim writes, plus their own settings.
        let copied = merged(&json!({ "model": "opus", "env": { "X": "1" } })).unwrap();
        write_json(&two.join("settings.json"), &copied);
        let copied_bytes = std::fs::read(two.join("settings.json")).unwrap();
        // .claude-work: another tool's hook and an older Glim relay path.
        let old_path = json!({ "theme": "dark", "hooks": {
            "Stop": [
                { "hooks": [{ "type": "command", "command": "other-tool.exe" }] },
                { "hooks": [{ "type": "command", "command": "\"C:/Old/Place/bin/glim-hook.exe\" Stop", "timeout": 10 }] }
            ]
        } });
        write_json(&work.join("settings.json"), &old_path);
        let dirs = vec![main.clone(), two.clone(), work.clone()];

        // The hand copy is recognised as Glim's: in sync already, so not touched.
        let before = folder_statuses(&dirs);
        assert_eq!(before.iter().map(|f| (f.installed, f.up_to_date)).collect::<Vec<_>>(), [(false, false), (true, true), (true, false)]);
        let edits = edits_for(&dirs, true);
        assert_eq!(edits.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), [main.join("settings.json"), work.join("settings.json")]);

        let plan = config_file::preview(&edits).unwrap();
        assert!(plan.diff.contains(".claude-work"), "one preview names every folder: {}", plan.diff);
        let backups = config_file::apply(&edits, &plan.fingerprint).unwrap();
        assert_eq!(backups.len(), 1, "a backup of each file that was there");
        assert_eq!(json_at(&backups[0]), old_path);

        assert_eq!(std::fs::read(two.join("settings.json")).unwrap(), copied_bytes, ".claude2 byte for byte");
        let created = json_at(&main.join("settings.json"));
        let updated = json_at(&work.join("settings.json"));
        for v in [&created, &updated] {
            assert!(ours_per_event(v).iter().all(|n| *n == 1), "one Glim entry per event: {v}");
            assert!(has_current_hooks(v));
        }
        assert_eq!(updated["theme"], "dark");
        assert_eq!(updated["hooks"]["Stop"][0]["hooks"][0]["command"], "other-tool.exe");
        assert!(!updated.to_string().contains("C:/Old/Place"), "the old relay path is replaced");
        assert!(folder_statuses(&dirs).iter().all(|f| f.installed && f.up_to_date));
        // Installing again: nothing to do anywhere.
        assert!(edits_for(&dirs, true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_that_appears_later_is_picked_up_by_the_next_install() {
        let home = scratch_home("later");
        used_by_claude(&home.join(".claude"), "history.jsonl");
        write_json(&home.join(".claude").join("settings.json"), &merged(&json!({})).unwrap());
        assert!(edits_for(&config_dirs_in(&home, None), true).is_empty());
        write_json(&home.join(".claude3").join("settings.json"), &json!({ "model": "sonnet" }));
        assert_eq!(config_dirs_in(&home, None).len(), 1, "not until Claude Code has used it");
        used_by_claude(&home.join(".claude3"), ".claude.json");
        let dirs = config_dirs_in(&home, None);
        let statuses = folder_statuses(&dirs);
        assert_eq!(statuses.iter().map(|f| f.up_to_date).collect::<Vec<_>>(), [true, false]);
        let edits = edits_for(&dirs, true);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path, home.join(".claude3").join("settings.json"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn uninstall_takes_glims_hooks_out_of_every_folder_and_nothing_else() {
        let home = scratch_home("uninstall");
        let (main, two, bare) = (home.join(".claude"), home.join(".claude2"), home.join("bare"));
        let theirs = json!({ "model": "opus", "hooks": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "keep-me.exe" }] }] } });
        write_json(&main.join("settings.json"), &merged(&theirs).unwrap());
        write_json(&two.join("settings.json"), &merged(&json!({ "env": { "A": "b" } })).unwrap());
        std::fs::create_dir_all(&bare).unwrap(); // no settings.json: never created by an uninstall
        let dirs = vec![main.clone(), two.clone(), bare.clone()];

        let edits = edits_for(&dirs, false);
        assert_eq!(edits.len(), 2);
        let plan = config_file::preview(&edits).unwrap();
        let backups = config_file::apply(&edits, &plan.fingerprint).unwrap();
        assert_eq!(backups.len(), 2);
        assert_eq!(json_at(&main.join("settings.json")), theirs);
        assert_eq!(json_at(&two.join("settings.json")), json!({ "env": { "A": "b" } }));
        assert!(!bare.join("settings.json").exists());
        assert!(folder_statuses(&dirs).iter().all(|f| !f.installed));
        assert!(edits_for(&dirs, false).is_empty(), "nothing left to remove");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_that_changed_since_the_preview_stops_every_write() {
        let home = scratch_home("stale");
        let (a, b) = (home.join(".claude"), home.join(".claude2"));
        write_json(&a.join("settings.json"), &json!({ "model": "a" }));
        write_json(&b.join("settings.json"), &json!({ "model": "b" }));
        let dirs = vec![a.clone(), b.clone()];
        let plan = config_file::preview(&edits_for(&dirs, true)).unwrap();
        write_json(&b.join("settings.json"), &json!({ "model": "edited" }));
        assert!(config_file::apply(&edits_for(&dirs, true), &plan.fingerprint).is_err());
        assert_eq!(json_at(&a.join("settings.json")), json!({ "model": "a" }), "nothing written anywhere");
        let _ = std::fs::remove_dir_all(&home);
    }

    // ── Entries from before the rename (coucou-hook) ──────────────────────────

    fn legacy_settings() -> Value {
        json!({
            "hooks": {
                "Stop": [
                    {"hooks": [{"type": "command", "command": "\"C:/Users/me/AppData/Local/Coucou/bin/coucou-hook.exe\" Stop", "timeout": 10}]},
                    {"hooks": [{"type": "command", "command": "other-tool.exe"}]}
                ]
            },
            "statusLine": {"type": "command", "command": "\"C:/Users/me/AppData/Local/Coucou/bin/coucou-hook.exe\" --statusline", "padding": 1}
        })
    }

    #[test]
    fn old_entries_are_ours_but_do_not_read_as_installed() {
        let legacy = legacy_settings();
        let old = &legacy["hooks"]["Stop"][0];
        assert!(entry_is_ours(old), "an install must replace it and an uninstall remove it");
        assert!(!entry_is_current(old), "its relay path is gone, so it must not read as installed");
        assert!(!entry_is_ours(&legacy["hooks"]["Stop"][1]));
        assert!(status_line_is_ours(&legacy["statusLine"]));
        assert!(!plan_relay_installed(&legacy));
    }

    #[test]
    fn installing_over_old_entries_replaces_them_and_installing_again_changes_nothing() {
        let once = merged(&legacy_settings()).unwrap();
        let twice = merged(&once).unwrap();
        assert_eq!(once, twice, "a second install must not add duplicates");
        for (event, _) in HOOK_EVENTS {
            let ours: Vec<&Value> = once["hooks"][*event].as_array().unwrap().iter().filter(|e| entry_is_ours(e)).collect();
            assert_eq!(ours.len(), 1, "{event}: exactly one Glim entry");
            assert!(entry_is_current(ours[0]), "{event}: and it runs glim-hook");
        }
        let stop = once["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0]["hooks"][0]["command"], "other-tool.exe", "the other tool's entry stays first");
        assert!(!once["hooks"].to_string().contains(crate::agents::LEGACY_MARKER));
        // The status line is a separate switch: installing hooks leaves it alone.
        assert_eq!(once["statusLine"], legacy_settings()["statusLine"]);
    }

    #[test]
    fn removing_takes_out_old_entries_too() {
        let removed = without_ours(&legacy_settings()).unwrap();
        assert_eq!(removed["hooks"], json!({"Stop": [{"hooks": [{"type": "command", "command": "other-tool.exe"}]}]}));
    }

    #[test]
    fn an_old_status_line_relay_is_swapped_for_the_new_one_or_removed() {
        let legacy = legacy_settings();
        let installed = status_line_after(legacy.get("statusLine"), true, None).unwrap();
        assert!(installed["command"].as_str().unwrap().contains("glim-hook"), "{installed}");
        assert_eq!(installed["padding"], 1, "the rest of the status line stays");
        assert!(plan_relay_installed(&json!({ "statusLine": installed })));
        assert_eq!(status_line_after(legacy.get("statusLine"), false, None), None);
    }
}
