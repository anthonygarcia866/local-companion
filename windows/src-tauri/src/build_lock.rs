// One Glim build at a time (owner 2026-11-15): windows/scripts/build-lock.mjs
// holds %LOCALAPPDATA%\Glim\build-lock\held while a glim-update or a release
// build (`npm run pack`) runs; one that starts meanwhile waits and leaves
// waiting-<pid>.json beside it. This reads both (local files only, a fixed
// shape: what, pid, since) so the island can say "glim-update waits for the
// release build to finish". Entries whose process is gone are ignored.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct Entry {
    what: String,
    pid: u32,
    since: u64,
}

/// What the island shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildWait {
    /// What holds the lock ("glim-update" / "release"), "" none.
    pub running: String,
    /// What waits for it, oldest first, with when it started waiting (ms).
    pub waiting: Vec<Waiter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiter {
    pub what: String,
    pub since: u64,
}

const WHATS: &[&str] = &["glim-update", "release"];

fn read(p: &Path) -> Option<Entry> {
    let b = std::fs::read(p).ok().filter(|b| b.len() < 4096)?;
    let e: Entry = serde_json::from_slice(&b).ok()?;
    WHATS.contains(&e.what.as_str()).then_some(e)
}

/// The lock folder's state; `alive` says whether a pid still runs.
pub fn status_in(dir: &Path, alive: &dyn Fn(u32) -> bool) -> BuildWait {
    let running = read(&dir.join("held").join("owner.json")).filter(|e| alive(e.pid)).map(|e| e.what).unwrap_or_default();
    let mut waiting: Vec<Waiter> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with("waiting-") && n.ends_with(".json")
                })
                .filter_map(|e| read(&e.path()))
                .filter(|e| alive(e.pid))
                .map(|e| Waiter { what: e.what, since: e.since })
                .collect()
        })
        .unwrap_or_default();
    waiting.sort_by_key(|w| w.since);
    BuildWait { running, waiting }
}

pub fn status() -> BuildWait {
    status_in(&crate::platform::local_dir().join("build-lock"), &crate::platform::pid_running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_build_shows_behind_the_running_one_and_gone_processes_dont() {
        let dir = std::env::temp_dir().join(format!("glim-build-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("held")).unwrap();
        assert_eq!(status_in(&dir, &|_| true), BuildWait::default());
        std::fs::write(dir.join("held").join("owner.json"), r#"{"what":"release","pid":10,"since":1,"token":"x"}"#).unwrap();
        std::fs::write(dir.join("waiting-20.json"), r#"{"what":"glim-update","pid":20,"since":5}"#).unwrap();
        std::fs::write(dir.join("waiting-30.json"), r#"{"what":"glim-update","pid":30,"since":3}"#).unwrap();
        std::fs::write(dir.join("waiting-40.json"), r#"{"what":"something-else","pid":40,"since":4}"#).unwrap();
        let s = status_in(&dir, &|p| p != 30);
        assert_eq!(s.running, "release");
        assert_eq!(s.waiting, vec![Waiter { what: "glim-update".into(), since: 5 }], "pid 30 is gone; unknown kinds are ignored");
        assert_eq!(status_in(&dir, &|p| p != 10).running, "", "a holder that's gone isn't running");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
