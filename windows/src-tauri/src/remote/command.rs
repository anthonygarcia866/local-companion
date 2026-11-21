// The remote command parser. Seven commands, each with fixed, checked
// arguments; anything else is refused.
//
//   session-status   {"repo"?: name}
//   transcript-tail  {"repo": name, "lines"?: 1-50 (20)}
//   start-session    {"repo": name, "prompt"?: 1-2000 chars}
//   send-to-session  {"repo": name, "session": its id, "text": 1-4000 chars}
//                    (owner 2026-11-17: the target session must be named;
//                    its full id as session-status shows it)
//   runbook          {"name": runbook name}  (approved ones whose file gives
//                    them a remote level; docs/remote-runbooks.md)
//   run-status       {"run"?: run id}  this client's own remote runbook
//                    runs (owner 2026-11-20, D2): status and times only
//   keepalive-restart {"folder"?: name}      restarts "Keep Claude Code
//                    reachable" sessions: the one whose folder has that name,
//                    or all; only folders already on its list (owner
//                    2026-11-15). 3 an hour.
//
// Repos must be on remote::REPOS. Free text (prompt, text) is the signer's
// own words; a transport that passes on text from anyone else must wrap it
// as quoted material (ai/quoted.rs, Source::Remote).

use serde_json::{Map, Value};

use super::{Rejection, REPOS};

pub const MAX_TAIL_LINES: u64 = 50;
pub const DEFAULT_TAIL_LINES: u64 = 20;
pub const MAX_PROMPT_CHARS: usize = 2000;
pub const MAX_TEXT_CHARS: usize = 4000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    SessionStatus { repo: Option<String> },
    TranscriptTail { repo: String, lines: u64 },
    StartSession { repo: String, prompt: Option<String> },
    /// `session`: a Claude Code session id (a lower-case UUID).
    SendToSession { repo: String, session: String, text: String },
    Runbook { name: String },
    /// A remote runbook run's id (32 hex), or None for the client's recent runs.
    RunStatus { run: Option<String> },
    /// A keep-alive folder's name (its last part), or None for all of them.
    KeepaliveRestart { folder: Option<String> },
}

impl Command {
    pub fn kind(&self) -> &'static str {
        match self {
            Command::SessionStatus { .. } => "session-status",
            Command::TranscriptTail { .. } => "transcript-tail",
            Command::StartSession { .. } => "start-session",
            Command::SendToSession { .. } => "send-to-session",
            Command::Runbook { .. } => "runbook",
            Command::RunStatus { .. } => "run-status",
            Command::KeepaliveRestart { .. } => "keepalive-restart",
        }
    }
}

/// Only these keys, nothing else.
fn only(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), Rejection> {
    if args.keys().all(|k| allowed.contains(&k.as_str())) {
        Ok(())
    } else {
        Err(Rejection::BadArgs)
    }
}

fn repo(args: &Map<String, Value>, required: bool) -> Result<Option<String>, Rejection> {
    match args.get("repo") {
        None if !required => Ok(None),
        None => Err(Rejection::BadArgs),
        Some(Value::String(r)) => {
            let r = r.trim();
            if REPOS.iter().any(|x| x.eq_ignore_ascii_case(r)) {
                Ok(Some(r.to_ascii_lowercase()))
            } else {
                Err(Rejection::RepoNotAllowed)
            }
        }
        Some(_) => Err(Rejection::BadArgs),
    }
}

/// A text argument: a string of 1..=max characters, no control characters
/// but line breaks and tabs.
fn text(args: &Map<String, Value>, key: &str, max: usize, required: bool) -> Result<Option<String>, Rejection> {
    match args.get(key) {
        None if !required => Ok(None),
        Some(Value::String(t)) => {
            let n = t.chars().count();
            if n == 0 || n > max || t.trim().is_empty() || t.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) {
                Err(Rejection::BadArgs)
            } else {
                Ok(Some(t.clone()))
            }
        }
        _ => Err(Rejection::BadArgs),
    }
}

/// A runbook name as runbooks/ allows them: 1-40 of a-z 0-9 - _.
fn runbook_name(args: &Map<String, Value>) -> Result<String, Rejection> {
    match args.get("name") {
        Some(Value::String(n)) if (1..=40).contains(&n.len()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') => Ok(n.clone()),
        _ => Err(Rejection::BadArgs),
    }
}

/// A Claude Code session id: a lower-case UUID (8-4-4-4-12 hex), the name of
/// its transcript file. Nothing else, so it can't name a path.
pub fn is_session_id(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5 && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
}

pub fn parse(cmd: &str, args: &Map<String, Value>) -> Result<Command, Rejection> {
    match cmd {
        "session-status" => {
            only(args, &["repo"])?;
            Ok(Command::SessionStatus { repo: repo(args, false)? })
        }
        "transcript-tail" => {
            only(args, &["repo", "lines"])?;
            let repo = repo(args, true)?.unwrap_or_default();
            let lines = match args.get("lines") {
                None => DEFAULT_TAIL_LINES,
                Some(v) => v.as_u64().filter(|n| (1..=MAX_TAIL_LINES).contains(n)).ok_or(Rejection::BadArgs)?,
            };
            Ok(Command::TranscriptTail { repo, lines })
        }
        "start-session" => {
            only(args, &["repo", "prompt"])?;
            let repo = repo(args, true)?.unwrap_or_default();
            Ok(Command::StartSession { repo, prompt: text(args, "prompt", MAX_PROMPT_CHARS, false)? })
        }
        "send-to-session" => {
            only(args, &["repo", "session", "text"])?;
            let repo = repo(args, true)?.unwrap_or_default();
            let session = match args.get("session") {
                Some(Value::String(s)) if is_session_id(s) => s.clone(),
                _ => return Err(Rejection::BadArgs),
            };
            let text = text(args, "text", MAX_TEXT_CHARS, true)?.unwrap_or_default();
            Ok(Command::SendToSession { repo, session, text })
        }
        "runbook" => {
            only(args, &["name"])?;
            Ok(Command::Runbook { name: runbook_name(args)? })
        }
        "run-status" => {
            only(args, &["run"])?;
            let run = match args.get("run") {
                None => None,
                Some(Value::String(r)) if super::runs::id_ok(r) => Some(r.clone()),
                Some(_) => return Err(Rejection::BadArgs),
            };
            Ok(Command::RunStatus { run })
        }
        "keepalive-restart" => {
            only(args, &["folder"])?;
            let folder = match args.get("folder") {
                None => None,
                // A folder's display name (keepalive::display_name): matched
                // against the keep-alive list on this PC, never a path.
                Some(Value::String(f)) if (1..=80).contains(&f.trim().chars().count()) && f.trim().chars().all(|c| c.is_alphanumeric() || " ._-()".contains(c)) => Some(f.trim().to_string()),
                Some(_) => return Err(Rejection::BadArgs),
            };
            Ok(Command::KeepaliveRestart { folder })
        }
        _ => Err(Rejection::UnknownCommand),
    }
}
