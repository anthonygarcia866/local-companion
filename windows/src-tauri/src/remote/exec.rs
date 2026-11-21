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
//   runbook          an approved, unchanged runbook whose file gives it a
//                    remote level (`[remote] level = "cloud"`; absent =
//                    never, refused `runbook-not-remote`; a "phone" or ask
//                    step: `runbook-needs-phone`; docs/remote-runbooks.md),
//                    as `glim.exe --run` runs it (Trigger::Remote in
//                    runbooks.log); one at a time (`runbook-busy`). A
//                    "phone" step pauses the run there (remote/runs.rs):
//                    the answer names the run id.
//   run-status       this client's own remote runbook runs (or the one it
//                    names): "<id> <name>: paused at step 3 of 5 (…)".
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
    fn run(&self, c: &Command, client: &str) -> Result<Ran, String>;
}

/// A command that ran: its detail, and the Claude Code session it started
/// or continued (the result's `session`, so cloud Glim can follow it up
/// with send-to-session or transcript-tail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub detail: String,
    pub session: Option<String>,
}

impl From<String> for Ran {
    fn from(detail: String) -> Ran {
        Ran { detail, session: None }
    }
}

/// A new session id for `claude -p --session-id` (a v4 UUID from the
/// system's randomness, the form command::is_session_id accepts).
pub fn new_session_id() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).map_err(|_| "no randomness".to_string())?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

/// Every result's detail on its way out (result format, owner 2026-11-19).
/// BEST EFFORT, not a guarantee: control characters but new lines and tabs
/// go, and what looks like a credential becomes "[secret]": a PEM block, a
/// token with a known prefix (sk-, sk_live_, ghp_, github_pat_, xoxb-, AKIA,
/// AIza, eyJ, whsec_, npm_, hf_ …; 16+ characters), a long word after
/// "Bearer", the value after a password / token / secret / api key /
/// private, access or secret key label (`password: x`, `"token":"x"`,
/// `API_KEY = x`, `x-api-key: x`, `--password x`), a URL's password
/// (`scheme://user:pw@host`) and labelled query values (`?token=x`). Session
/// ids and commit hashes are kept (cloud Glim needs them), so a bare long
/// hex or UUID is not masked.
pub fn redact_detail(s: &str) -> String {
    let clean = crate::runbooks::scrub(s.as_bytes(), None);
    let mut out: Vec<String> = Vec::new();
    // Inside a PEM block that didn't end on its line, EVERY line is masked
    // (fail closed, security review 2026-11-19) until END, at most
    // PEM_MAX_LINES; only the start of the next transcript-tail message
    // ("you: " / "claude: ", which no PEM body or armor header line can
    // be) ends it early, so a BEGIN in prose doesn't swallow the rest.
    let mut in_pem: usize = 0;
    for line in clean.split('\n') {
        if in_pem > 0 {
            if let Some(e) = line.find("-----END") {
                in_pem = 0;
                let (text, open) = pem_line(&line[e..]);
                // The END marker itself goes; what follows is checked again.
                let text = text.strip_prefix("[secret]").map(str::to_string).unwrap_or(text);
                if open {
                    in_pem = PEM_MAX_LINES;
                }
                if !text.trim().is_empty() {
                    out.push(text);
                }
                continue;
            }
            if !starts_message(line) {
                in_pem -= 1;
                continue;
            }
            in_pem = 0;
        }
        let (text, open) = pem_line(line);
        if open {
            in_pem = PEM_MAX_LINES;
        }
        out.push(text);
    }
    out.join("\n")
}

const PEM_MAX_LINES: usize = 200;

/// One line with every PEM block on it masked (security review
/// 2026-11-19: all of them, not just the first), the text around them
/// redacted; true when the last BEGIN has no END on this line (its body
/// follows on the next lines). From a BEGIN with no END, the rest of the
/// line is masked.
fn pem_line(line: &str) -> (String, bool) {
    let mut out = String::new();
    let mut rest = line;
    loop {
        // An END with no BEGIN before it (the tail of a block from earlier
        // lines): masked up to its closing dashes.
        let begin = rest.find("-----BEGIN");
        if let Some(e) = rest.find("-----END").filter(|e| begin.is_none_or(|b| *e < b)) {
            out.push_str(&redact_line(&rest[..e]));
            out.push_str("[secret]");
            rest = after_pem_end(&rest[e..]);
            continue;
        }
        let Some(b) = begin else {
            out.push_str(&redact_line(rest));
            return (out, false);
        };
        out.push_str(&redact_line(&rest[..b]));
        out.push_str("[secret]");
        let block = &rest[b..];
        match block.find("-----END") {
            Some(e) => rest = after_pem_end(&block[e..]),
            None => return (out, true),
        }
    }
}

/// The start of a transcript-tail message (exec::tail_of's prefixes).
fn starts_message(line: &str) -> bool {
    line.starts_with("you: ") || line.starts_with("claude: ") || line.starts_with("…you: ") || line.starts_with("…claude: ")
}

/// What follows a PEM END marker's closing dashes on its line.
fn after_pem_end(s: &str) -> &str {
    let Some(e) = s.find("-----END") else { return s };
    let tail = &s[e + "-----END".len()..];
    match tail.find("-----") {
        Some(c) => &tail[c + "-----".len()..],
        None => "",
    }
}

const SECRET_PREFIXES: [&str; 26] = [
    "sk-", "sk_live_", "sk_test_", "rk_live_", "rk_test_", "pk_live_", "whsec_", "ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "xoxb-", "xoxp-", "xoxa-", "xoxs-", "AKIA", "ASIA", "AIza", "eyJ", "glpat-",
    "npm_", "hf_", "shpat_", "dop_v1_", "pypi-",
];
/// Matched at a label's end, `-` read as `_` (x-api-key → x_api_key).
const SECRET_LABELS: [&str; 12] = ["password", "passwd", "pwd", "passcode", "api_key", "apikey", "secret", "token", "secret_key", "private_key", "access_key", "credentials"];

fn word_core(w: &str) -> &str {
    w.trim_matches(|c: char| "\"'`()[]{}<>,;.!?".contains(c))
}

fn is_label(w: &str) -> bool {
    let l = word_core(w).trim_start_matches('-').to_ascii_lowercase().replace('-', "_");
    !l.is_empty() && SECRET_LABELS.iter().any(|x| l.ends_with(x))
}

/// A URL's password (`scheme://user:pw@host`) and its labelled query
/// values (`?token=x&…`); None when `w` has neither.
fn mask_url(w: &str) -> Option<String> {
    let mut s = w.to_string();
    let mut changed = false;
    if let Some(p) = s.find("://") {
        let auth_end = s[p + 3..].find(['/', '?', '#']).map(|i| p + 3 + i).unwrap_or(s.len());
        if let Some(at) = s[p + 3..auth_end].rfind('@').map(|i| p + 3 + i) {
            if let Some(colon) = s[p + 3..at].find(':').map(|i| p + 3 + i) {
                s.replace_range(colon + 1..at, "[secret]");
                changed = true;
            }
        }
    }
    let mut from = 0;
    while let Some(q) = s[from..].find(['?', '&']).map(|i| from + i) {
        let key_start = q + 1;
        let Some(eq) = s[key_start..].find('=').map(|i| key_start + i) else { break };
        let key = &s[key_start..eq];
        let end = s[eq + 1..].find(['&', '#', ' ']).map(|i| eq + 1 + i).unwrap_or(s.len());
        if !key.contains(['?', '&']) && is_label(key) && end > eq + 1 {
            s.replace_range(eq + 1..end, "[secret]");
            changed = true;
            from = eq + 1 + "[secret]".len();
        } else {
            from = key_start;
        }
    }
    changed.then_some(s)
}

fn redact_line(line: &str) -> String {
    // Words and the separators between them (spaces or tabs, kept as they were).
    let words: Vec<&str> = line.split([' ', '\t']).collect();
    let seps: Vec<char> = line.chars().filter(|c| *c == ' ' || *c == '\t').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    // A label ended its word or stood before a lone "=" / ":" (or was a
    // --flag): the next value goes.
    let mut mask_next = false;
    for (i, w) in words.iter().enumerate() {
        if mask_next && !word_core(w).is_empty() && word_core(w) != "=" && word_core(w) != ":" {
            mask_next = false;
            out.push("[secret]".into());
            continue;
        }
        // A URL's password and labelled query values first, then every other
        // check on what's left (security review 2026-11-19: never instead).
        let masked_url = mask_url(w);
        let w: &str = masked_url.as_deref().unwrap_or(*w);
        let c = word_core(w);
        if let Some(cut) = c.find(['=', ':']) {
            // "password:hunter2", "token=abc", "\"token\":\"x\"", "password:" (value next)
            if is_label(c[..cut].trim_end()) {
                if cut + 1 < c.len() {
                    out.push(format!("{}[secret]", &c[..=cut]));
                } else {
                    mask_next = true;
                    out.push(w.to_string());
                }
                continue;
            }
        } else if is_label(c) {
            let next = words.get(i + 1).map(|n| word_core(n));
            // "password = hunter2", "--password hunter2"; prose ("the token works") is left alone.
            if matches!(next, Some("=") | Some(":")) || (c.starts_with("--") && next.is_some_and(|n| !n.is_empty() && !n.starts_with('-'))) {
                mask_next = true;
                out.push(w.to_string());
                continue;
            }
        }
        let prefixed = c.len() >= 16 && SECRET_PREFIXES.iter().any(|p| c.starts_with(p));
        let bearer = i > 0 && word_core(words[i - 1]).eq_ignore_ascii_case("bearer") && c.len() >= 16;
        out.push(if prefixed || bearer { "[secret]".into() } else { w.to_string() });
    }
    let mut joined = String::with_capacity(line.len());
    for (i, w) in out.iter().enumerate() {
        if i > 0 {
            joined.push(seps.get(i - 1).copied().unwrap_or(' '));
        }
        joined.push_str(w);
    }
    joined
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
    // `scrub`, not `redact`: redact's own 4,000-character cap (Settings'
    // runbook output) cut a 50-message tail to its last ~10 messages, mid
    // message (2026-11-19). Oldest whole messages go first to fit 12,000.
    let mut msgs: Vec<String> = out.iter().map(|m| crate::runbooks::scrub(m.as_bytes(), None).replace('\n', " ")).collect();
    let mut total: usize = msgs.iter().map(|m| m.chars().count() + 1).sum();
    let mut dropped = false;
    while msgs.len() > 1 && total > MAX_DETAIL_CHARS {
        total -= msgs[0].chars().count() + 1;
        msgs.remove(0);
        dropped = true;
    }
    // Older messages left out: marked, so the result says `capped`.
    let joined = msgs.join("\n");
    cap(if dropped { format!("…{joined}") } else { joined })
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
    fn run(&self, c: &Command, client: &str) -> Result<Ran, String> {
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
                Ok(lines.join("\n").into())
            }
            Command::TranscriptTail { repo, lines } => {
                let scope = self.scope(client)?;
                let f = self.folder(repo)?;
                let (p, _) = scope.latest(&f).ok_or_else(|| scope.none_reason(&f))?;
                Ok(tail_of(&read_tail(&p)?, *lines as usize).into())
            }
            Command::StartSession { repo, prompt } => {
                let scope = self.scope(client)?;
                let f = self.folder(repo)?;
                let exe = self.claude(&f)?;
                let (var, dir) = crate::accounts::env(&scope.account);
                match prompt {
                    Some(p) => {
                        // `-w`: a git worktree of its own (owner 2026-11-15).
                        // Its id chosen here (`--session-id`, 2026-11-19), so
                        // the result names the session cloud Glim can follow up.
                        let id = new_session_id()?;
                        crate::platform::start_hidden_with_stdin(&exe, &["-p", "-w", "--session-id", id.as_str(), "--output-format", "text"], &f, &[(var, dir.as_str())], p)?;
                        Ok(Ran { detail: format!("{repo}: started a Claude Code task (claude -p) as {}, session {id}", scope.account.name), session: Some(id) })
                    }
                    None => {
                        let folder = f.to_string_lossy().into_owned();
                        let line = crate::keepalive::launch_line_for(&exe.to_string_lossy(), &folder, &scope.account);
                        crate::platform::start_in_console(&line, &f, &[(crate::keepalive::ENV_NAME, crate::keepalive::ENV_VALUE), (var, dir.as_str())], true)?;
                        // Remote control names its sessions itself, later: none here.
                        Ok(format!("{repo}: started a remote-control session as {}", scope.account.name).into())
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
                Ok(Ran { detail: format!("{repo}: sent to {}'s session {session} (claude -p --resume)", scope.account.name), session: Some(session.clone()) })
            }
            Command::KeepaliveRestart { folder } => {
                // Only this client's account's servers.
                let scope = self.scope(client)?;
                let account = if scope.account.config_dir.is_empty() { String::new() } else { scope.account.name.clone() };
                let n = crate::keepalive::restart(folder.as_deref(), Some(&account))?;
                crate::log::line(format!("remote: keepalive-restart restarted {n}"));
                Ok(restart_line(n, &scope.account.name).into())
            }
            Command::Runbook { name } => {
                // One remote run at a time (owner 2026-11-20, D6); a paused
                // run counts (runs::Runs::busy).
                let Some(_running) = RemoteRun::claim() else { return Err("runbook-busy".into()) };
                let id = super::runs::new_id()?;
                super::runs::with(|r| r.begin(&id, name, client, super::now_s()))?;
                // Run outside the lock: the kill switch and run-status stay free.
                let stopped = || super::runs::with(|r| r.killed(&id));
                let seg = crate::runbooks::run_remote_now(name, None, 0, false, &stopped);
                let run = super::runs::with(|r| r.began(&id, seg, super::now_s()))?;
                if run.state == super::runs::RunState::Paused {
                    ask_phone(&run);
                }
                runbook_answer(&run)
            }
            Command::RunStatus { run } => {
                let mine = super::runs::with(|r| r.for_client(client, run.as_deref(), super::now_s()));
                if mine.is_empty() {
                    return Err(if run.is_some() { "no-such-run".into() } else { "no-runs".into() });
                }
                Ok(mine.iter().map(super::runs::line).collect::<Vec<_>>().join("\n").into())
            }
        }
    }
}

/// A runbook task's answer: codes and counts only (docs/remote-runbooks.md
/// §3.4); the run id, so cloud Glim can follow it with run-status.
pub fn runbook_answer(run: &super::runs::Run) -> Result<Ran, String> {
    use super::runs::RunState;
    match run.state {
        RunState::Done => Ok(format!("{}: OK ({}/{} steps) (run {})", run.name, run.done, run.total, run.id).into()),
        RunState::Paused => Ok(format!("{}: paused at step {} of {}, waiting for the owner's OK (run {})", run.name, run.step, run.total, run.id).into()),
        RunState::Failed => Err(format!("runbook-failed-{}-of-{}", run.done, run.total)),
        _ => Err(format!("runbook-{}", run.status())),
    }
}

/// Resumes a paused remote run at its paused step, once the owner OK'd it
/// (phone approval, PR 3, calls this after the phone's signature verified).
pub fn resume_run(id: &str) -> Result<super::runs::Run, String> {
    let Some(_running) = RemoteRun::claim() else { return Err("runbook-busy".into()) };
    let (name, hash, from) = super::runs::with(|r| r.begin_resume(id, super::now_s()))?;
    let stopped = || super::runs::with(|r| r.killed(id));
    let seg = crate::runbooks::run_remote_now(&name, Some(&hash), from, true, &stopped);
    let run = super::runs::with(|r| r.resumed(id, seg, super::now_s()))?;
    // Another phone step further on: ask again.
    if run.state == super::runs::RunState::Paused {
        ask_phone(&run);
    }
    Ok(run)
}

/// What the courier hands this module once (courier::start): the settings,
/// the relay outbox and the phone (Pushover, with a link).
pub struct Hooks {
    pub settings: std::sync::Arc<dyn Fn() -> super::RemoteSettings + Send + Sync>,
    pub send: std::sync::Arc<dyn Fn(String) + Send + Sync>,
    pub phone: std::sync::Arc<dyn Fn(String, String) + Send + Sync>,
}

static HOOKS: std::sync::OnceLock<Hooks> = std::sync::OnceLock::new();

pub fn set_hooks(h: Hooks) {
    let _ = HOOKS.set(h);
}

/// A run paused at a phone step: one approval request, sealed to every
/// enrolled approver, and a Pushover ping with its match code and link
/// (docs/remote-runbooks.md §8). No approver: it waits, and expires.
pub fn ask_phone(run: &super::runs::Run) {
    use super::approve;
    let Some(h) = HOOKS.get() else { return };
    let s = (h.settings)();
    let keys = approve::read_keys(&approve::keys_vault());
    let approvers: Vec<(approve::Approver, [u8; 32])> = s
        .approvers
        .iter()
        .filter_map(|a| keys.get(&a.id).and_then(|k| super::seal::from_hex(k)).and_then(|k| <[u8; 32]>::try_from(k).ok()).map(|k| (a.clone(), k)))
        .collect();
    if approvers.is_empty() {
        crate::log::line("remote: runbook paused; no phone approver enrolled");
        return;
    }
    let host = s.relay.trim_start_matches("https://").trim_end_matches('/').to_string();
    let (Some(rid), Some(code)) = (approve::new_slot_id(), approve::new_match_code()) else { return };
    let Some(req) = approve::request(&rid, &code, run, &approvers, &host) else { return };
    super::runs::with(|r| r.set_approval(&run.id, &rid, &code));
    (h.send)(req.frame);
    (h.phone)(req.text, req.url);
    crate::log::line("remote: phone approval requested");
}

/// The approval page's answer to request `rid`, as the relay hands it over.
/// The request is used up whatever the answer; only a verified Approve
/// resumes the run (on a thread of its own).
pub fn on_approval_answer(rid: &str, answer: &str, now: i64) {
    use super::approve::{judge, Verdict};
    // Acknowledged first, whatever happens next (review 2026-11-21): the
    // relay never hands the same answer over again.
    super::courier::send_frame(serde_json::json!({ "t": "approval-ack", "id": rid }).to_string());
    let Some(h) = HOOKS.get() else {
        // Not set up yet (can't judge it): the request is used up and its
        // run ends failed `not-ready`, visibly, rather than waiting on an
        // answer that already came.
        if let Some(run) = super::runs::with(|r| r.take_approval(rid, now)) {
            let _ = super::runs::with(|r| r.refuse(&run.id, "not-ready", now));
        }
        crate::log::line("remote: phone approval refused not-ready");
        return;
    };
    let s = (h.settings)();
    if s.killed {
        crate::log::line("remote: phone approval refused killed");
        return;
    }
    let Some(run) = super::runs::with(|r| r.take_approval(rid, now)) else {
        crate::log::line("remote: phone approval refused no-waiting-run");
        return;
    };
    match judge(answer, rid, &run, &run.code, &s.approvers) {
        Verdict::Approve => {
            crate::log::line("remote: phone approval verified; resuming");
            let id = run.id.clone();
            let _ = std::thread::Builder::new().name("remote-resume".into()).spawn(move || {
                let wait = || std::thread::sleep(RESUME_WAIT);
                if let Err(e) = resume_when_free(&id, &resume_run, &wait, RESUME_TRIES) {
                    if e == "runbook-busy" {
                        let _ = super::runs::with(|r| r.refuse(&id, "runbook-busy", super::now_s()));
                    }
                    crate::log::line(format!("remote: resume refused {e}"));
                }
            });
        }
        Verdict::Stop => {
            let _ = super::runs::with(|r| r.decline(&run.id, now));
            crate::log::line("remote: phone approval declined");
        }
        Verdict::Refused(code) => {
            let _ = super::runs::with(|r| r.refuse(&run.id, code, now));
            crate::log::line(format!("remote: phone approval refused {code}"));
        }
    }
}

/// A verified Approve can arrive while the segment before it still holds the
/// one-run claim (it asked the phone just before returning): its resume
/// waits for the claim, up to RESUME_TRIES x RESUME_WAIT (review 2026-11-21).
const RESUME_TRIES: u32 = 120;
const RESUME_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

/// Resumes run `id`, trying again while the claim is busy. The run stays
/// paused meanwhile, so the kill switch (or the expiry) still ends it and the
/// next try is refused. Still busy after every try: `runbook-busy` (the
/// caller then ends the run failed, rather than leaving it paused with its
/// approval used up).
fn resume_when_free(id: &str, resume: &dyn Fn(&str) -> Result<super::runs::Run, String>, wait: &dyn Fn(), tries: u32) -> Result<super::runs::Run, String> {
    let mut left = tries;
    loop {
        match resume(id) {
            Err(e) if e == "runbook-busy" && left > 0 => {
                left -= 1;
                wait();
            }
            r => return r,
        }
    }
}

/// The one remote runbook run allowed at a time; released when dropped.
struct RemoteRun;

static REMOTE_RUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl RemoteRun {
    fn claim() -> Option<RemoteRun> {
        use std::sync::atomic::Ordering;
        REMOTE_RUN.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok().map(|_| RemoteRun)
    }
}

impl Drop for RemoteRun {
    fn drop(&mut self) {
        REMOTE_RUN.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod remote_run_tests {
    use super::RemoteRun;

    #[test]
    fn one_remote_runbook_run_at_a_time() {
        let first = RemoteRun::claim().expect("free at first");
        assert!(RemoteRun::claim().is_none(), "a second is busy");
        drop(first);
        assert!(RemoteRun::claim().is_some(), "free again once the first ends");
    }

    #[test]
    fn a_verified_approve_waits_for_the_claim_instead_of_being_lost() {
        use std::cell::Cell;
        let calls = Cell::new(0u32);
        let waits = Cell::new(0u32);
        let run = super::super::runs::Run { id: "ab".repeat(16), ..Default::default() };
        // Busy twice, then free: resumed, after two waits.
        let resume = |_: &str| {
            calls.set(calls.get() + 1);
            if calls.get() <= 2 { Err("runbook-busy".to_string()) } else { Ok(run.clone()) }
        };
        let wait = || waits.set(waits.get() + 1);
        assert_eq!(super::resume_when_free(&run.id, &resume, &wait, 5).unwrap().id, run.id);
        assert_eq!((calls.get(), waits.get()), (3, 2));
        // Any other refusal (killed meanwhile, expired) ends it at once.
        calls.set(0);
        let killed = |_: &str| {
            calls.set(calls.get() + 1);
            Err::<super::super::runs::Run, String>("run-killed".into())
        };
        assert_eq!(super::resume_when_free(&run.id, &killed, &wait, 5).unwrap_err(), "run-killed");
        assert_eq!(calls.get(), 1);
        // Busy every time: gives up after the tries, never loops forever.
        calls.set(0);
        let busy = |_: &str| {
            calls.set(calls.get() + 1);
            Err::<super::super::runs::Run, String>("runbook-busy".into())
        };
        assert_eq!(super::resume_when_free(&run.id, &busy, &wait, 3).unwrap_err(), "runbook-busy");
        assert_eq!(calls.get(), 4);
    }
}
