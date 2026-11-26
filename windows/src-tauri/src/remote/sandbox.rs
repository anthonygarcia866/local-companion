// The agent sandbox's quick check (owner 2026-11-26; docs/agent-sandbox-plan.md
// §7). Apps recreate folders (C:\OneDriveTemp lost its lock that way), so the
// sandbox setup.ps1 made is proven again, never assumed:
//   - when a lease is granted (exec::on_lease_answer): a failure goes to
//     Remote activity as `sandbox-check-failed`, with a phone ping;
//   - before Glim starts any glim-agent session (sandbox PR 6 calls `gate`,
//     whose refusal is `sandbox-check-failed`);
//   - once a day (`watch_daily`): the phone is pinged only on a failure.
//
// The check is scripts/agent-sandbox/quick-check.ps1 (with the authz.ps1 and
// targets.ps1 it needs), compiled INTO glim.exe: written to
// %LOCALAPPDATA%\Glim\sandbox-check\ (your profile, which glim-agent is denied)
// and read back byte for byte just before each run, then run by System32's
// Windows PowerShell, no console, not elevated, as you, `-Repair -Json`. It
// opens no socket (the firewall rules are read back locally). -Repair puts a
// lock back only where that's safe without elevation (see the script);
// anything else fails. What it prints is paths and reasons only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// The scripts, as built into this exe.
const SCRIPTS: &[(&str, &str)] = &[
    ("quick-check.ps1", include_str!("../../../../scripts/agent-sandbox/quick-check.ps1")),
    ("authz.ps1", include_str!("../../../../scripts/agent-sandbox/authz.ps1")),
    ("targets.ps1", include_str!("../../../../scripts/agent-sandbox/targets.ps1")),
];

/// The check's own limit (it takes ~3 s; elevated folders aren't walked).
const TIMEOUT: Duration = Duration::from_secs(120);
/// The daily check: this long between runs, the first a while after start.
pub const DAILY: Duration = Duration::from_secs(24 * 3600);
const FIRST_AFTER: Duration = Duration::from_secs(10 * 60);
/// Failures kept in a Remote activity summary (the rest counted).
const SHOWN: usize = 3;

/// What the check found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Outcome {
    pub ok: bool,
    /// False when there's no sandbox here at all (no glim-agent, no record).
    pub installed: bool,
    pub failures: Vec<String>,
    pub repaired: Vec<String>,
    pub unverified: Vec<String>,
}

impl Outcome {
    fn broken(why: &str) -> Self {
        Outcome { ok: false, installed: true, failures: vec![why.to_string()], ..Default::default() }
    }

    /// For Remote activity: how many problems, the first few (paths and
    /// reasons, on this PC only), and what was put back.
    pub fn summary(&self) -> String {
        let mut s = if self.ok { "passed".to_string() } else { format!("{} problem(s): ", self.failures.len()) };
        if !self.ok {
            let shown: Vec<&str> = self.failures.iter().take(SHOWN).map(String::as_str).collect();
            s.push_str(&shown.join("; "));
            if self.failures.len() > SHOWN {
                s.push_str(&format!("; and {} more", self.failures.len() - SHOWN));
            }
        }
        if !self.repaired.is_empty() {
            s.push_str(&format!(" ({} lock(s) put back)", self.repaired.len()));
        }
        s.chars().filter(|c| !c.is_control()).take(600).collect()
    }
}

/// The check's last JSON line, or a failure when it printed none.
pub fn parse(output: &[u8]) -> Outcome {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .and_then(|l| serde_json::from_str::<Outcome>(l).ok())
        .unwrap_or_else(|| Outcome::broken("the check gave no answer"))
}

/// Writes the built-in scripts to `dir` and reads each back: the exact bytes
/// this exe carries, or an error (nothing runs then).
pub fn install(dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|_| "couldn't create the check's folder".to_string())?;
    for (name, body) in SCRIPTS {
        let p = dir.join(name);
        // A UTF-8 BOM: Windows PowerShell 5.1 reads a BOM-less file as ANSI.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(body.as_bytes());
        std::fs::write(&p, &bytes).map_err(|_| format!("couldn't write {name}"))?;
        if std::fs::read(&p).ok().as_deref() != Some(bytes.as_slice()) {
            return Err(format!("{name} changed after it was written"));
        }
    }
    Ok(dir.join("quick-check.ps1"))
}

fn dir() -> PathBuf {
    crate::settings::local_dir().join("sandbox-check")
}

/// Runs the quick check now (with repair), on this thread.
#[cfg(windows)]
pub fn run() -> Outcome {
    let script = match install(&dir()) {
        Ok(p) => p,
        Err(e) => return Outcome::broken(&e),
    };
    match crate::platform::run_ps_script(&script, &["-Repair".into(), "-Json".into()], &dir(), TIMEOUT, None) {
        Ok((_, out)) => parse(&out),
        Err(_) => Outcome::broken("the check couldn't start"),
    }
}

#[cfg(not(windows))]
pub fn run() -> Outcome {
    Outcome::broken("the agent sandbox is Windows only")
}

/// Before a glim-agent session starts (sandbox PR 6): runs the check; a
/// failure is refused `sandbox-check-failed` and noted in Remote activity.
#[allow(dead_code)] // its caller is sandbox PR 6 (sessions as glim-agent)
pub fn gate(client: &str, now: i64) -> Result<(), super::Rejection> {
    let o = run();
    if o.ok {
        if !o.repaired.is_empty() {
            super::with(|g| g.note_lease(client, "sandbox-repaired", &o.summary(), now));
        }
        return Ok(());
    }
    super::with(|g| g.note_lease(client, "sandbox-check-failed", &o.summary(), now));
    Err(super::Rejection::SandboxCheckFailed)
}

static PING: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// The app's phone ping for a failed check (phone::Kind::SandboxCheck: a
/// fixed text, through Settings → Phone like every other ping).
pub fn set_ping(f: Box<dyn Fn() + Send + Sync>) {
    let _ = PING.set(f);
}

fn ping() {
    if let Some(f) = PING.get() {
        f();
    }
}

/// After a lease was granted: the check on a thread of its own; a failure
/// (or a lock put back) goes to Remote activity under the client's name, and
/// a failure also to the phone.
pub fn after_lease(client: String) {
    let _ = std::thread::Builder::new().name("sandbox-check".into()).spawn(move || {
        let o = run();
        let now = super::now_s();
        crate::log::line(format!("sandbox: lease check {}", if o.ok { "passed" } else { "FAILED" }));
        if !o.ok {
            super::with(|g| g.note_lease(&client, "sandbox-check-failed", &o.summary(), now));
            if daily_wants_ping(&o) {
                ping(); // a sandbox never set up: noted, no ping (its sessions are refused anyway)
            }
        } else if !o.repaired.is_empty() {
            super::with(|g| g.note_lease(&client, "sandbox-repaired", &o.summary(), now));
        }
    });
}

/// What the daily check does with a result: note it, and ping or not. Only a
/// sandbox that exists is reported; one never set up stays quiet.
pub fn daily_wants_ping(o: &Outcome) -> bool {
    o.installed && !o.ok
}

/// The daily check's thread: first ~10 minutes after Glim starts, then every
/// 24 hours. A failure: Remote activity + a phone ping (once per failing run).
pub fn watch_daily() {
    let _ = std::thread::Builder::new().name("sandbox-daily".into()).spawn(move || {
        std::thread::sleep(FIRST_AFTER);
        loop {
            let o = run();
            let now = super::now_s();
            if o.installed {
                crate::log::line(format!("sandbox: daily check {}", if o.ok { "passed" } else { "FAILED" }));
                if !o.ok {
                    super::with(|g| g.note_lease("", "sandbox-check-failed", &o.summary(), now));
                } else if !o.repaired.is_empty() {
                    super::with(|g| g.note_lease("", "sandbox-repaired", &o.summary(), now));
                }
            }
            if daily_wants_ping(&o) {
                ping();
            }
            std::thread::sleep(DAILY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_json_line() {
        let out = b"some warning\r\n{\"ok\":false,\"installed\":true,\"failures\":[\"C:\\\\x: writable\"],\"repaired\":[],\"unverified\":[],\"ms\":3}\r\n";
        let o = parse(out);
        assert!(!o.ok && o.installed);
        assert_eq!(o.failures, vec![r"C:\x: writable".to_string()]);
    }

    #[test]
    fn no_answer_is_a_failure() {
        for out in [&b""[..], b"garbage", b"{not json"] {
            let o = parse(out);
            assert!(!o.ok, "{out:?}");
            assert!(o.installed, "an unreadable answer must still be reported");
        }
    }

    #[test]
    fn an_ok_without_failures_field_is_still_only_what_it_says() {
        let o = parse(br#"{"ok":true,"installed":true}"#);
        assert!(o.ok && o.failures.is_empty());
    }

    #[test]
    fn daily_pings_only_on_a_failure_of_an_existing_sandbox() {
        assert!(!daily_wants_ping(&Outcome { ok: true, installed: true, ..Default::default() }));
        assert!(!daily_wants_ping(&Outcome { ok: false, installed: false, ..Default::default() }));
        assert!(daily_wants_ping(&Outcome { ok: false, installed: true, ..Default::default() }));
    }

    #[test]
    fn summary_is_short_and_counts_the_rest() {
        let o = Outcome { ok: false, installed: true, failures: (0..5).map(|i| format!("p{i}")).collect(), repaired: vec!["r".into()], unverified: vec![] };
        let s = o.summary();
        assert!(s.starts_with("5 problem(s): p0; p1; p2; and 2 more"), "{s}");
        assert!(s.ends_with("(1 lock(s) put back)"), "{s}");
        let long = Outcome { failures: vec!["x\n".repeat(1000)], ..Default::default() };
        assert!(long.summary().chars().count() <= 600 && !long.summary().contains('\n'));
    }

    #[test]
    fn install_writes_exactly_the_built_in_scripts() {
        let d = std::env::temp_dir().join(format!("glim-sandbox-test-{}", std::process::id()));
        let p = install(&d).unwrap();
        assert!(p.ends_with("quick-check.ps1"));
        for (name, body) in SCRIPTS {
            let b = std::fs::read(d.join(name)).unwrap();
            assert_eq!(&b[..3], &[0xEF, 0xBB, 0xBF]);
            assert_eq!(&b[3..], body.as_bytes(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_built_in_check_repairs_only_safely_and_never_elevates() {
        let q = SCRIPTS[0].1;
        assert!(q.contains("[GlimFs]::IsLink($p)"), "repair skips links");
        assert!(q.contains("AddDeny"), "repair uses the non-propagating writer");
        assert!(!q.contains("icacls"), "never icacls (it re-stamps whole trees)");
        assert!(!q.contains("RunAs") && !q.contains("-Verb"), "never elevates");
        for net in ["Invoke-WebRequest", "Invoke-RestMethod", "Net.WebClient", "TcpClient", "curl"] {
            assert!(!q.contains(net), "no network in the check: {net}");
        }
    }
}
