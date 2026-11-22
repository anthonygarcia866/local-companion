// Remote runbook runs and their checkpoints (owner 2026-11-20, D2,
// docs/remote-runbooks.md §3.3).
//
// A remote run that reaches a level-2 ("ask on my phone") step pauses there:
// its checkpoint (run id, runbook, the approved hash it started under, steps
// done, client, times) is kept in %LOCALAPPDATA%\Glim\remote-runs.json, with
// no output and no secret values. On the owner's approval it resumes at that
// step under the SAME hash (else `resume-changed`); no answer in 24 h and it
// ends `expired`; the kill switch ends it `killed`. A paused run counts as
// the one remote run at a time. `run-status` shows a client its own runs.

use serde::{Deserialize, Serialize};

use crate::runbooks::{Refusal, RemoteEnd, RemoteSegment};

/// How long a paused run waits for the owner's answer.
pub const PAUSE_TTL_S: i64 = 24 * 3600;
/// Runs kept (newest), finished or not.
pub const KEPT: usize = 50;
/// Runs one run-status answer lists, newest first.
pub const LISTED: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    #[default]
    Running,
    Resumed,
    Paused,
    Done,
    Failed,
    Declined,
    Expired,
    Killed,
    ResumeChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// 32 lower-case hex characters.
    pub id: String,
    pub name: String,
    /// The cloud client that started it: the only one run-status shows it to.
    pub client: String,
    /// The runbook's approved hash when the run started.
    pub hash: String,
    pub total: usize,
    /// Steps that ran OK so far.
    pub done: usize,
    pub state: RunState,
    /// The step (1-based) it is paused, failed, declined or expired at; 0 none.
    pub step: usize,
    /// The failed step's kind ("run" …), or a refusal code; "" none.
    pub kind: String,
    /// Unix seconds.
    pub started: i64,
    pub updated: i64,
    /// While paused: when it expires (Unix seconds); 0 otherwise.
    pub expires: i64,
    /// The paused step's kind ("run" …).
    #[serde(default)]
    pub step_kind: String,
    /// While paused: the phone approval request's id (32 hex), "" none.
    #[serde(default)]
    pub approval: String,
    /// Its match code ("ABC-DEF").
    #[serde(default)]
    pub code: String,
    /// Its approval link (the page, the request id, the relay host sealed
    /// per approver): run-status shows it while paused (owner 2026-11-21,
    /// the approval doorbell), so an agent can forward it.
    #[serde(default)]
    pub url: String,
}

impl Run {
    /// The status line Muse / cloud Glim reads.
    pub fn status(&self) -> String {
        let of = self.total;
        match self.state {
            RunState::Running => format!("running (step {} of {of})", (self.done + 1).min(of.max(1))),
            RunState::Resumed => format!("resumed (step {} of {of})", (self.done + 1).min(of.max(1))),
            RunState::Paused => format!("paused at step {} of {of}", self.step),
            RunState::Done => "done".into(),
            RunState::Failed => format!("failed at step {} of {of} ({})", self.step, self.kind),
            RunState::Declined => format!("declined at step {} of {of}", self.step),
            RunState::Expired => format!("expired at step {} of {of}", self.step),
            RunState::Killed => "killed".into(),
            RunState::ResumeChanged => "resume-changed".into(),
        }
    }

    fn open(&self) -> bool {
        matches!(self.state, RunState::Running | RunState::Resumed | RunState::Paused)
    }
}

/// A new run id: 16 bytes of the system's randomness, hex.
pub fn new_id() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).map_err(|_| "no randomness".to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// A run id as run-status accepts it: 32 lower-case hex characters.
pub fn id_ok(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Debug, Default)]
pub struct Runs {
    pub runs: Vec<Run>,
    path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    runs: Vec<Run>,
}

impl Runs {
    /// The runs kept in `path`. A run still running when Glim stopped can't
    /// be resumed (its step may or may not have finished): it ends failed,
    /// "interrupted". Paused runs keep waiting.
    pub fn load(path: std::path::PathBuf, now: i64) -> Runs {
        let saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut r = Runs { runs: saved.runs, path: Some(path) };
        for run in r.runs.iter_mut().filter(|x| matches!(x.state, RunState::Running | RunState::Resumed)) {
            run.state = RunState::Failed;
            run.step = (run.done + 1).min(run.total);
            run.kind = "interrupted".into();
            run.updated = now;
        }
        r.save();
        r
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::settings::local_dir().join("remote-runs.json")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Ok(bytes) = serde_json::to_vec(&Saved { runs: self.runs.clone() }) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Paused runs past their 24 h end `expired`.
    pub fn sweep(&mut self, now: i64) {
        let mut changed = false;
        for run in self.runs.iter_mut().filter(|x| x.state == RunState::Paused && now > x.expires) {
            run.state = RunState::Expired;
            run.expires = 0;
            run.updated = now;
            changed = true;
            crate::log::line("remote: runbook run expired");
        }
        if changed {
            self.save();
        }
    }

    /// A run is running or paused: another remote run must wait.
    pub fn busy(&mut self, now: i64) -> bool {
        self.sweep(now);
        self.runs.iter().any(Run::open)
    }

    pub fn get(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// The kill switch was used on this run (exec's stop check between steps).
    pub fn killed(&self, id: &str) -> bool {
        self.get(id).is_some_and(|r| r.state == RunState::Killed)
    }

    /// `client`'s own runs, newest first (or just `id`, if it's theirs).
    pub fn for_client(&mut self, client: &str, id: Option<&str>, now: i64) -> Vec<Run> {
        self.sweep(now);
        self.runs.iter().rev().filter(|r| r.client == client && id.is_none_or(|i| r.id == i)).take(LISTED).cloned().collect()
    }

    fn push(&mut self, run: Run) {
        self.runs.push(run);
        let extra = self.runs.len().saturating_sub(KEPT);
        // Drop the oldest finished ones first; an open run is never dropped.
        for _ in 0..extra {
            if let Some(i) = self.runs.iter().position(|r| !r.open()) {
                self.runs.remove(i);
            }
        }
    }

    /// The kill switch: every open run (paused, or running a step now) ends
    /// `killed`. A run in the middle of a step stops before its next one
    /// (`killed`, exec's stop check) and is never paused or resumed after.
    /// Returns the approval requests it voided (the relay drops them too).
    pub fn kill_all(&mut self, now: i64) -> Vec<String> {
        let mut voided = Vec::new();
        for run in self.runs.iter_mut().filter(|r| r.open()) {
            run.state = RunState::Killed;
            run.expires = 0;
            run.updated = now;
            if !run.approval.is_empty() {
                voided.push(std::mem::take(&mut run.approval));
            }
        }
        self.save();
        voided
    }

    /// `client`'s open runs end `killed` (its lease was locked or the client
    /// removed: review 2026-11-21). Returns the approval requests voided.
    pub fn kill_client(&mut self, client: &str, now: i64) -> Vec<String> {
        let mut voided = Vec::new();
        for run in self.runs.iter_mut().filter(|r| r.open() && r.client == client) {
            run.state = RunState::Killed;
            run.expires = 0;
            run.updated = now;
            run.url.clear();
            if !run.approval.is_empty() {
                voided.push(std::mem::take(&mut run.approval));
            }
        }
        self.save();
        voided
    }

    /// A paused run now waits on approval request `rid` (code `code`).
    pub fn set_approval(&mut self, id: &str, rid: &str, code: &str, url: &str) {
        if let Some(run) = self.runs.iter_mut().find(|r| r.id == id && r.state == RunState::Paused) {
            run.approval = rid.into();
            run.code = code.into();
            run.url = url.into();
            self.save();
        }
    }

    /// The paused, unexpired run waiting on request `rid`, its request
    /// used up (single use): whatever the answer, it can't be answered again.
    pub fn take_approval(&mut self, rid: &str, now: i64) -> Option<Run> {
        self.sweep(now);
        let run = self.runs.iter_mut().find(|r| !rid.is_empty() && r.approval == rid && r.state == RunState::Paused)?;
        let out = run.clone();
        run.approval.clear();
        run.url.clear();
        self.save();
        Some(out)
    }

    /// Records how a segment ended.
    fn finish(&mut self, id: &str, seg: &RemoteSegment, now: i64) -> Option<Run> {
        let run = self.runs.iter_mut().find(|r| r.id == id)?;
        run.done += seg.ran_ok;
        run.total = seg.total;
        run.updated = now;
        run.expires = 0;
        // Killed while a step ran: it stays killed, never paused or done.
        if run.state == RunState::Killed {
            let out = run.clone();
            self.save();
            return Some(out);
        }
        match &seg.end {
            RemoteEnd::Done => {
                run.state = RunState::Done;
                run.step = 0;
            }
            RemoteEnd::Failed { step, kind } => {
                run.state = RunState::Failed;
                run.step = *step;
                run.kind = (*kind).into();
            }
            RemoteEnd::Paused { step } => {
                run.state = RunState::Paused;
                run.step = *step;
                run.step_kind = seg.paused_kind.into();
                run.expires = now + PAUSE_TTL_S;
                run.approval.clear();
            }
            RemoteEnd::Stopped { .. } => run.state = RunState::Killed,
        }
        let out = run.clone();
        self.save();
        Some(out)
    }

    /// Records a new remote run before its first step (busy if another run
    /// is running or paused).
    pub fn begin(&mut self, id: &str, name: &str, client: &str, now: i64) -> Result<(), String> {
        if self.busy(now) {
            return Err("runbook-busy".into());
        }
        self.push(Run { id: id.into(), name: name.into(), client: client.into(), state: RunState::Running, started: now, updated: now, ..Run::default() });
        self.save();
        Ok(())
    }

    /// How the first segment went. Refused before step 1 (not remote-allowed,
    /// not approved …): no run is kept, and the code comes back.
    pub fn began(&mut self, id: &str, r: Result<RemoteSegment, Refusal>, now: i64) -> Result<Run, String> {
        match r {
            Err(e) => {
                self.runs.retain(|x| x.id != id);
                self.save();
                Err(format!("runbook-{}", e.code()))
            }
            Ok(seg) => {
                if let Some(run) = self.runs.iter_mut().find(|x| x.id == id) {
                    run.hash = seg.hash.clone();
                }
                self.finish(id, &seg, now).ok_or_else(|| "runbook-lost".into())
            }
        }
    }

    /// A paused run about to resume (PR 3 calls it once the phone's
    /// signature verified): (runbook, the hash it started under, the step
    /// to resume at, 0-based). Expired, killed or finished runs don't.
    pub fn begin_resume(&mut self, id: &str, now: i64) -> Result<(String, String, usize), String> {
        self.sweep(now);
        let run = self.runs.iter_mut().find(|r| r.id == id).ok_or("no-such-run")?;
        if run.state != RunState::Paused {
            return Err(format!("run-{}", run.status().split(' ').next().unwrap_or("not-paused")));
        }
        run.state = RunState::Resumed;
        run.expires = 0;
        run.updated = now;
        let out = (run.name.clone(), run.hash.clone(), run.step - 1);
        self.save();
        Ok(out)
    }

    /// How a resume went: a changed runbook ends `resume-changed` and
    /// nothing more ran.
    pub fn resumed(&mut self, id: &str, r: Result<RemoteSegment, Refusal>, now: i64) -> Result<Run, String> {
        match r {
            Err(e) => {
                let run = self.runs.iter_mut().find(|x| x.id == id).ok_or("no-such-run")?;
                run.updated = now;
                if run.state == RunState::Killed {
                    // Killed meanwhile: stays killed.
                } else if e == Refusal::Changed {
                    run.state = RunState::ResumeChanged;
                } else {
                    run.state = RunState::Failed;
                    run.step = run.done + 1;
                    run.kind = e.code().into();
                }
                let out = run.clone();
                self.save();
                Ok(out)
            }
            Ok(seg) => self.finish(id, &seg, now).ok_or_else(|| "no-such-run".into()),
        }
    }

    /// `begin` + run + `began` in one (tests; the app runs the runbook
    /// outside the lock: remote::exec).
    pub fn start_with(&mut self, id: &str, name: &str, client: &str, now: i64, runner: impl FnOnce(&str, Option<&str>, usize, bool) -> Result<RemoteSegment, Refusal>) -> Result<Run, String> {
        self.begin(id, name, client, now)?;
        let r = runner(name, None, 0, false);
        self.began(id, r, now)
    }

    /// `begin_resume` + run + `resumed` in one (tests).
    pub fn resume_with(&mut self, id: &str, now: i64, runner: impl FnOnce(&str, Option<&str>, usize, bool) -> Result<RemoteSegment, Refusal>) -> Result<Run, String> {
        let (name, hash, from) = self.begin_resume(id, now)?;
        let r = runner(&name, Some(&hash), from, true);
        self.resumed(id, r, now)
    }

    /// An approval answer that didn't verify: the paused run ends failed
    /// with that code (its request is used up; nothing more runs).
    pub fn refuse(&mut self, id: &str, code: &str, now: i64) -> Result<Run, String> {
        self.sweep(now);
        let run = self.runs.iter_mut().find(|r| r.id == id && r.state == RunState::Paused).ok_or("not-paused")?;
        run.state = RunState::Failed;
        run.kind = code.into();
        run.expires = 0;
        run.updated = now;
        let out = run.clone();
        self.save();
        Ok(out)
    }

    /// The owner's Stop on the phone (PR 3): the paused run ends `declined`.
    pub fn decline(&mut self, id: &str, now: i64) -> Result<Run, String> {
        self.sweep(now);
        let run = self.runs.iter_mut().find(|r| r.id == id && r.state == RunState::Paused).ok_or("not-paused")?;
        run.state = RunState::Declined;
        run.expires = 0;
        run.updated = now;
        let out = run.clone();
        self.save();
        Ok(out)
    }
}

/// UTC "2026-11-20T21:26:35Z".
pub fn utc(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// One run-status line: id, name, status, times. Never output or secrets.
pub fn line(r: &Run) -> String {
    let mut s = format!("{} {}: {} (started {}, updated {})", r.id, r.name, r.status(), utc(r.started), utc(r.updated));
    if r.state == RunState::Paused {
        s.push_str(&format!(", waits until {}", utc(r.expires)));
        // The approval doorbell (owner 2026-11-21): the link and match code
        // to forward to the owner while the request is open. The link
        // carries only the request id and the relay host sealed to the
        // approvers; approving still needs the owner's Face ID.
        if !r.approval.is_empty() && !r.url.is_empty() {
            s.push_str(&format!("; approve at {} · match code {}", r.url, r.code));
        }
    }
    s
}

/// The process's runs, loaded on first use.
pub fn with<R>(f: impl FnOnce(&mut Runs) -> R) -> R {
    static RUNS: std::sync::Mutex<Option<Runs>> = std::sync::Mutex::new(None);
    let mut g = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    let runs = g.get_or_insert_with(|| Runs::load(Runs::default_path(), super::now_s()));
    f(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_238_795;
    const ID: &str = "0123456789abcdef0123456789abcdef";
    const ID2: &str = "fedcba9876543210fedcba9876543210";

    fn seg(ran_ok: usize, end: RemoteEnd) -> RemoteSegment {
        RemoteSegment { hash: "h1".into(), total: 5, ran_ok, end, paused_kind: "run" }
    }

    #[test]
    fn a_run_that_reaches_a_phone_step_pauses_and_resumes_there_without_redoing_steps() {
        let mut r = Runs::default();
        let run = r.start_with(ID, "glim-check", "account-1", NOW, |n, h, from, ok| {
            assert_eq!((n, h, from, ok), ("glim-check", None, 0, false));
            Ok(seg(2, RemoteEnd::Paused { step: 3 }))
        }).unwrap();
        assert_eq!(run.status(), "paused at step 3 of 5");
        assert_eq!(run.expires, NOW + PAUSE_TTL_S);
        assert!(r.busy(NOW + 10), "a paused run is the one remote run");
        assert_eq!(r.start_with(ID2, "other", "account-1", NOW + 10, |_, _, _, _| panic!("must not run")), Err("runbook-busy".into()));
        let resumed = r.resume_with(ID, NOW + 60, |n, h, from, ok| {
            assert_eq!((n, h, from, ok), ("glim-check", Some("h1"), 2, true), "at step 3, under the hash it started with, approved");
            Ok(seg(3, RemoteEnd::Done))
        }).unwrap();
        assert_eq!(resumed.status(), "done");
        assert_eq!(resumed.done, 5);
        assert!(!r.busy(NOW + 61));
        assert!(r.resume_with(ID, NOW + 70, |_, _, _, _| panic!("must not run")).is_err(), "a finished run doesn't resume");
    }

    #[test]
    fn a_changed_runbook_never_resumes() {
        let mut r = Runs::default();
        r.start_with(ID, "glim-check", "account-1", NOW, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
        let out = r.resume_with(ID, NOW + 5, |_, _, _, _| Err(Refusal::Changed)).unwrap();
        assert_eq!(out.status(), "resume-changed");
        assert_eq!(out.done, 2);
        assert!(!r.busy(NOW + 6));
    }

    #[test]
    fn no_answer_in_24_hours_expires_and_the_kill_switch_ends_paused_runs() {
        let mut r = Runs::default();
        r.start_with(ID, "glim-check", "account-1", NOW, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
        assert!(r.busy(NOW + PAUSE_TTL_S));
        assert!(!r.busy(NOW + PAUSE_TTL_S + 1));
        assert_eq!(r.get(ID).unwrap().status(), "expired at step 3 of 5");
        assert_eq!(r.resume_with(ID, NOW + PAUSE_TTL_S + 2, |_, _, _, _| panic!("must not run")), Err("run-expired".into()));
        r.start_with(ID2, "glim-check", "account-1", NOW + PAUSE_TTL_S + 3, |_, _, _, _| Ok(seg(0, RemoteEnd::Paused { step: 1 }))).unwrap();
        r.kill_all(NOW + PAUSE_TTL_S + 4);
        assert_eq!(r.get(ID2).unwrap().status(), "killed");
    }

    #[test]
    fn a_kill_while_a_step_runs_is_never_undone_by_the_pause_that_follows() {
        let mut r = Runs::default();
        r.begin(ID, "glim-check", "account-1", NOW).unwrap();
        r.kill_all(NOW + 1);
        assert!(r.killed(ID), "exec's stop check sees it before the next step");
        let out = r.began(ID, Ok(seg(1, RemoteEnd::Paused { step: 3 })), NOW + 2).unwrap();
        assert_eq!(out.status(), "killed");
        assert!(!r.busy(NOW + 3));
        assert!(r.begin_resume(ID, NOW + 4).is_err(), "a killed run never resumes");
        // Killed during a resumed segment too.
        r.start_with(ID2, "glim-check", "account-1", NOW + 5, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
        r.begin_resume(ID2, NOW + 6).unwrap();
        r.kill_all(NOW + 7);
        assert_eq!(r.resumed(ID2, Ok(seg(1, RemoteEnd::Paused { step: 5 })), NOW + 8).unwrap().status(), "killed");
        assert_eq!(r.resumed(ID2, Err(Refusal::Changed), NOW + 9).unwrap().status(), "killed");
        // A segment the stop check ended is killed as well.
        r.start_with(ID, "glim-check", "account-1", NOW + 10, |_, _, _, _| Ok(seg(1, RemoteEnd::Stopped { step: 2 }))).unwrap();
        assert_eq!(r.get(ID).unwrap().status(), "killed");
    }

    #[test]
    fn refused_before_step_one_keeps_no_run_and_failures_name_the_step_kind() {
        let mut r = Runs::default();
        assert_eq!(r.start_with(ID, "glim-check", "account-1", NOW, |_, _, _, _| Err(Refusal::NotRemote)), Err("runbook-not-remote".into()));
        assert!(r.runs.is_empty());
        let f = r.start_with(ID2, "glim-check", "account-1", NOW, |_, _, _, _| Ok(seg(1, RemoteEnd::Failed { step: 2, kind: "run" }))).unwrap();
        assert_eq!(f.status(), "failed at step 2 of 5 (run)");
    }

    #[test]
    fn a_client_sees_only_its_own_runs_and_a_stop_declines() {
        let mut r = Runs::default();
        r.start_with(ID, "glim-check", "account-1", NOW, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
        assert_eq!(r.for_client("account-1", None, NOW + 1).len(), 1);
        assert!(r.for_client("account-2", None, NOW + 1).is_empty());
        assert!(r.for_client("account-2", Some(ID), NOW + 1).is_empty());
        assert_eq!(r.decline(ID, NOW + 2).unwrap().status(), "declined at step 3 of 5");
        let l = line(r.get(ID).unwrap());
        assert!(l.starts_with(&format!("{ID} glim-check: declined at step 3 of 5 (started ")), "{l}");
    }

    #[test]
    fn a_run_interrupted_by_a_restart_ends_failed_and_paused_ones_keep_waiting() {
        let dir = std::env::temp_dir().join(format!("glim-runs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("remote-runs.json");
        {
            let mut r = Runs::load(path.clone(), NOW);
            r.start_with(ID, "a", "account-1", NOW, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
            r.runs.push(Run { id: ID2.into(), name: "b".into(), client: "account-1".into(), hash: "h".into(), total: 4, done: 1, state: RunState::Running, step: 0, kind: String::new(), started: NOW, updated: NOW, expires: 0, ..Run::default() });
            r.save();
        }
        let r = Runs::load(path.clone(), NOW + 5);
        assert_eq!(r.get(ID).unwrap().state, RunState::Paused);
        assert_eq!(r.get(ID2).unwrap().status(), "failed at step 2 of 4 (interrupted)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_approval_request_is_single_use_and_the_kill_switch_voids_it() {
        let mut r = Runs::default();
        r.start_with(ID, "glim-check", "account-1", NOW, |_, _, _, _| Ok(seg(2, RemoteEnd::Paused { step: 3 }))).unwrap();
        assert_eq!(r.get(ID).unwrap().step_kind, "run");
        r.set_approval(ID, ID2, "ABC-DEF", "https://example.invalid/#r=x");
        assert_eq!(r.take_approval(ID2, NOW + 1).unwrap().code, "ABC-DEF");
        assert!(r.take_approval(ID2, NOW + 2).is_none(), "used up");
        assert!(r.take_approval("", NOW + 2).is_none());
        r.set_approval(ID, ID2, "ABC-DEF", "https://example.invalid/#r=x");
        assert_eq!(r.kill_all(NOW + 3), vec![ID2.to_string()]);
        assert!(r.take_approval(ID2, NOW + 4).is_none(), "voided");
        // Expired with the run.
        r.start_with("aa".repeat(16).as_str(), "x", "account-1", NOW + 5, |_, _, _, _| Ok(seg(0, RemoteEnd::Paused { step: 1 }))).unwrap();
        r.set_approval(&"aa".repeat(16), &"bb".repeat(16), "XYZ-234", "https://example.invalid/#r=y");
        assert!(r.take_approval(&"bb".repeat(16), NOW + 5 + PAUSE_TTL_S + 1).is_none());
    }

    #[test]
    fn ids_and_times() {
        let id = new_id().unwrap();
        assert!(id_ok(&id) && id_ok(ID));
        for bad in ["", "0123", "0123456789ABCDEF0123456789ABCDEF", "../../0123456789abcdef0123456789"] {
            assert!(!id_ok(bad), "{bad}");
        }
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_795_238_795), "2026-11-21T05:26:35Z");
    }
}
