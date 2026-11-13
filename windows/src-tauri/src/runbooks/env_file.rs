// A runbook's `env_secret` step (runbooks/mod.rs): one saved secret written
// as KEY=value into a dotenv-style file.
//
// Before anything is written, git must say the file is ignored and not
// tracked (`git ls-files --error-unmatch`, then `git check-ignore`): a
// tracked file, one git doesn't ignore, or a folder that isn't in a git repo
// is refused. The old file's backup (dated) and the temp file never go in
// the project, or in any git repository: they live in
// %LOCALAPPDATA%\Glim\backups\<runbook>\ (owner 2026-11-12: a backup beside
// `.env.local` wasn't covered by the repo's `*.local` rule), and nothing in
// the repo is touched but the file itself. The other lines stay as
// they were; an existing KEY is replaced (later repeats of it dropped); the
// line endings are kept. The new file is written in the backups folder and
// moved over the old one in one step (same drive needed). Errors never
// quote the value.

use std::path::Path;
use std::process::Command;

use crate::secrets::SecretValue;

/// Backups kept per file; older ones go.
pub const KEEP_BACKUPS: usize = 3;

/// What `git` said about a file.
#[derive(Debug, PartialEq, Eq)]
pub enum Git {
    Ignored,
    NotIgnored,
    Tracked,
    NotARepo,
    Missing,
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::platform::no_console(&mut c);
    c
}

/// Asks git about `file` (in its own folder): tracked beats ignored.
pub fn git_status(file: &Path) -> Git {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return Git::NotARepo };
    let tracked = match git(dir).args(["ls-files", "--error-unmatch", "--"]).arg(name).status() {
        Err(_) => return Git::Missing,
        Ok(s) => s.code(),
    };
    if tracked == Some(0) {
        return Git::Tracked;
    }
    match git(dir).args(["check-ignore", "-q", "--"]).arg(name).status().map(|s| s.code()) {
        Ok(Some(0)) => Git::Ignored,
        Ok(Some(1)) => Git::NotIgnored,
        Ok(_) => Git::NotARepo,
        Err(_) => Git::Missing,
    }
}

fn refusal(g: &Git, what: &str) -> String {
    match g {
        Git::Ignored => String::new(),
        Git::NotIgnored => format!("{what} is not ignored by git: add it to .gitignore first"),
        Git::Tracked => format!("{what} is tracked by git: refusing to put a secret in it"),
        Git::NotARepo => format!("{what} is not in a git repository: refusing (Glim only writes files git ignores)"),
        Git::Missing => "git isn't installed or isn't on PATH".to_string(),
    }
}

/// The value as it goes after `KEY=`: bare when it is plain, else in double
/// quotes with `\` and `"` escaped.
pub fn quoted(value: &str) -> String {
    let plain = value.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:/+=@~,".contains(c));
    if plain {
        value.to_string()
    } else {
        let mut s = String::from("\"");
        for c in value.chars() {
            if c == '\\' || c == '"' {
                s.push('\\');
            }
            s.push(c);
        }
        s.push('"');
        s
    }
}

/// Whether `line` sets `key` (`KEY=…`, `KEY =…`, `export KEY=…`).
fn sets(line: &str, key: &str) -> bool {
    let l = line.trim_start();
    let l = l.strip_prefix("export ").map(str::trim_start).unwrap_or(l);
    l.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// `text` with `key` set to `value`: the first line setting it replaced (its
/// `export ` kept), later ones dropped, or a new line at the end. Every other
/// line, and the line ending style, unchanged.
pub fn set_line(text: &str, key: &str, value: &str) -> String {
    let crlf = text.contains("\r\n");
    let nl = if crlf { "\r\n" } else { "\n" };
    let body = text.replace("\r\n", "\n");
    let had_final = body.ends_with('\n') || body.is_empty();
    let mut lines: Vec<String> = if body.is_empty() { Vec::new() } else { body.split('\n').map(str::to_string).collect() };
    if body.ends_with('\n') {
        lines.pop();
    }
    let new_line = |old: &str| {
        let export = old.trim_start().starts_with("export ");
        format!("{}{key}={}", if export { "export " } else { "" }, quoted(value))
    };
    let mut done = false;
    let mut out = Vec::with_capacity(lines.len() + 1);
    for l in lines {
        if sets(&l, key) {
            if !done {
                out.push(new_line(&l));
                done = true;
            }
        } else {
            out.push(l);
        }
    }
    if !done {
        out.push(new_line(""));
    }
    let mut s = out.join(nl);
    if had_final || !done {
        s.push_str(nl);
    }
    s
}

fn stamp() -> String {
    let t = crate::platform::local_time();
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// Whether `dir` is inside a git work tree.
fn inside_repo(dir: &Path) -> bool {
    git(dir).args(["rev-parse", "--is-inside-work-tree"]).status().map(|s| s.success()).unwrap_or(false)
}

/// The start of every backup name for `file` in its runbook's folder: the
/// file name and a short hash of its full path, so two projects' `.env.local`
/// never mix.
pub fn backup_prefix(file: &Path) -> String {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let full = super::show(file).to_ascii_lowercase();
    format!("{name}.{}.bak-", &super::sha256_hex(full.as_bytes())[..8])
}

/// Drops all but the newest KEEP_BACKUPS backups of `file` in `backups`.
fn prune_backups(backups: &Path, file: &Path) {
    let prefix = backup_prefix(file);
    let mut found: Vec<_> = std::fs::read_dir(backups)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&prefix)).collect())
        .unwrap_or_default();
    found.sort();
    while found.len() > KEEP_BACKUPS {
        let _ = std::fs::remove_file(backups.join(found.remove(0)));
    }
}

/// The env_secret step itself. `backups`: this runbook's backup folder,
/// outside every repository (refused otherwise).
pub fn write_secret(file: &Path, key: &str, value: &SecretValue, backups: &Path) -> Result<(), String> {
    let shown = super::show(file);
    match git_status(file) {
        Git::Ignored => {}
        other => return Err(refusal(&other, &shown)),
    }
    std::fs::create_dir_all(backups).map_err(|e| format!("couldn't make the backup folder: {e}"))?;
    let _ = crate::platform::ensure_private_dir(backups);
    if inside_repo(backups) {
        return Err(format!("the backup folder {} is inside a git repository: refusing", super::show(backups)));
    }
    let backup = backups.join(format!("{}{}", backup_prefix(file), stamp()));
    let temp = backups.join(format!("{}tmp-{}", backup_prefix(file), std::process::id()));
    let old = match std::fs::read(file) {
        Ok(b) => Some(String::from_utf8(b).map_err(|_| format!("{shown} is not UTF-8 text"))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("couldn't read {shown}: {e}")),
    };
    if old.is_some() {
        std::fs::copy(file, &backup).map_err(|e| format!("couldn't back up {shown}: {e}"))?;
    }
    let mut new = set_line(old.as_deref().unwrap_or(""), key, value.reveal());
    let written = std::fs::write(&temp, new.as_bytes()).and_then(|()| std::fs::rename(&temp, file));
    // SAFETY: zeroes are valid UTF-8.
    unsafe { new.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("couldn't write {shown} (it must be on the same drive as %LOCALAPPDATA%)"));
    }
    prune_backups(backups, file);
    Ok(())
}
