// The last update's result (runbooks round 2, owner 2026-11-13).
//
// An updater the owner runs (a runbook's `launch_detached` step; the script
// lives outside the repo) stops Glim, rebuilds and reinstalls it, and leaves
// %LOCALAPPDATA%\Glim\update-result.json behind BEFORE starting the new Glim:
// the commit, OK/FAIL, the self-test's count and each step's name, outcome
// and time. No secrets and no command output are in it (the script writes
// short fixed English details only).
//
// On startup the island shows it once ("Updated to <commit>: self-test 6/6"
// or the failure): `take_notice` remembers the run's id in
// update-result.seen. Settings → Runbooks always shows the last one. A result
// older than a day never pops up on the island.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "update-result.json";
const SEEN: &str = "update-result.seen";
/// A result this old never pops up on the island (Settings still shows it).
pub const FRESH: Duration = Duration::from_secs(24 * 3600);
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub ms: u64,
    pub detail: String,
}

/// update-result.json as the updater writes it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateResult {
    pub run_id: String,
    /// Short commit ("1a2b3c4d").
    pub commit: String,
    pub ok: bool,
    /// "6/6", or the self-test's last line when it failed.
    pub selftest: String,
    pub failed_step: String,
    pub error: String,
    /// ISO 8601, as written.
    pub finished_at: String,
    pub total_ms: u64,
    pub steps: Vec<StepResult>,
}

fn clip(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

/// Reads and tidies a result: every text field is clipped, so a broken file
/// can't flood the island.
pub fn parse(bytes: &[u8]) -> Option<UpdateResult> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut r: UpdateResult = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    if r.run_id.trim().is_empty() {
        return None;
    }
    r.run_id = clip(&r.run_id, 80);
    r.commit = clip(&r.commit, 12);
    r.selftest = clip(&r.selftest, 80);
    r.failed_step = clip(&r.failed_step, 40);
    r.error = clip(&r.error, 200);
    r.finished_at = clip(&r.finished_at, 40);
    r.steps.truncate(30);
    for s in &mut r.steps {
        s.name = clip(&s.name, 40);
        s.detail = clip(&s.detail, 200);
    }
    Some(r)
}

/// The result in `dir`, if there is one.
pub fn read_in(dir: &Path) -> Option<UpdateResult> {
    let p = dir.join(FILE);
    let m = std::fs::symlink_metadata(&p).ok()?;
    if !m.is_file() || m.len() > MAX_BYTES {
        return None;
    }
    parse(&std::fs::read(&p).ok()?)
}

/// The result to show on the island now, once: a fresh one whose run id
/// isn't in the seen file yet. Remembers it as seen.
pub fn take_notice_in(dir: &Path, now: SystemTime) -> Option<UpdateResult> {
    let r = read_in(dir)?;
    let modified = std::fs::metadata(dir.join(FILE)).and_then(|m| m.modified()).ok()?;
    if now.duration_since(modified).unwrap_or_default() > FRESH {
        return None;
    }
    let seen_path = dir.join(SEEN);
    if std::fs::read_to_string(&seen_path).ok().is_some_and(|s| s.trim() == r.run_id) {
        return None;
    }
    std::fs::write(&seen_path, &r.run_id).ok()?;
    Some(r)
}

fn dir() -> PathBuf {
    crate::settings::local_dir()
}

pub fn last() -> Option<UpdateResult> {
    read_in(&dir())
}

pub fn take_notice() -> Option<UpdateResult> {
    take_notice_in(&dir(), SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glim-update-result-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const OK: &str = r#"{"runId":"2026-11-13_220000-123","commit":"1a2b3c4d","ok":true,"selftest":"6/6","failedStep":"","error":"","startedAt":"x","finishedAt":"2026-11-13T22:10:00","totalMs":600000,"steps":[{"name":"pack","ok":true,"ms":500000,"detail":"CARGO_BUILD_JOBS=2"}]}"#;

    #[test]
    fn reads_what_the_updater_writes_with_a_bom_and_unknown_fields() {
        let r = parse(format!("\u{feff}{OK}").as_bytes()).unwrap();
        assert_eq!((r.commit.as_str(), r.ok, r.selftest.as_str(), r.total_ms), ("1a2b3c4d", true, "6/6", 600000));
        assert_eq!(r.steps[0].name, "pack");
    }

    #[test]
    fn a_broken_or_idless_file_is_no_result() {
        assert!(parse(b"not json").is_none());
        assert!(parse(br#"{"commit":"1a2b3c4d","ok":true}"#).is_none());
    }

    #[test]
    fn long_or_control_text_is_clipped() {
        let long = "x".repeat(5000);
        let r = parse(format!(r#"{{"runId":"a","error":"{long}\n\u0007","commit":"{long}"}}"#).as_bytes()).unwrap();
        assert_eq!(r.error.len(), 200);
        assert_eq!(r.commit.len(), 12);
        assert!(!r.error.chars().any(|c| c.is_control()));
    }

    #[test]
    fn the_island_shows_each_run_once() {
        let d = tmp("once");
        std::fs::write(d.join(FILE), OK).unwrap();
        let now = SystemTime::now();
        assert_eq!(take_notice_in(&d, now).unwrap().commit, "1a2b3c4d");
        assert!(take_notice_in(&d, now).is_none(), "seen");
        // A new run (new id) shows again.
        std::fs::write(d.join(FILE), OK.replace("-123", "-456")).unwrap();
        assert!(take_notice_in(&d, now).is_some());
        // Settings still reads it.
        assert!(read_in(&d).is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_old_result_never_pops_up() {
        let d = tmp("old");
        std::fs::write(d.join(FILE), OK).unwrap();
        let later = SystemTime::now() + FRESH + Duration::from_secs(60);
        assert!(take_notice_in(&d, later).is_none());
        assert!(read_in(&d).is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_file_no_notice() {
        let d = tmp("none");
        assert!(take_notice_in(&d, SystemTime::now()).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
