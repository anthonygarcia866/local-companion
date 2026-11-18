// What the five remote commands do on this PC (remote courier Phase 1,
// owner 2026-11-14). Each runs only after `Gate::accept` passed it; each
// answers with a short detail or a fixed error code, which courier.rs signs.
//
//   session-status   per allowed repo: is a Claude Code session active (its
//                    transcript written in the last minute), when it last
//                    did something, its short id. Read only.
//   transcript-tail  the last N user/Claude messages of the repo's latest
//                    transcript (~/.claude*/projects/<folder>/*.jsonl), each
//                    cut to 400 characters, tool output left out, run through
//                    the runbook redactor; 12 KB at most. Read only.
//   start-session    with a prompt: `claude -p` in the repo folder, the
//                    prompt on stdin (never the command line), no window;
//                    Claude Code's own permission rules apply, so anything
//                    needing approval is refused in that mode. Without one:
//                    the keep-alive's fixed remote-control command in a
//                    console, to pick up from the Claude app.
//   send-to-session  `claude -p --resume <session>` where that session works
//                    (the repo folder or one of its worktrees), the text on
//                    stdin. The session must be named and be this account's
//                    (owner 2026-11-17): "no-such-session" / "not-your-
//                    session" otherwise; "busy" while it wrote in the last
//                    30 s.
//   runbook          an approved, unchanged, non-asking runbook, exactly as
//                    `glim.exe --run` runs it (Trigger::Remote in runbooks.log).
//   keepalive-restart stops and starts again the "Keep Claude Code reachable"
//                    sessions (keepalive.rs) for folders already on its list,
//                    with their fixed command; nothing when keep-alive is off.
//
// A repo's folder: a keep-alive folder (Settings → Remote) with that name,
// else %USERPROFILE%\<repo> holding a .git folder; never one runbooks would
// refuse (a drive, Windows, your whole profile).
//
// Accounts (owner 2026-11-15, accounts.rs): each task comes from a cloud
// client, and each client acts for one Claude account (account-1 → Claude,
// account-2 → Claude2). session-status, transcript-tail and send-to-session
// see only that account's sessions (the transcripts in its own config
// folder, its worktrees' included); start-session and send-to-session run
// with its CLAUDE_CONFIG_DIR. A client with no account, or a repo whose
// sessions are only another account's, gets "not-your-session".

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::{Command, REPOS};

/// What a task answers: a detail, or a fixed error code.
pub trait Exec: Send + Sync {
    /// `client`: the cloud client that sent the task (remote::Accepted).
    fn run(&self, c: &Command, client: &str) -> Result<String, String>;
}

pub const MAX_DETAIL_CHARS: usize = 12_000;
const MSG_CHARS: usize = 400;
const ACTIVE_S: u64 = 60;
const BUSY_S: u64 = 30;
const TAIL_BYTES: u64 = 512 * 1024;

/// The Claude Code transcript folder name for a project folder: every
/// character but ASCII letters and digits becomes '-'
/// (`C:\Users\sam\local-companion` → `C--Users-sam-local-companion`).
/// A canonicalized path's verbatim prefix is dropped first (2026-11-14:
/// `runbooks::allowed_folder` canonicalizes, and `\\?\C:\…` made
/// `----C--…`, so session-status found no session at all).
pub fn project_slug(folder: &Path) -> String {
    let raw = folder.to_string_lossy();
    let plain = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    };
    plain.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The project folders under `dir`/projects named `slug`, ignoring ASCII
/// case (Windows paths are; a session started from `c:\…` has `c--…`), and
/// those of its worktrees (`<folder>\.claude\worktrees\<name>`, where
/// remote control's `--spawn worktree` puts each session).
fn project_dirs(dir: &Path, slug: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir.join("projects")) else { return Vec::new() };
    let worktrees = format!("{slug}--claude-worktrees-").to_ascii_lowercase();
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n == slug.to_ascii_lowercase() || n.starts_with(&worktrees)
        })
        .map(|e| e.path())
        .collect()
}

/// Session `id`'s transcript (`<id>.jsonl`) among `config_dirs`' projects
/// for `folder` and its worktrees. `id` is a checked UUID (command.rs), so
/// it can't name anything else.
pub fn session_transcript(config_dirs: &[PathBuf], folder: &Path, id: &str) -> Option<PathBuf> {
    if !super::command::is_session_id(id) {
        return None;
    }
    let slug = project_slug(folder);
    config_dirs.iter().flat_map(|dir| project_dirs(dir, &slug)).map(|p| p.join(format!("{id}.jsonl"))).find(|p| p.is_file())
}

/// The newest transcript among `config_dirs`' projects/<slug>/*.jsonl.
pub fn latest_transcript(config_dirs: &[PathBuf], folder: &Path) -> Option<(PathBuf, SystemTime)> {
    let slug = project_slug(folder);
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for project in config_dirs.iter().flat_map(|dir| project_dirs(dir, &slug)) {
        let Ok(rd) = std::fs::read_dir(&project) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Ok(m) = e.metadata().and_then(|m| m.modified()) else { continue };
            if best.as_ref().is_none_or(|(_, t)| m > *t) {
                best = Some((p, m));
            }
        }
    }
    best
}

/// keepalive-restart's result: how many, and whose (a task only ever
/// restarts its own client's account's servers), e.g.
/// "keep-alive: restarted 1 session(s) [Claude2]".
pub fn restart_line(n: usize, account: &str) -> String {
    format!("keep-alive: restarted {n} session(s) [{account}]")
}

fn age_s(t: SystemTime, now: SystemTime) -> u64 {
    now.duration_since(t).unwrap_or(Duration::ZERO).as_secs()
}

fn ago(s: u64) -> String {
    match s {
        0..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// One status line for a repo.
pub fn status_line(repo: &str, latest: Option<(PathBuf, SystemTime)>, now: SystemTime) -> String {
    match latest {
        None => format!("{repo}: no Claude Code session yet"),
        Some((p, t)) => {
            let s = age_s(t, now);
            // The full id: what send-to-session names (owner 2026-11-17).
            let id: String = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let state = if s < ACTIVE_S { "active" } else { "idle" };
            format!("{repo}: {state}, last activity {} (session {id})", ago(s))
        }
    }
}

fn message_text(v: &Value) -> Option<(&'static str, String)> {
    let who = match v.get("type").and_then(Value::as_str) {
        Some("user") => "you",
        Some("assistant") => "claude",
        _ => return None,
    };
    let content = v.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.starts_with('<') {
        // Tool results only, or Claude Code's own wrapped notices.
        return None;
    }
    let cut: String = text.chars().take(MSG_CHARS).collect();
    Some((who, if cut.len() < text.len() { format!("{cut}…") } else { cut }))
}

/// The last `lines` messages of a transcript's text (its last 512 KB).
pub fn tail_of(bytes: &[u8], lines: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().rev() {
        if out.len() >= lines {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if let Some((who, t)) = message_text(&v) {
            out.push(format!("{who}: {t}"));
        }
    }
    out.reverse();
    let joined = crate::runbooks::redact(out.join("\n").as_bytes(), None);
    cap(joined)
}

pub fn cap(s: String) -> String {
    let n = s.chars().count();
    if n <= MAX_DETAIL_CHARS { s } else { format!("…{}", s.chars().skip(n - MAX_DETAIL_CHARS).collect::<String>()) }
}

fn read_tail(p: &Path) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).map_err(|_| "no-transcript".to_string())?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)));
    let mut b = Vec::new();
    f.read_to_end(&mut b).map_err(|_| "no-transcript".to_string())?;
    Ok(b)
}

/// The folder a transcript's session worked in (its last "cwd").
pub fn session_cwd(bytes: &[u8]) -> Option<PathBuf> {
    String::from_utf8_lossy(bytes).lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()?.get("cwd")?.as_str().map(PathBuf::from))
}

/// `cwd` when it is the repo folder or inside its `.claude\worktrees`
/// (canonical), else None.
pub fn within_repo(cwd: &Path, repo: &Path) -> Option<PathBuf> {
    let c = std::fs::canonicalize(cwd).ok()?;
    let r = std::fs::canonicalize(repo).ok()?;
    (c == r || c.starts_with(r.join(".claude").join("worktrees"))).then_some(c)
}

/// The real thing.
pub struct RealExec {
    /// Keep-alive folders (Settings → Remote), read when a task runs.
    pub keepalive_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    /// The Claude accounts (accounts::list), read when a task runs.
    pub accounts: Box<dyn Fn() -> Vec<crate::accounts::ClaudeAccount> + Send + Sync>,
}

/// Whose sessions a task may reach: the account's own config folder, and
/// every other one Glim knows (to tell "not yours" from "none at all").
pub struct Scope {
    pub account: crate::accounts::ClaudeAccount,
    pub own: PathBuf,
    pub others: Vec<PathBuf>,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_lowercase();
    key(a) == key(b)
}

/// The scope for `client`: None when no account is mapped to it.
pub fn scope(accounts: &[crate::accounts::ClaudeAccount], client: &str, known: &[PathBuf], home: &Path) -> Option<Scope> {
    let account = crate::accounts::for_client(accounts, client)?.clone();
    let own = crate::accounts::config_dir(&account, home);
    let mut others: Vec<PathBuf> = accounts.iter().map(|a| crate::accounts::config_dir(a, home)).chain(known.iter().cloned()).filter(|d| !same_dir(d, &own)).collect();
    others.dedup_by(|a, b| same_dir(a, b));
    Some(Scope { account, own, others })
}

impl Scope {
    /// This account's newest transcript for the folder.
    pub fn latest(&self, folder: &Path) -> Option<(PathBuf, SystemTime)> {
        latest_transcript(std::slice::from_ref(&self.own), folder)
    }

    /// The transcript of session `id` for the folder (or one of its
    /// worktrees) in this account's own config folder. Another account's:
    /// `not-your-session`; nowhere: `no-such-session`.
    pub fn session(&self, folder: &Path, id: &str) -> Result<PathBuf, String> {
        if let Some(p) = session_transcript(std::slice::from_ref(&self.own), folder, id) {
            return Ok(p);
        }
        Err(if session_transcript(&self.others, folder, id).is_some() { "not-your-session".into() } else { "no-such-session".into() })
    }

    /// Why there's no session of its own: another account has one, or none does.
    pub fn none_reason(&self, folder: &Path) -> String {
        if latest_transcript(&self.others, folder).is_some() { "not-your-session".into() } else { "no-transcript".into() }
    }
}

impl RealExec {
    fn folder(&self, repo: &str) -> Result<PathBuf, String> {
        let from_list = (self.keepalive_folders)().into_iter().find(|f| crate::keepalive::display_name(f).eq_ignore_ascii_case(repo));
        let raw = match from_list {
            Some(f) => f,
            None => {
                let p = crate::platform::home_dir().join(repo);
                if !p.join(".git").exists() {
                    return Err("no-folder".into());
                }
                p.to_string_lossy().into_owned()
            }
        };
        crate::runbooks::allowed_folder(&raw).map_err(|_| "no-folder".to_string())
    }

    fn claude(&self, folder: &Path) -> Result<PathBuf, String> {
        let exe = crate::platform::find_claude().ok_or("no-claude")?;
        let canon = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
        if canon.starts_with(folder) {
            return Err("claude-in-folder".into());
        }
        Ok(exe)
    }

    fn scope(&self, client: &str) -> Result<Scope, String> {
        let home = crate::platform::home_dir();
        scope(&(self.accounts)(), client, &crate::hooks::config_dirs(), &home).ok_or_else(|| "not-your-session".to_string())
    }
}

impl Exec for RealExec {
    fn run(&self, c: &Command, client: &str) -> Result<String, String> {
        let now = SystemTime::now();
        match c {
            Command::SessionStatus { repo } => {
                let scope = self.scope(client)?;
                let repos: Vec<&str> = match repo {
                    Some(r) => vec![r.as_str()],
                    None => REPOS.to_vec(),
                };
                let lines: Vec<String> = repos
                    .iter()
                    .map(|r| match self.folder(r) {
                        Ok(f) => format!("{} [{}]", status_line(r, scope.latest(&f), now), scope.account.name),
                        Err(_) => format!("{r}: folder not found on this PC"),
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
            Command::TranscriptTail { repo, lines } => {
                let scope = self.scope(client)?;
                let f = self.folder(repo)?;
                let (p, _) = scope.latest(&f).ok_or_else(|| scope.none_reason(&f))?;
                Ok(tail_of(&read_tail(&p)?, *lines as usize))
            }
            Command::StartSession { repo, prompt } => {
                let scope = self.scope(client)?;
                let f = self.folder(repo)?;
                let exe = self.claude(&f)?;
                let (var, dir) = crate::accounts::env(&scope.account);
                match prompt {
                    Some(p) => {
                        // `-w`: a git worktree of its own (owner 2026-11-15).
                        crate::platform::start_hidden_with_stdin(&exe, &["-p", "-w", "--output-format", "text"], &f, &[(var, dir.as_str())], p)?;
                        Ok(format!("{repo}: started a Claude Code task (claude -p) as {}", scope.account.name))
                    }
                    None => {
                        let folder = f.to_string_lossy().into_owned();
                        let line = crate::keepalive::launch_line_for(&exe.to_string_lossy(), &folder, &scope.account);
                        crate::platform::start_in_console(&line, &f, &[(crate::keepalive::ENV_NAME, crate::keepalive::ENV_VALUE), (var, dir.as_str())], true)?;
                        Ok(format!("{repo}: started a remote-control session as {}", scope.account.name))
                    }
                }
            }
            Command::SendToSession { repo, session, text } => {
                // The named session only (owner 2026-11-17): this account's,
                // for this repo, still there; else refused.
                let scope = self.scope(client)?;
                let f = self.folder(repo)?;
                let transcript = scope.session(&f, session)?;
                let t = std::fs::metadata(&transcript).and_then(|m| m.modified()).map_err(|_| "no-such-session".to_string())?;
                if age_s(t, now) < BUSY_S {
                    return Err("busy".into());
                }
                // Continued where that session works: the repo folder or one
                // of its worktrees, never anywhere else.
                let at = read_tail(&transcript).ok().and_then(|b| session_cwd(&b)).and_then(|cwd| within_repo(&cwd, &f)).unwrap_or_else(|| f.clone());
                let exe = self.claude(&at)?;
                let (var, dir) = crate::accounts::env(&scope.account);
                crate::platform::start_hidden_with_stdin(&exe, &["-p", "--resume", session.as_str(), "--output-format", "text"], &at, &[(var, dir.as_str())], text)?;
                Ok(format!("{repo}: sent to {}'s session {session} (claude -p --resume)", scope.account.name))
            }
            Command::KeepaliveRestart { folder } => {
                // Only this client's account's servers.
                let scope = self.scope(client)?;
                let account = if scope.account.config_dir.is_empty() { String::new() } else { scope.account.name.clone() };
                let n = crate::keepalive::restart(folder.as_deref(), Some(&account))?;
                crate::log::line(format!("remote: keepalive-restart restarted {n}"));
                Ok(restart_line(n, &scope.account.name))
            }
            Command::Runbook { name } => {
                let r = crate::runbooks::run_now(name, None, crate::runbooks::Trigger::Remote, &[], &mut |_| {});
                match r {
                    Ok(steps) => {
                        let done = steps.iter().filter(|o| o.ok).count();
                        let total = steps.first().map(|o| o.total).unwrap_or(0);
                        if done == steps.len() {
                            Ok(format!("{name}: OK ({done}/{total} steps)"))
                        } else {
                            Err(format!("runbook-failed-{done}-of-{total}"))
                        }
                    }
                    Err(e) => Err(format!("runbook-{}", e.code())),
                }
            }
        }
    }
}
