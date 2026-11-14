// Runbooks v1 (local agency v1, part 2; owner 2026-11-12).
//
// A runbook is a TOML file the owner puts in %APPDATA%\Glim\runbooks\
// (never in the repo: examples/runbook.example.toml has placeholders only).
// Its steps run in order and stop at the first failure. Only six kinds:
//
// - `env_secret`: a saved secret (secrets.rs) written as KEY=value into a
//   file under an allowlisted folder, only when git says that file is
//   ignored and not tracked (env_file.rs); the old file's backup goes in
//   %LOCALAPPDATA%\Glim\backups\<runbook>\, never in a repository.
//   `require_prefix = [...]`: the value must start with one of them, or
//   nothing is written (the refusal names neither the value nor a prefix).
// - `start_app`: an allowlisted .exe/.lnk or protocol, no arguments, no shell.
// - `run`: an exact allowlisted command line in an allowlisted folder, with
//   a timeout; the exit code is logged, the output is thrown away.
// - `ps_script`: an allowlisted .ps1 ([allow] scripts) through System32's
//   Windows PowerShell (-NoProfile -ExecutionPolicy Bypass -File <script>
//   <fixed args>), with a timeout. The script's SHA-256 is part of the
//   approved hash, so a changed script means approving again (and it is
//   hashed again just before it runs). The exit code is logged; the output
//   never is: its last few KB, with any stdin secret blanked out, go to
//   %LOCALAPPDATA%\Glim\runbooks-output\<runbook>.txt for Settings.
// - `secret_stdin = "<name>"` on run / ps_script: that saved secret is
//   written to the process's stdin and nowhere else (never argv, env or a log).
// - `check`: a file exists / a port on this PC answers / a URL on this PC
//   returns 200. Read only.
// - `launch_detached`: an exact allowlisted command line in an allowlisted
//   folder, started in a console of its own and not waited for; it keeps
//   running when Glim stops (so an updater Glim starts can stop Glim). Only
//   as the runbook's last step.
//
// Approval: the first run shows every step (secret NAMES only) and needs
// "Approve runbook". Glim keeps the file's SHA-256 with a MAC whose key is
// in Credential Manager, so any change to the file, or a hand-edited
// approvals file, means approving again. Steps with `ask = true` ask every
// run (and the command line refuses them).
//
// Triggers: Settings → Runbooks (Run), voice ("run <name>", matched on this
// PC: nothing goes to Claude), and `glim.exe --run <name>` (cli.rs).
// Audit: %LOCALAPPDATA%\Glim\runbooks.log (time, runbook, step, outcome;
// never a value or a command's output).
//
// Messages about runbook files and steps are English, like the file's own
// keywords, and are the same in Settings and on the command line.

pub mod cli;
pub mod env_file;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ai::key::Vault;
use crate::secrets;

/// Glim's own credential for the approvals' MAC key (secrets::RESERVED has it).
pub const APPROVAL_KEY_TARGET: &str = "Glim/RunbookApprovalKey";

pub const MAX_STEPS: usize = 30;
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
pub const DEFAULT_TIMEOUT_S: u64 = 120;
pub const MAX_TIMEOUT_S: u64 = 3600;
pub const MAX_WAIT_S: u64 = 600;
pub const MAX_NAME: usize = 40;
/// How many runs Settings shows.
pub const HISTORY: usize = 20;

/// %APPDATA%\Glim\runbooks
pub fn dir() -> PathBuf {
    crate::platform::config_dir().join("runbooks")
}

fn approvals_path() -> PathBuf {
    crate::platform::config_dir().join("runbooks-approved.json")
}

/// %LOCALAPPDATA%\Glim\backups: env_secret's backups, one folder per runbook.
pub fn backups_dir() -> PathBuf {
    crate::settings::local_dir().join("backups")
}

/// %LOCALAPPDATA%\Glim\runbooks-output: each runbook's last ps_script output (redacted).
pub fn outputs_dir() -> PathBuf {
    crate::settings::local_dir().join("runbooks-output")
}

pub fn audit_path() -> PathBuf {
    crate::settings::local_dir().join("runbooks.log")
}

// ── The file ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawBook {
    #[serde(default)]
    description: String,
    #[serde(default)]
    allow: RawAllow,
    #[serde(default)]
    step: Vec<RawStep>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAllow {
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    apps: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    scripts: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawStep {
    #[serde(rename = "type")]
    kind: String,
    secret: Option<String>,
    file: Option<String>,
    key: Option<String>,
    app: Option<String>,
    command: Option<String>,
    folder: Option<String>,
    timeout_s: Option<u64>,
    file_exists: Option<String>,
    port: Option<u16>,
    url: Option<String>,
    wait_s: Option<u64>,
    script: Option<String>,
    args: Option<Vec<String>>,
    secret_stdin: Option<String>,
    require_prefix: Option<Vec<String>>,
    #[serde(default)]
    ask: bool,
}

/// What a `start_app` step may start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum App {
    /// A full local path to an .exe or .lnk.
    Path(String),
    /// A protocol link ("example-app:", "example-app://open").
    Protocol(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    FileExists(PathBuf),
    Port(u16),
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    EnvSecret { secret: String, file: PathBuf, key: String, require_prefix: Vec<String> },
    StartApp { app: App },
    Run { command: String, folder: PathBuf, timeout_s: u64, stdin_secret: Option<String> },
    PsScript { script: PathBuf, sha256: String, args: Vec<String>, folder: PathBuf, timeout_s: u64, stdin_secret: Option<String> },
    Check { check: Check, wait_s: u64 },
    LaunchDetached { command: String, folder: PathBuf },
}

impl Step {
    pub fn kind(&self) -> &'static str {
        match self {
            Step::EnvSecret { .. } => "env_secret",
            Step::StartApp { .. } => "start_app",
            Step::Run { .. } => "run",
            Step::PsScript { .. } => "ps_script",
            Step::Check { .. } => "check",
            Step::LaunchDetached { .. } => "launch_detached",
        }
    }

    /// One line saying what the step does: names, paths and commands, never
    /// a secret's value.
    pub fn describe(&self) -> String {
        match self {
            Step::EnvSecret { secret, file, key, require_prefix } => {
                let only = if require_prefix.is_empty() { String::new() } else { format!(" (only if it starts with {})", require_prefix.join(" or ")) };
                format!("Write secret {secret} as {key}=… into {}{only}", show(file))
            }
            Step::StartApp { app: App::Path(p) } => format!("Start {p}"),
            Step::StartApp { app: App::Protocol(p) } => format!("Open {p}"),
            Step::Run { command, folder, timeout_s, stdin_secret } => format!("Run `{command}` in {} (stops after {timeout_s} s){}", show(folder), stdin_note(stdin_secret)),
            Step::PsScript { script, sha256, args, folder, timeout_s, stdin_secret } => {
                let a = if args.is_empty() { String::new() } else { format!(" with arguments {}", args.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(" ")) };
                format!("Run PowerShell script {} (SHA-256 {}…){a} in {} (stops after {timeout_s} s){}", show(script), &sha256[..12], show(folder), stdin_note(stdin_secret))
            }
            Step::Check { check, wait_s } => {
                let what = match check {
                    Check::FileExists(p) => format!("Check that {} exists", show(p)),
                    Check::Port(p) => format!("Check that port {p} answers on this PC"),
                    Check::Url(u) => format!("Check that {u} returns 200"),
                };
                if *wait_s > 0 { format!("{what} (waits up to {wait_s} s)") } else { what }
            }
            Step::LaunchDetached { command, folder } => format!("Start `{command}` in {} and don't wait (it keeps running after Glim stops)", show(folder)),
        }
    }
}

fn stdin_note(secret: &Option<String>) -> String {
    secret.as_ref().map(|n| format!("; secret {n} goes to its input only")).unwrap_or_default()
}

/// A path as the owner typed it (no `\\?\` prefix).
pub fn show(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

#[derive(Clone, Debug)]
pub struct BookStep {
    pub step: Step,
    pub ask: bool,
}

#[derive(Clone, Debug)]
pub struct Allow {
    pub folders: Vec<PathBuf>,
    pub apps: Vec<App>,
    pub commands: Vec<String>,
    /// Canonical paths of the .ps1 files ps_script may run.
    pub scripts: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Runbook {
    pub name: String,
    pub description: String,
    pub allow: Allow,
    pub steps: Vec<BookStep>,
    /// SHA-256 of the file's bytes, hex.
    pub hash: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A runbook's name (its file's stem): 1-40 letters, digits, `_`, `-`, `.`,
/// starting with a letter or digit.
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// An env key: a letter or `_` first, then letters, digits, `_`.
pub fn env_key_ok(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A full local path: a drive letter, no `..`, no network share, no stream.
fn full_local(raw: &str) -> Result<PathBuf, String> {
    let b = raw.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    if !drive || raw[2..].contains(':') || raw.chars().any(|c| c.is_control() || matches!(c, '"' | '*' | '?' | '<' | '>' | '|')) {
        return Err(format!("{raw:?} is not a full local path (like C:\\folder\\file)"));
    }
    let p = PathBuf::from(raw);
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{raw:?} may not contain .."));
    }
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDriveTypeW;
        let root: Vec<u16> = format!("{}\\", &raw[..2]).encode_utf16().chain(Some(0)).collect();
        // DRIVE_REMOTE is 4: a mapped network drive.
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == 4 {
            return Err(format!("{raw:?} is on a network drive"));
        }
    }
    Ok(p)
}

/// Folders a runbook may never be allowed into.
fn forbidden_folder(canon: &Path) -> bool {
    if canon.parent().is_none() || canon.components().count() <= 2 {
        // A drive root (\\?\C:\ has two components).
        return true;
    }
    let under = |var: &str| std::env::var_os(var).and_then(|v| std::fs::canonicalize(v).ok()).is_some_and(|base| canon.starts_with(&base));
    let is = |var: &str| std::env::var_os(var).and_then(|v| std::fs::canonicalize(v).ok()).is_some_and(|base| canon == base);
    under("WINDIR") || under("ProgramFiles") || under("ProgramFiles(x86)") || under("ProgramData") || is("USERPROFILE")
}

pub(crate) fn allowed_folder(raw: &str) -> Result<PathBuf, String> {
    let p = full_local(raw)?;
    let canon = std::fs::canonicalize(&p).map_err(|_| format!("allowed folder {raw:?} doesn't exist"))?;
    if !canon.is_dir() {
        return Err(format!("allowed folder {raw:?} is not a folder"));
    }
    if forbidden_folder(&canon) {
        return Err(format!("{raw:?} can't be allowed (a drive, Windows, Program Files or your whole profile)"));
    }
    Ok(canon)
}

const BANNED_SCHEMES: &[&str] = &["http", "https", "file", "ftp", "javascript", "vbscript", "data", "ms-msdt", "search-ms", "ms-officecmd", "shell"];

fn app_entry(raw: &str) -> Result<App, String> {
    let lower = raw.to_ascii_lowercase();
    if lower.ends_with(".exe") || lower.ends_with(".lnk") {
        if let Some(problem) = crate::voice::own_apps::path_problem(raw) {
            return Err(format!("app {raw:?} is not a full local path to an .exe or .lnk ({problem})"));
        }
        full_local(raw)?;
        return Ok(App::Path(raw.to_string()));
    }
    let scheme = raw.split(':').next().unwrap_or("");
    let ok = raw.contains(':')
        && scheme.len() >= 2
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && raw.len() <= 300
        && !raw.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"');
    if !ok {
        return Err(format!("app {raw:?} is neither a path to an .exe/.lnk nor a protocol link (like example-app:)"));
    }
    if BANNED_SCHEMES.iter().any(|b| b.eq_ignore_ascii_case(scheme)) {
        return Err(format!("{scheme}: links can't be a runbook app"));
    }
    Ok(App::Protocol(raw.to_string()))
}

/// One command, one line, no shell operators or variables.
fn command_entry(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() || raw.len() > 300 || raw.trim() != raw {
        return Err(format!("command {raw:?} must be one trimmed line of at most 300 characters"));
    }
    if raw.chars().any(|c| c.is_control() || matches!(c, '&' | '|' | '<' | '>' | '^' | '%' | '!' | '\n' | '\r')) {
        return Err(format!("command {raw:?} may not use & | < > ^ % ! (one command, no shell operators or variables)"));
    }
    Ok(raw.to_string())
}

pub const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
pub const MAX_ARGS: usize = 16;

/// An [allow] scripts entry: a full local path to an existing .ps1 file
/// (not a link), as its canonical path.
fn script_entry(raw: &str) -> Result<PathBuf, String> {
    if !raw.to_ascii_lowercase().ends_with(".ps1") {
        return Err(format!("script {raw:?} must be a .ps1 file"));
    }
    let p = full_local(raw)?;
    let meta = std::fs::symlink_metadata(&p).map_err(|_| format!("script {raw:?} doesn't exist"))?;
    if !meta.is_file() {
        return Err(format!("script {raw:?} is a link or a folder"));
    }
    if meta.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script {raw:?} is over 1 MB"));
    }
    std::fs::canonicalize(&p).map_err(|_| format!("script {raw:?} doesn't exist"))
}

/// SHA-256 of a script's bytes, hex.
pub fn script_sha(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| format!("script {} is gone", show(path)))?;
    if !meta.is_file() || meta.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script {} is a link, a folder or over 1 MB", show(path)));
    }
    std::fs::read(path).map(|b| sha256_hex(&b)).map_err(|e| format!("couldn't read script {}: {e}", show(path)))
}

/// A ps_script's fixed arguments: at most 16, each one line of at most 200
/// characters, no quotes.
fn script_args(raw: &[String]) -> Result<Vec<String>, String> {
    if raw.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} args"));
    }
    for a in raw {
        if a.is_empty() || a.len() > 200 || a.chars().any(|c| c.is_control() || c == '"') {
            return Err(format!("arg {a:?} must be 1-200 characters on one line, without \""));
        }
    }
    Ok(raw.to_vec())
}

fn secret_name(raw: &Option<String>, what: &str) -> Result<Option<String>, String> {
    match raw {
        None => Ok(None),
        Some(n) => {
            secrets::check_name(n).map_err(|_| format!("{what} {n:?} is not a usable secret name"))?;
            Ok(Some(n.clone()))
        }
    }
}

/// env_secret's require_prefix: 1-8 prefixes, each 1-32 printable characters.
fn prefixes(raw: &Option<Vec<String>>) -> Result<Vec<String>, String> {
    let Some(list) = raw else { return Ok(vec![]) };
    if list.is_empty() || list.len() > 8 {
        return Err("require_prefix needs 1 to 8 prefixes".into());
    }
    for x in list {
        if x.is_empty() || x.len() > 32 || x.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(format!("prefix {x:?} must be 1-32 characters without spaces"));
        }
    }
    Ok(list.clone())
}

fn inside(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| path.starts_with(f))
}

/// The file an env_secret step writes: its folder must exist and be inside
/// an allowed folder; the file itself may not be a link or a folder.
fn env_target(raw: &str, folders: &[PathBuf]) -> Result<PathBuf, String> {
    let p = full_local(raw)?;
    let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
        return Err(format!("{raw:?} is not a file path"));
    };
    let parent = std::fs::canonicalize(parent).map_err(|_| format!("the folder of {raw:?} doesn't exist"))?;
    if !inside(&parent, folders) {
        return Err(format!("{raw:?} is outside the allowed folders"));
    }
    let file = parent.join(name);
    if let Ok(m) = std::fs::symlink_metadata(&file) {
        if m.file_type().is_symlink() || m.is_dir() {
            return Err(format!("{raw:?} is a link or a folder"));
        }
    }
    Ok(file)
}

fn none_of(s: &RawStep, fields: &[(&str, bool)]) -> Result<(), String> {
    match fields.iter().find(|(_, set)| *set) {
        Some((f, _)) => Err(format!("a {} step can't have {f}", s.kind)),
        None => Ok(()),
    }
}

fn step(s: &RawStep, allow: &Allow) -> Result<Step, String> {
    let need = |v: &Option<String>, f: &str| v.clone().filter(|x| !x.trim().is_empty()).ok_or_else(|| format!("a {} step needs {f}", s.kind));
    match s.kind.as_str() {
        "env_secret" => {
            none_of(s, &[("app", s.app.is_some()), ("command", s.command.is_some()), ("folder", s.folder.is_some()), ("timeout_s", s.timeout_s.is_some()), ("file_exists", s.file_exists.is_some()), ("port", s.port.is_some()), ("url", s.url.is_some()), ("wait_s", s.wait_s.is_some()), ("script", s.script.is_some()), ("args", s.args.is_some()), ("secret_stdin", s.secret_stdin.is_some())])?;
            let secret = need(&s.secret, "secret")?;
            secrets::check_name(&secret).map_err(|_| format!("secret {secret:?} is not a usable secret name"))?;
            let key = need(&s.key, "key")?;
            if !env_key_ok(&key) {
                return Err(format!("key {key:?} must be letters, digits and _ (not starting with a digit)"));
            }
            let file = env_target(&need(&s.file, "file")?, &allow.folders)?;
            let require_prefix = prefixes(&s.require_prefix)?;
            Ok(Step::EnvSecret { secret, file, key, require_prefix })
        }
        "start_app" => {
            none_of(s, &[("secret", s.secret.is_some()), ("file", s.file.is_some()), ("key", s.key.is_some()), ("command", s.command.is_some()), ("folder", s.folder.is_some()), ("timeout_s", s.timeout_s.is_some()), ("file_exists", s.file_exists.is_some()), ("port", s.port.is_some()), ("url", s.url.is_some()), ("wait_s", s.wait_s.is_some()), ("script", s.script.is_some()), ("args", s.args.is_some()), ("secret_stdin", s.secret_stdin.is_some()), ("require_prefix", s.require_prefix.is_some())])?;
            let raw = need(&s.app, "app")?;
            let app = app_entry(&raw)?;
            let listed = allow.apps.iter().any(|a| match (a, &app) {
                (App::Path(x), App::Path(y)) => x.eq_ignore_ascii_case(y),
                (App::Protocol(x), App::Protocol(y)) => x == y,
                _ => false,
            });
            if !listed {
                return Err(format!("app {raw:?} is not in [allow] apps"));
            }
            Ok(Step::StartApp { app })
        }
        "run" | "launch_detached" => {
            none_of(s, &[("secret", s.secret.is_some()), ("file", s.file.is_some()), ("key", s.key.is_some()), ("app", s.app.is_some()), ("file_exists", s.file_exists.is_some()), ("port", s.port.is_some()), ("url", s.url.is_some()), ("wait_s", s.wait_s.is_some()), ("script", s.script.is_some()), ("args", s.args.is_some()), ("require_prefix", s.require_prefix.is_some())])?;
            let command = command_entry(&need(&s.command, "command")?)?;
            if !allow.commands.contains(&command) {
                return Err(format!("command {command:?} is not exactly in [allow] commands"));
            }
            let raw = need(&s.folder, "folder")?;
            let folder = std::fs::canonicalize(full_local(&raw)?).map_err(|_| format!("folder {raw:?} doesn't exist"))?;
            if !folder.is_dir() || !inside(&folder, &allow.folders) {
                return Err(format!("folder {raw:?} is not in or under [allow] folders"));
            }
            if s.kind == "launch_detached" {
                if s.timeout_s.is_some() {
                    return Err("a launch_detached step can't have timeout_s (Glim doesn't wait for it)".into());
                }
                if s.secret_stdin.is_some() {
                    return Err("a launch_detached step can't have secret_stdin".into());
                }
                return Ok(Step::LaunchDetached { command, folder });
            }
            let timeout_s = s.timeout_s.unwrap_or(DEFAULT_TIMEOUT_S);
            if !(1..=MAX_TIMEOUT_S).contains(&timeout_s) {
                return Err(format!("timeout_s must be 1 to {MAX_TIMEOUT_S}"));
            }
            let stdin_secret = secret_name(&s.secret_stdin, "secret_stdin")?;
            Ok(Step::Run { command, folder, timeout_s, stdin_secret })
        }
        "ps_script" => {
            none_of(s, &[("secret", s.secret.is_some()), ("file", s.file.is_some()), ("key", s.key.is_some()), ("app", s.app.is_some()), ("command", s.command.is_some()), ("file_exists", s.file_exists.is_some()), ("port", s.port.is_some()), ("url", s.url.is_some()), ("wait_s", s.wait_s.is_some()), ("require_prefix", s.require_prefix.is_some())])?;
            let raw = need(&s.script, "script")?;
            let script = script_entry(&raw)?;
            if !allow.scripts.iter().any(|a| a == &script) {
                return Err(format!("script {raw:?} is not in [allow] scripts"));
            }
            let sha256 = script_sha(&script)?;
            let args = script_args(s.args.as_deref().unwrap_or(&[]))?;
            let raw_folder = need(&s.folder, "folder")?;
            let folder = std::fs::canonicalize(full_local(&raw_folder)?).map_err(|_| format!("folder {raw_folder:?} doesn't exist"))?;
            if !folder.is_dir() || !inside(&folder, &allow.folders) {
                return Err(format!("folder {raw_folder:?} is not in or under [allow] folders"));
            }
            let timeout_s = s.timeout_s.unwrap_or(DEFAULT_TIMEOUT_S);
            if !(1..=MAX_TIMEOUT_S).contains(&timeout_s) {
                return Err(format!("timeout_s must be 1 to {MAX_TIMEOUT_S}"));
            }
            let stdin_secret = secret_name(&s.secret_stdin, "secret_stdin")?;
            Ok(Step::PsScript { script, sha256, args, folder, timeout_s, stdin_secret })
        }
        "check" => {
            none_of(s, &[("secret", s.secret.is_some()), ("file", s.file.is_some()), ("key", s.key.is_some()), ("app", s.app.is_some()), ("command", s.command.is_some()), ("folder", s.folder.is_some()), ("timeout_s", s.timeout_s.is_some()), ("script", s.script.is_some()), ("args", s.args.is_some()), ("secret_stdin", s.secret_stdin.is_some()), ("require_prefix", s.require_prefix.is_some())])?;
            let set = [s.file_exists.is_some(), s.port.is_some(), s.url.is_some()].iter().filter(|x| **x).count();
            if set != 1 {
                return Err("a check step needs exactly one of file_exists, port, url".into());
            }
            let check = if let Some(f) = &s.file_exists {
                Check::FileExists(full_local(f)?)
            } else if let Some(p) = s.port {
                if p == 0 {
                    return Err("port must be 1 to 65535".into());
                }
                Check::Port(p)
            } else {
                let raw = s.url.clone().unwrap_or_default();
                let url = crate::net::Url::parse(&raw).map_err(|_| format!("url {raw:?} is not a URL"))?;
                if !matches!(url.scheme(), "http" | "https") || !crate::net::is_loopback_url(&url) {
                    return Err(format!("url {raw:?} must be http(s) on this PC (127.0.0.1 or localhost)"));
                }
                Check::Url(raw)
            };
            let wait_s = s.wait_s.unwrap_or(0);
            if wait_s > MAX_WAIT_S {
                return Err(format!("wait_s must be 0 to {MAX_WAIT_S}"));
            }
            Ok(Step::Check { check, wait_s })
        }
        "" => Err("a step needs a type".into()),
        other => Err(format!("unknown step type {other:?} (only env_secret, start_app, run, ps_script, check, launch_detached)")),
    }
}

/// Reads and checks a runbook from its text. Every problem is one English
/// sentence, naming the step.
pub fn parse(name: &str, bytes: &[u8]) -> Result<Runbook, String> {
    if !name_ok(name) {
        return Err(format!("the file name {name:?} must be 1-{MAX_NAME} letters, digits, - _ or . (no spaces)"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text".to_string())?;
    let raw: RawBook = toml::from_str(text).map_err(|e| format!("not valid runbook TOML: {}", e.message()))?;
    if raw.step.is_empty() {
        return Err("the runbook has no [[step]]".into());
    }
    if raw.step.len() > MAX_STEPS {
        return Err(format!("at most {MAX_STEPS} steps"));
    }
    let folders = raw.allow.folders.iter().map(|f| allowed_folder(f)).collect::<Result<Vec<_>, _>>()?;
    let apps = raw.allow.apps.iter().map(|a| app_entry(a)).collect::<Result<Vec<_>, _>>()?;
    let commands = raw.allow.commands.iter().map(|c| command_entry(c)).collect::<Result<Vec<_>, _>>()?;
    let scripts = raw.allow.scripts.iter().map(|c| script_entry(c)).collect::<Result<Vec<_>, _>>()?;
    let allow = Allow { folders, apps, commands, scripts };
    let mut steps = Vec::new();
    for (i, s) in raw.step.iter().enumerate() {
        let st = step(s, &allow).map_err(|e| format!("step {}: {e}", i + 1))?;
        if matches!(st, Step::LaunchDetached { .. }) && i + 1 != raw.step.len() {
            return Err(format!("step {}: a launch_detached step must be the last step (Glim doesn't wait for it)", i + 1));
        }
        steps.push(BookStep { step: st, ask: s.ask });
    }
    let hash = book_hash(bytes, &steps);
    Ok(Runbook { name: name.to_string(), description: raw.description.chars().take(200).collect(), allow, steps, hash })
}

/// What approval covers: the file's bytes, plus each ps_script's path and
/// SHA-256 (a runbook without scripts keeps the plain file hash, so its
/// approval stays valid).
pub fn book_hash(bytes: &[u8], steps: &[BookStep]) -> String {
    let scripts: Vec<String> = steps
        .iter()
        .filter_map(|s| match &s.step {
            Step::PsScript { script, sha256, .. } => Some(format!("script {} {sha256}\n", show(script))),
            _ => None,
        })
        .collect();
    if scripts.is_empty() {
        return sha256_hex(bytes);
    }
    let mut all = bytes.to_vec();
    all.extend_from_slice(b"\n--glim-scripts--\n");
    for x in scripts {
        all.extend_from_slice(x.as_bytes());
    }
    sha256_hex(&all)
}

/// The runbook files there are, by name (file stem), sorted.
pub fn names_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) && p.is_file())
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out
}

/// Reads `<dir>\<name>.toml`: its bytes, then the checked runbook.
pub fn load_from(dir: &Path, name: &str) -> Result<(Vec<u8>, Result<Runbook, String>), String> {
    if !name_ok(name) {
        return Err(format!("no runbook called {name:?}"));
    }
    let path = dir.join(format!("{name}.toml"));
    let meta = std::fs::symlink_metadata(&path).map_err(|_| format!("no runbook called {name:?}"))?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(format!("{name}.toml is not a runbook file (a link, a folder or over 64 KB)"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("couldn't read {name}.toml: {e}"))?;
    let book = parse(name, &bytes);
    Ok((bytes, book))
}

// ── Approval ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub hash: String,
    pub approved_ms: u64,
    /// HMAC-SHA256(key, name "\n" hash), hex; the key is in Credential Manager.
    pub mac: String,
}

pub type Approvals = BTreeMap<String, Approval>;

/// HMAC-SHA256 (RFC 2104), with the sha2 Glim already builds.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

fn mac(key: &[u8], name: &str, hash: &str) -> String {
    hmac_sha256(key, format!("{name}\n{hash}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex32() -> Result<String, String> {
    let mut buf = [0u8; 32];
    #[cfg(windows)]
    {
        use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
        let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if status.is_err() {
            return Err("Windows couldn't make a random key".into());
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).map_err(|e| e.to_string())?;
    }
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// The approvals' key: read, or made once.
pub fn approval_key(vault: &dyn Vault, create: bool) -> Result<Option<Vec<u8>>, String> {
    match vault.read()? {
        Some(k) if k.len() == 64 => Ok(Some(k.into_bytes())),
        _ if create => {
            let k = random_hex32()?;
            vault.write(&k)?;
            Ok(Some(k.into_bytes()))
        }
        _ => Ok(None),
    }
}

pub fn approval_vault() -> impl Vault {
    crate::ai::key::CredentialManager { target: APPROVAL_KEY_TARGET }
}

pub fn read_approvals(path: &Path) -> Approvals {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_approvals(path: &Path, a: &Approvals) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(a).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Approved, and the file is the one approved.
    Approved,
    /// Never approved.
    New,
    /// Approved once, but the file (or its approval) changed since.
    Changed,
    /// The file can't be used.
    Invalid,
}

pub fn state(book: &Runbook, approvals: &Approvals, key: Option<&[u8]>) -> State {
    match (approvals.get(&book.name), key) {
        (None, _) => State::New,
        (Some(a), Some(k)) if a.hash == book.hash && a.mac == mac(k, &book.name, &book.hash) => State::Approved,
        _ => State::Changed,
    }
}

/// Approves the runbook as it is now, only if it is still the version the
/// owner was shown (`seen_hash`).
pub fn approve(book: &Runbook, seen_hash: &str, path: &Path, vault: &dyn Vault, now_ms: u64) -> Result<(), String> {
    if book.hash != seen_hash {
        return Err("the file changed while you were reading it: look at it again".into());
    }
    let key = approval_key(vault, true)?.ok_or("no approval key")?;
    let mut all = read_approvals(path);
    all.insert(book.name.clone(), Approval { hash: book.hash.clone(), approved_ms: now_ms, mac: mac(&key, &book.name, &book.hash) });
    write_approvals(path, &all)
}

// ── Running ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Settings,
    Voice,
    Cli,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::Settings => "settings",
            Trigger::Voice => "voice",
            Trigger::Cli => "cli",
        }
    }
}

/// A step's result: never a value or a command's output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// 1-based.
    pub step: usize,
    pub total: usize,
    pub kind: &'static str,
    pub ok: bool,
    /// What happened, in a few English words ("exit code 0", "the file is tracked by git").
    pub detail: String,
}

/// Why a run didn't start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotFound(String),
    Invalid(String),
    NotApproved,
    Changed,
    /// Steps with ask = true that weren't confirmed for this run (1-based).
    Asks(Vec<usize>),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::NotFound(e) | Refusal::Invalid(e) => e.clone(),
            Refusal::NotApproved => "not approved yet: approve it in Glim (Settings → Runbooks → Run) first".into(),
            Refusal::Changed => "the file changed since it was approved: approve it again in Glim (Settings → Runbooks)".into(),
            Refusal::Asks(s) => format!(
                "step {} asks every run: run it from Glim (Settings → Runbooks, or say \"run <name>\")",
                s.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Refusal::NotFound(_) => "not-found",
            Refusal::Invalid(_) => "invalid",
            Refusal::NotApproved => "not-approved",
            Refusal::Changed => "changed",
            Refusal::Asks(_) => "asks",
        }
    }
}

/// What the runner needs from the outside world: real in the app, fakes in tests.
pub trait World {
    fn secret(&self, name: &str) -> Result<Option<secrets::SecretValue>, String>;
    /// Where a runbook's env_secret backups go (outside every repository).
    fn backups(&self, runbook: &str) -> PathBuf;
    fn start_app(&self, app: &App) -> bool;
    /// Exit code, or Err("timeout" / "couldn't start").
    fn run(&self, command: &str, folder: &Path, timeout: Duration, input: Option<&[u8]>) -> Result<i32, String>;
    /// Exit code and output (for redact + save_output only, never a log).
    fn ps_script(&self, script: &Path, args: &[String], folder: &Path, timeout: Duration, input: Option<&[u8]>) -> Result<(i32, Vec<u8>), String>;
    /// Keeps a ps_script's last output (already redacted) for Settings.
    fn save_output(&self, runbook: &str, text: &str);
    /// Starts it in a console of its own, outside Glim's lifetime; doesn't wait.
    fn launch_detached(&self, command: &str, folder: &Path) -> Result<(), String>;
    fn port_answers(&self, port: u16) -> bool;
    fn url_status(&self, url: &str) -> Result<u16, String>;
    fn sleep(&self, d: Duration);
}

/// Runs a checked, approved runbook's steps in order, stopping at the
/// first failure. `confirmed` = the ask steps the owner said yes to now.
pub fn run_steps(book: &Runbook, world: &dyn World, confirmed: &[usize], progress: &mut dyn FnMut(&Outcome)) -> Result<Vec<Outcome>, Refusal> {
    let unconfirmed: Vec<usize> = book.steps.iter().enumerate().filter(|(i, s)| s.ask && !confirmed.contains(&(i + 1))).map(|(i, _)| i + 1).collect();
    if !unconfirmed.is_empty() {
        return Err(Refusal::Asks(unconfirmed));
    }
    let total = book.steps.len();
    let mut out = Vec::new();
    for (i, s) in book.steps.iter().enumerate() {
        let (ok, detail) = match one(&book.name, i + 1, total, &s.step, world) {
            Ok(d) => (true, d),
            Err(d) => (false, d),
        };
        let o = Outcome { step: i + 1, total, kind: s.step.kind(), ok, detail };
        progress(&o);
        out.push(o);
        if !ok {
            break;
        }
    }
    Ok(out)
}

fn needed_secret(world: &dyn World, name: &str) -> Result<secrets::SecretValue, String> {
    world.secret(name)?.ok_or_else(|| format!("no secret called {name} (Settings → Secrets → Add a secret)"))
}

/// The output Settings may show: text, the secret (if one went to stdin)
/// blanked out, no control characters but new lines and tabs, the last
/// OUTPUT_SHOWN characters.
pub const OUTPUT_SHOWN: usize = 4000;

pub fn redact(output: &[u8], secret: Option<&str>) -> String {
    let mut text = String::from_utf8_lossy(output).replace("\r\n", "\n");
    if let Some(v) = secret.filter(|v| !v.is_empty()) {
        text = text.replace(v, "[secret]");
    }
    let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    let n = clean.chars().count();
    if n <= OUTPUT_SHOWN {
        clean
    } else {
        format!("…{}", clean.chars().skip(n - OUTPUT_SHOWN).collect::<String>())
    }
}

fn one(runbook: &str, index: usize, total: usize, step: &Step, world: &dyn World) -> Result<String, String> {
    match step {
        Step::EnvSecret { secret, file, key, require_prefix } => {
            let value = needed_secret(world, secret)?;
            if !require_prefix.is_empty() && !require_prefix.iter().any(|p| value.reveal().starts_with(p.as_str())) {
                return Err(format!("secret {secret} doesn't start with an allowed prefix: nothing written"));
            }
            env_file::write_secret(file, key, &value, &world.backups(runbook)).map(|()| "written".to_string())
        }
        Step::StartApp { app } => {
            if world.start_app(app) { Ok("started".into()) } else { Err("Windows couldn't start it".into()) }
        }
        Step::Run { command, folder, timeout_s, stdin_secret } => {
            let value = match stdin_secret {
                Some(n) => Some(needed_secret(world, n)?),
                None => None,
            };
            match world.run(command, folder, Duration::from_secs(*timeout_s), value.as_ref().map(|v| v.reveal().as_bytes())) {
                Ok(0) => Ok("exit code 0".into()),
                Ok(code) => Err(format!("exit code {code}")),
                Err(e) => Err(e),
            }
        }
        Step::PsScript { script, sha256, args, folder, timeout_s, stdin_secret } => {
            // Hashed again just before it runs: approval covered these bytes.
            if script_sha(script)? != *sha256 {
                return Err("the script changed since the runbook was approved: approve it again".into());
            }
            let value = match stdin_secret {
                Some(n) => Some(needed_secret(world, n)?),
                None => None,
            };
            let r = world.ps_script(script, args, folder, Duration::from_secs(*timeout_s), value.as_ref().map(|v| v.reveal().as_bytes()));
            let secret = value.as_ref().map(|v| v.reveal());
            match r {
                Ok((code, out)) => {
                    world.save_output(runbook, &format!("step {index}/{total} exit code {code}\n{}", redact(&out, secret)));
                    if code == 0 { Ok("exit code 0".into()) } else { Err(format!("exit code {code}")) }
                }
                Err(e) => {
                    world.save_output(runbook, &format!("step {index}/{total}: {e}"));
                    Err(e)
                }
            }
        }
        Step::LaunchDetached { command, folder } => world.launch_detached(command, folder).map(|()| "started, not waited for".to_string()),
        Step::Check { check, wait_s } => {
            let until = Instant::now() + Duration::from_secs(*wait_s);
            loop {
                let r = match check {
                    Check::FileExists(p) => if p.exists() { Ok("exists".to_string()) } else { Err("not there".to_string()) },
                    Check::Port(p) => if world.port_answers(*p) { Ok("answers".into()) } else { Err(format!("nothing answers on port {p}")) },
                    Check::Url(u) => match world.url_status(u) {
                        Ok(200) => Ok("200".into()),
                        Ok(s) => Err(format!("status {s}")),
                        Err(e) => Err(e),
                    },
                };
                if r.is_ok() || Instant::now() >= until {
                    return r;
                }
                world.sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Everything about one run of a named runbook: load, check approval, run,
/// audit. `seen_hash`: Settings passes the version it showed; a different
/// file is refused as changed.
#[allow(clippy::too_many_arguments)]
pub fn run_named(
    dir: &Path,
    approvals: &Approvals,
    key: Option<&[u8]>,
    name: &str,
    seen_hash: Option<&str>,
    trigger: Trigger,
    confirmed: &[usize],
    world: &dyn World,
    audit: &mut dyn FnMut(String),
    progress: &mut dyn FnMut(&Outcome),
) -> Result<Vec<Outcome>, Refusal> {
    let refused = |r: Refusal, audit: &mut dyn FnMut(String)| {
        let shown = if name_ok(name) { name } else { "?" };
        audit(format!("{shown} {} refused {}", trigger.as_str(), r.code()));
        Err(r)
    };
    let (_, book) = match load_from(dir, name) {
        Ok(x) => x,
        Err(e) => return refused(Refusal::NotFound(e), audit),
    };
    let book = match book {
        Ok(b) => b,
        Err(e) => return refused(Refusal::Invalid(e), audit),
    };
    match state(&book, approvals, key) {
        State::Approved => {}
        State::New => return refused(Refusal::NotApproved, audit),
        _ => return refused(Refusal::Changed, audit),
    }
    if seen_hash.is_some_and(|h| h != book.hash) {
        return refused(Refusal::Changed, audit);
    }
    let t = trigger.as_str();
    audit(format!("{name} {t} start {} steps", book.steps.len()));
    let result = run_steps(&book, world, confirmed, &mut |o| {
        audit(format!("{name} {t} step {}/{} {} {} ({})", o.step, o.total, o.kind, if o.ok { "OK" } else { "FAIL" }, o.detail));
        progress(o);
    });
    match &result {
        Ok(outcomes) => {
            let done = outcomes.iter().filter(|o| o.ok).count();
            let all = done == book.steps.len();
            audit(format!("{name} {t} end {} {done}/{}", if all { "OK" } else { "FAIL" }, book.steps.len()));
        }
        Err(r) => audit(format!("{name} {t} refused {}", r.code())),
    }
    result
}

// ── The audit log ─────────────────────────────────────────────────────────────

/// One line in runbooks.log, with the time. Never a value or output: the
/// callers only pass names, kinds, outcomes and fixed English details.
pub fn audit_line(path: &Path, line: &str) {
    use std::io::Write;
    if let Some(d) = path.parent() {
        let _ = crate::platform::ensure_private_dir(d);
    }
    crate::log::rotate(path, crate::log::MAX_BYTES);
    let t = crate::platform::local_time();
    let stamp = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{stamp} {line}");
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    /// "2026-11-12 14:05:00"
    pub time: String,
    pub name: String,
    pub trigger: String,
    /// "OK", "FAIL" or "refused <code>".
    pub outcome: String,
    pub ok: bool,
    pub refused: bool,
    pub done: usize,
    pub total: usize,
}

/// The last `n` runs (ends and refusals), newest first.
pub fn history(text: &str, n: usize) -> Vec<RunView> {
    let mut out = Vec::new();
    for line in text.lines().rev() {
        let w: Vec<&str> = line.split(' ').collect();
        // date time name trigger end OK 2/2  |  date time name trigger refused code
        if w.len() >= 6 && w[4] == "end" {
            let (done, total) = w.get(6).and_then(|f| f.split_once('/')).map(|(a, b)| (a.parse().unwrap_or(0), b.parse().unwrap_or(0))).unwrap_or((0, 0));
            out.push(RunView { time: format!("{} {}", w[0], w[1]), name: w[2].into(), trigger: w[3].into(), outcome: w[5].into(), ok: w[5] == "OK", refused: false, done, total });
        } else if w.len() >= 6 && w[4] == "refused" {
            out.push(RunView { time: format!("{} {}", w[0], w[1]), name: w[2].into(), trigger: w[3].into(), outcome: format!("refused {}", w[5]), ok: false, refused: true, done: 0, total: 0 });
        }
        if out.len() == n {
            break;
        }
    }
    out
}

// ── The real world ────────────────────────────────────────────────────────────

pub struct RealWorld;

impl World for RealWorld {
    fn secret(&self, name: &str) -> Result<Option<secrets::SecretValue>, String> {
        // runbooks: the only place a secret is read out (secrets.rs test).
        secrets::read(&secrets::store(), name)
    }

    fn backups(&self, runbook: &str) -> PathBuf {
        backups_dir().join(runbook)
    }

    fn start_app(&self, app: &App) -> bool {
        match app {
            App::Path(p) => crate::platform::open_own_app(p),
            App::Protocol(link) => crate::platform::open_protocol(link),
        }
    }

    fn run(&self, command: &str, folder: &Path, timeout: Duration, input: Option<&[u8]>) -> Result<i32, String> {
        crate::platform::run_command_line(command, folder, timeout, input)
    }

    fn ps_script(&self, script: &Path, args: &[String], folder: &Path, timeout: Duration, input: Option<&[u8]>) -> Result<(i32, Vec<u8>), String> {
        crate::platform::run_ps_script(script, args, folder, timeout, input)
    }

    fn save_output(&self, runbook: &str, text: &str) {
        let d = outputs_dir();
        let _ = std::fs::create_dir_all(&d);
        let _ = crate::platform::ensure_private_dir(&d);
        let t = crate::platform::local_time();
        let stamp = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
        let _ = std::fs::write(d.join(format!("{runbook}.txt")), format!("{stamp} {text}"));
    }

    fn launch_detached(&self, command: &str, folder: &Path) -> Result<(), String> {
        crate::platform::launch_detached(command, folder)
    }

    fn port_answers(&self, port: u16) -> bool {
        crate::net::loopback_port_answers(port, Duration::from_secs(2))
    }

    fn url_status(&self, url: &str) -> Result<u16, String> {
        crate::net::loopback_status(url, Duration::from_secs(10))
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Today's runbooks and approvals, read for one call.
pub struct Here {
    pub dir: PathBuf,
    pub approvals: Approvals,
    pub key: Option<Vec<u8>>,
}

impl Here {
    pub fn read() -> Here {
        Here { dir: dir(), approvals: read_approvals(&approvals_path()), key: approval_key(&approval_vault(), false).ok().flatten() }
    }
}

/// Runs a named runbook for real, auditing to runbooks.log.
pub fn run_now(name: &str, seen_hash: Option<&str>, trigger: Trigger, confirmed: &[usize], progress: &mut dyn FnMut(&Outcome)) -> Result<Vec<Outcome>, Refusal> {
    let here = Here::read();
    let audit = audit_path();
    run_named(&here.dir, &here.approvals, here.key.as_deref(), name, seen_hash, trigger, confirmed, &RealWorld, &mut |l| audit_line(&audit, &l), progress)
}

/// Settings → Runbooks → Approve runbook.
pub fn approve_now(name: &str, seen_hash: &str) -> Result<(), String> {
    let (_, book) = load_from(&dir(), name)?;
    let book = book?;
    let r = approve(&book, seen_hash, &approvals_path(), &approval_vault(), crate::learn::now() * 1000);
    if r.is_ok() {
        audit_line(&audit_path(), &format!("{name} settings approved"));
    }
    r
}

// ── What Settings sees ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    pub kind: &'static str,
    /// The English one-liner (names, paths, commands; never a value).
    pub text: String,
    pub ask: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookView {
    pub name: String,
    pub description: String,
    pub state: State,
    /// Why it can't be used (State::Invalid).
    pub error: String,
    pub hash: String,
    pub steps: Vec<StepView>,
    pub folders: Vec<String>,
    pub apps: Vec<String>,
    pub commands: Vec<String>,
    pub scripts: Vec<String>,
    /// The last ps_script output (redacted, truncated), "" for none.
    pub last_output: String,
}

fn last_output(name: &str) -> String {
    let p = outputs_dir().join(format!("{name}.txt"));
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_file() && m.len() <= 64 * 1024 => std::fs::read_to_string(&p).unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn view(name: &str, here: &Here) -> BookView {
    let empty = |state, error: String| BookView { name: name.into(), description: String::new(), state, error, hash: String::new(), steps: vec![], folders: vec![], apps: vec![], commands: vec![], scripts: vec![], last_output: String::new() };
    match load_from(&here.dir, name) {
        Err(e) => empty(State::Invalid, e),
        Ok((_, Err(e))) => empty(State::Invalid, e),
        Ok((_, Ok(b))) => BookView {
            name: b.name.clone(),
            description: b.description.clone(),
            state: state(&b, &here.approvals, here.key.as_deref()),
            error: String::new(),
            hash: b.hash.clone(),
            steps: b.steps.iter().map(|s| StepView { kind: s.step.kind(), text: s.step.describe(), ask: s.ask }).collect(),
            folders: b.allow.folders.iter().map(|f| show(f)).collect(),
            apps: b.allow.apps.iter().map(|a| match a { App::Path(p) | App::Protocol(p) => p.clone() }).collect(),
            commands: b.allow.commands.clone(),
            scripts: b.allow.scripts.iter().map(|x| show(x)).collect(),
            last_output: last_output(&b.name),
        },
    }
}

pub fn views() -> Vec<BookView> {
    let here = Here::read();
    names_in(&here.dir).iter().map(|n| view(n, &here)).collect()
}

pub fn recent() -> Vec<RunView> {
    history(&std::fs::read_to_string(audit_path()).unwrap_or_default(), HISTORY)
}

// ── Voice ─────────────────────────────────────────────────────────────────────

/// Letters and digits only, lower case: "Dale-Booking" ~ "dale booking".
fn fold(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// "run example", "please run the example runbook" → the runbook it names,
/// matched on this PC among `names` (its words in any order, so "run site
/// example" also finds example-site). None when it names none of them.
pub fn voice_match(transcript: &str, names: &[String]) -> Option<String> {
    let lower = transcript.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.').filter(|w| !w.is_empty()).collect();
    let filler = ["um", "uh", "please", "glim", "hey", "can", "you", "could", "would", "now"];
    let mut rest: Vec<&str> = words.iter().copied().skip_while(|w| filler.contains(w)).collect();
    if rest.first() != Some(&"run") {
        return None;
    }
    rest.remove(0);
    if rest.first() == Some(&"the") || rest.first() == Some(&"my") {
        rest.remove(0);
    }
    while rest.last().is_some_and(|w| matches!(*w, "runbook" | "please" | "now")) {
        rest.pop();
    }
    if rest.is_empty() {
        return None;
    }
    let said = fold(&rest.join(" "));
    // The same words in any order too: "run update glim" ~ glim-update.
    let words_of = |s: &str| {
        let mut w: Vec<String> = s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_ascii_lowercase()).collect();
        w.sort();
        w
    };
    let said_words = words_of(&rest.join(" "));
    names.iter().find(|n| fold(n) == said).or_else(|| names.iter().find(|n| words_of(n).len() > 1 && words_of(n) == said_words)).cloned()
}

/// A runbook asked for by voice, for the Settings page to pick up if it
/// wasn't listening yet.
pub static REQUESTED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
mod tests;
