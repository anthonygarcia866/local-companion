// A runbook's `env_secret` step (runbooks/mod.rs): one saved secret written
// as KEY=value into a dotenv-style file.
//
// Before anything is written, git must say the file is ignored and not
// tracked (`git ls-files --error-unmatch`, then `git check-ignore`): a
// tracked file, one git doesn't ignore, or a folder that isn't in a git repo
// is refused. The backup of the old file (dated, beside it) must be ignored
// too: when it wouldn't be, `*.glim-backup-*` goes into the repo's own
// .git/info/exclude (never a tracked .gitignore). The other lines stay as
// they were; an existing KEY is replaced (later repeats of it dropped); the
// line endings are kept. The new file is written next to it and moved over
// it in one step. Errors never quote the value.

use std::path::Path;
use std::process::Command;

use crate::secrets::SecretValue;

/// The backups' name pattern (and the temp file's: same pattern).
pub const BACKUP_PATTERN: &str = "*.glim-backup-*";

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

/// The repo's own exclude file (`git rev-parse --git-path info/exclude`).
fn exclude_file(dir: &Path) -> Option<std::path::PathBuf> {
    let mut c = git(dir);
    c.args(["rev-parse", "--git-path", "info/exclude"]).stdout(std::process::Stdio::piped());
    let out = c.output().ok().filter(|o| o.status.success())?;
    let p = String::from_utf8(out.stdout).ok()?.trim().to_string();
    let p = std::path::PathBuf::from(p);
    Some(if p.is_absolute() { p } else { dir.join(p) })
}

/// Makes sure git ignores the backups next to `file`, through
/// .git/info/exclude when it doesn't already.
fn ensure_backups_ignored(file: &Path, probe: &Path) -> Result<(), String> {
    if git_status(probe) == Git::Ignored {
        return Ok(());
    }
    let dir = file.parent().ok_or("no folder")?;
    let exclude = exclude_file(dir).ok_or("couldn't find the repository's info/exclude")?;
    let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == BACKUP_PATTERN) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("# Glim runbook backups (env_secret)\n");
        text.push_str(BACKUP_PATTERN);
        text.push('\n');
        if let Some(d) = exclude.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("couldn't write info/exclude: {e}"))?;
        }
        std::fs::write(&exclude, text).map_err(|e| format!("couldn't write info/exclude: {e}"))?;
    }
    match git_status(probe) {
        Git::Ignored => Ok(()),
        _ => Err("git still doesn't ignore the backup file: refusing".into()),
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

/// Drops all but the newest KEEP_BACKUPS backups of `file`.
fn prune_backups(file: &Path) {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().map(|n| n.to_string_lossy().into_owned())) else { return };
    let prefix = format!("{name}.glim-backup-2");
    let mut backups: Vec<_> = std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&prefix)).collect())
        .unwrap_or_default();
    backups.sort();
    while backups.len() > KEEP_BACKUPS {
        let _ = std::fs::remove_file(dir.join(backups.remove(0)));
    }
}

/// The env_secret step itself.
pub fn write_secret(file: &Path, key: &str, value: &SecretValue) -> Result<(), String> {
    let shown = super::show(file);
    match git_status(file) {
        Git::Ignored => {}
        other => return Err(refusal(&other, &shown)),
    }
    let name = file.file_name().ok_or("not a file")?.to_string_lossy().into_owned();
    let dir = file.parent().ok_or("no folder")?;
    let backup = dir.join(format!("{name}.glim-backup-{}", stamp()));
    let temp = dir.join(format!("{name}.glim-backup-tmp-{}", std::process::id()));
    ensure_backups_ignored(file, &backup)?;
    if git_status(&temp) != Git::Ignored {
        return Err("git doesn't ignore Glim's temporary file: refusing".into());
    }
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
        return Err(format!("couldn't write {shown}"));
    }
    prune_backups(file);
    Ok(())
}
