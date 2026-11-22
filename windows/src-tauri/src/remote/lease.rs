// Leases for agent clients (owner 2026-11-21, design item 3; docs/remote-courier.md
// "Leases"). An agent client (CloudClient::agent, e.g. Muse) is LOCKED by
// default: it may send only the commands ticked for it, plus `lease-request`.
// A lease, granted only by a Face ID approval on the owner's phone, unlocks
// it for 1, 4, 8 or 12 hours: then it may send all seven commands (session
// commands still only inside the agent sandbox, which doesn't exist yet:
// `no-sandbox`). A lease ends at its expiry, on the kill switch, or with
// "Lock now" (Settings → Remote, or the phone page's lock link); it is never
// renewed by itself. When it ends, a summary of what the client did goes to
// Remote activity (kinds, counts, runbook names; never arguments).
//
// The agent can only ASK: `lease-request` answers with the approval link and
// match code, which it forwards to the owner. Granting needs the owner's
// passkey signature over exactly that request (approve::lease_binding),
// checked here on the PC.
//
// Kept in %LOCALAPPDATA%\Glim\remote-leases.json: client names, hours, times,
// request ids, match codes and usage counts. No keys, no arguments.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The lengths a lease may have (hours); the first is the default.
pub const HOURS: &[u32] = &[1, 4, 8, 12];
/// A lease request waits this long for the owner's answer.
pub const REQUEST_TTL_S: i64 = 30 * 60;
/// Runbook names kept in a lease's summary (the rest only counted).
const RUNBOOKS_KEPT: usize = 10;

/// What the client did while leased: command kinds and counts, runbook
/// names, sessions started or continued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub commands: BTreeMap<String, u32>,
    pub runbooks: Vec<String>,
    pub sessions: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lease {
    pub client: String,
    pub hours: u32,
    /// Unix seconds.
    pub granted: i64,
    pub until: i64,
    /// The lock slot's id at the relay (32 hex): any answer to it locks now.
    pub lock: String,
    pub usage: Usage,
}

/// A lease request waiting for the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pending {
    /// The approval request's id (32 hex).
    pub rid: String,
    pub code: String,
    pub client: String,
    pub hours: u32,
    /// When the request lapses (Unix seconds).
    pub expires: i64,
    /// The approval page's link (request id + the relay host sealed per approver).
    pub url: String,
    /// The lock slot the lease will use once granted.
    pub lock: String,
}

/// Why a lease ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Expired,
    Killed,
    /// "Lock now" in Settings.
    Locked,
    /// The phone page's lock link.
    LockedPhone,
    /// A new lease replaced it (the owner approved another request).
    Replaced,
    /// The client was removed.
    Removed,
}

impl Ended {
    pub fn code(self) -> &'static str {
        match self {
            Ended::Expired => "expired",
            Ended::Killed => "killed",
            Ended::Locked => "locked",
            Ended::LockedPhone => "locked-phone",
            Ended::Replaced => "replaced",
            Ended::Removed => "removed",
        }
    }
}

#[derive(Debug, Default)]
pub struct Leases {
    pub active: Vec<Lease>,
    pub pending: Vec<Pending>,
    path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    active: Vec<Lease>,
    #[serde(default)]
    pending: Vec<Pending>,
}

/// The lease length asked for: one of HOURS (default the first).
pub fn parse_hours(v: Option<u64>) -> Option<u32> {
    match v {
        None => Some(HOURS[0]),
        Some(h) => HOURS.iter().copied().find(|x| u64::from(*x) == h),
    }
}

impl Leases {
    pub fn load(path: std::path::PathBuf) -> Leases {
        let saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Leases { active: saved.active, pending: saved.pending, path: Some(path) }
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::settings::local_dir().join("remote-leases.json")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Ok(bytes) = serde_json::to_vec(&Saved { active: self.active.clone(), pending: self.pending.clone() }) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Leases past their end, and requests past theirs, go. Returns the
    /// leases that ended (for their summaries).
    pub fn sweep(&mut self, now: i64) -> Vec<Lease> {
        let before = (self.active.len(), self.pending.len());
        let (gone, kept): (Vec<Lease>, Vec<Lease>) = std::mem::take(&mut self.active).into_iter().partition(|l| now >= l.until);
        self.active = kept;
        self.pending.retain(|p| now < p.expires);
        if before != (self.active.len(), self.pending.len()) {
            self.save();
        }
        gone
    }

    /// `client` holds an unexpired lease (an expired one isn't swept here:
    /// the courier's sweep reports it).
    pub fn active_for(&self, client: &str, now: i64) -> Option<&Lease> {
        self.active.iter().find(|l| !client.is_empty() && l.client == client && now < l.until)
    }

    /// A new request from `client`: it replaces the client's earlier one
    /// (whose slot the caller voids: returned).
    pub fn request(&mut self, p: Pending) -> Option<Pending> {
        let old = self.pending.iter().position(|x| x.client == p.client).map(|i| self.pending.remove(i));
        self.pending.push(p);
        self.save();
        old
    }

    /// The unexpired request `rid`, used up (single use) whatever the answer.
    pub fn take_pending(&mut self, rid: &str, now: i64) -> Option<Pending> {
        let i = self.pending.iter().position(|p| !rid.is_empty() && p.rid == rid)?;
        let p = self.pending.remove(i);
        self.save();
        (now < p.expires).then_some(p)
    }

    /// The owner approved: the lease starts now. A lease the client already
    /// held ends `replaced` (returned, for its summary).
    pub fn grant(&mut self, p: &Pending, now: i64) -> Option<Lease> {
        let old = self.active.iter().position(|l| l.client == p.client).map(|i| self.active.remove(i));
        self.active.push(Lease { client: p.client.clone(), hours: p.hours, granted: now, until: now + i64::from(p.hours) * 3600, lock: p.lock.clone(), usage: Usage::default() });
        self.save();
        old
    }

    /// The client whose lease lock slot is `rid`.
    pub fn lock_slot_client(&self, rid: &str) -> Option<String> {
        self.active.iter().find(|l| !rid.is_empty() && l.lock == rid).map(|l| l.client.clone())
    }

    /// `client`'s lease ends now (and any request it has waiting goes).
    pub fn end(&mut self, client: &str) -> Option<Lease> {
        self.pending.retain(|p| p.client != client);
        let i = self.active.iter().position(|l| l.client == client);
        let out = i.map(|i| self.active.remove(i));
        self.save();
        out
    }

    /// The kill switch: every lease and request ends. Returns the leases
    /// (for their summaries) and every slot to void at the relay.
    pub fn end_all(&mut self) -> (Vec<Lease>, Vec<String>) {
        let mut slots: Vec<String> = self.pending.drain(..).map(|p| p.rid).collect();
        let leases: Vec<Lease> = self.active.drain(..).collect();
        slots.extend(leases.iter().map(|l| l.lock.clone()).filter(|s| !s.is_empty()));
        self.save();
        (leases, slots)
    }

    /// An accepted command of `client`'s, counted on its lease (if any).
    pub fn note(&mut self, client: &str, kind: &str, runbook: &str, now: i64) {
        let Some(l) = self.active.iter_mut().find(|l| l.client == client && now < l.until) else { return };
        *l.usage.commands.entry(kind.to_string()).or_default() += 1;
        if !runbook.is_empty() && l.usage.runbooks.len() < RUNBOOKS_KEPT && !l.usage.runbooks.iter().any(|r| r == runbook) {
            l.usage.runbooks.push(runbook.to_string());
        }
        if matches!(kind, "start-session" | "send-to-session") {
            l.usage.sessions += 1;
        }
        self.save();
    }
}

/// A lease's end, for Remote activity: what it was and what the client did.
/// Kinds, counts and runbook names only.
pub fn summary(l: &Lease, why: Ended) -> String {
    let cmds = if l.usage.commands.is_empty() {
        "no commands".to_string()
    } else {
        l.usage.commands.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    };
    let books = if l.usage.runbooks.is_empty() { String::new() } else { format!("; runbooks: {}", l.usage.runbooks.join(", ")) };
    format!("{} h lease ended ({}): {cmds}{books}; sessions {}; PRs not tracked (no sandbox yet)", l.hours, why.code(), l.usage.sessions)
}

/// The process's leases, loaded on first use.
pub fn with<R>(f: impl FnOnce(&mut Leases) -> R) -> R {
    static LEASES: std::sync::Mutex<Option<Leases>> = std::sync::Mutex::new(None);
    let mut g = LEASES.lock().unwrap_or_else(|e| e.into_inner());
    let l = g.get_or_insert_with(|| Leases::load(Leases::default_path()));
    f(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_238_795;

    fn pending(client: &str, rid: &str, hours: u32) -> Pending {
        Pending { rid: rid.into(), code: "ABC-DEF".into(), client: client.into(), hours, expires: NOW + REQUEST_TTL_S, url: "https://example.invalid/#r".into(), lock: format!("{rid}-lock") }
    }

    #[test]
    fn hours_are_one_of_the_listed_lengths_default_one() {
        assert_eq!(parse_hours(None), Some(1));
        for h in [1, 4, 8, 12] {
            assert_eq!(parse_hours(Some(h)), Some(h as u32));
        }
        for h in [0, 2, 13, 24, 1000] {
            assert_eq!(parse_hours(Some(h)), None);
        }
    }

    #[test]
    fn a_lease_exists_only_after_a_granted_request_and_ends_at_its_hour() {
        let mut l = Leases::default();
        assert!(l.active_for("muse", NOW).is_none(), "locked by default");
        l.request(pending("muse", "r1", 4));
        assert!(l.active_for("muse", NOW).is_none(), "a request is not a lease");
        let p = l.take_pending("r1", NOW + 10).unwrap();
        assert!(l.take_pending("r1", NOW + 11).is_none(), "single use");
        assert!(l.grant(&p, NOW + 10).is_none());
        assert_eq!(l.active_for("muse", NOW + 10).unwrap().until, NOW + 10 + 4 * 3600);
        assert!(l.active_for("bot", NOW + 10).is_none(), "only that client");
        assert!(l.active_for("", NOW + 10).is_none());
        assert!(l.active_for("muse", NOW + 10 + 4 * 3600).is_none(), "not one second past the end");
        let gone = l.sweep(NOW + 10 + 4 * 3600);
        assert_eq!(gone.len(), 1);
        assert!(l.active.is_empty());
    }

    #[test]
    fn a_late_answer_finds_no_request_and_a_new_request_replaces_the_old() {
        let mut l = Leases::default();
        l.request(pending("muse", "r1", 1));
        assert!(l.take_pending("r1", NOW + REQUEST_TTL_S).is_none(), "lapsed");
        l.request(pending("muse", "r2", 1));
        let old = l.request(pending("muse", "r3", 8)).unwrap();
        assert_eq!(old.rid, "r2");
        assert!(l.take_pending("r2", NOW).is_none(), "replaced");
        assert_eq!(l.take_pending("r3", NOW).unwrap().hours, 8);
    }

    #[test]
    fn lock_now_the_lock_slot_and_the_kill_switch_end_it_and_nothing_renews_it() {
        let mut l = Leases::default();
        let p = pending("muse", "r1", 12);
        l.grant(&p, NOW);
        assert_eq!(l.lock_slot_client("r1-lock").as_deref(), Some("muse"));
        assert!(l.lock_slot_client("").is_none());
        assert!(l.end("muse").is_some());
        assert!(l.active_for("muse", NOW + 1).is_none());
        assert!(l.end("muse").is_none());
        l.grant(&pending("muse", "r2", 1), NOW);
        l.request(pending("bot", "r3", 1));
        let (leases, slots) = l.end_all();
        assert_eq!(leases.len(), 1);
        assert_eq!(slots, vec!["r3".to_string(), "r2-lock".to_string()]);
        assert!(l.active.is_empty() && l.pending.is_empty());
        // Nothing renews: noting commands never moves the end.
        l.grant(&pending("muse", "r4", 1), NOW);
        l.note("muse", "session-status", "", NOW + 3599);
        assert_eq!(l.active_for("muse", NOW + 3599).unwrap().until, NOW + 3600);
    }

    #[test]
    fn the_summary_names_kinds_counts_and_runbooks_never_arguments() {
        let mut l = Leases::default();
        l.grant(&pending("muse", "r1", 4), NOW);
        l.note("muse", "session-status", "", NOW + 1);
        l.note("muse", "session-status", "", NOW + 2);
        l.note("muse", "runbook", "phone-test", NOW + 3);
        l.note("muse", "runbook", "phone-test", NOW + 4);
        l.note("muse", "start-session", "", NOW + 5);
        l.note("bot", "runbook", "other", NOW + 5);
        let lease = l.end("muse").unwrap();
        assert_eq!(summary(&lease, Ended::Locked), "4 h lease ended (locked): runbook 2, session-status 2, start-session 1; runbooks: phone-test; sessions 1; PRs not tracked (no sandbox yet)");
        assert_eq!(summary(&Lease { hours: 1, ..Default::default() }, Ended::Expired), "1 h lease ended (expired): no commands; sessions 0; PRs not tracked (no sandbox yet)");
        // A command after the lease's end isn't counted on it.
        let mut l2 = Leases::default();
        l2.grant(&pending("muse", "r9", 1), NOW);
        l2.note("muse", "run-status", "", NOW + 3600);
        assert!(l2.active[0].usage.commands.is_empty());
    }

    #[test]
    fn kept_on_disk_across_restarts() {
        let dir = std::env::temp_dir().join(format!("glim-lease-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("remote-leases.json");
        let mut l = Leases::load(path.clone());
        l.request(pending("muse", "r1", 1));
        let p = l.take_pending("r1", NOW).unwrap();
        l.grant(&p, NOW);
        let again = Leases::load(path.clone());
        assert_eq!(again.active_for("muse", NOW + 5).unwrap().hours, 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("seed") && !text.contains("key"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
