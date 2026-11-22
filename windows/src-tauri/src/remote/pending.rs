// Settings → Remote → "Waiting for your OK" (owner 2026-11-22, agent sandbox
// plan §7 PR 1): with no Pushover, the open phone approvals are listed on the
// PC itself: each paused run's step and each open lease request, with its
// match code and a QR code of its approval link, so the owner can scan it
// with the paired phone and approve with Face ID. Nothing new leaves the PC:
// the link is the one the request already has (run-status shows it to the
// asking client too), and approving still needs the owner's passkey.

use super::lease::Pending;
use super::runs::{Run, RunState};
use serde::Serialize;

/// One open approval, as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    /// The approval request's id (32 hex): what the QR lookup takes.
    pub rid: String,
    /// "run" (a paused runbook step) or "lease" (an agent asks to be unlocked).
    pub kind: String,
    pub client: String,
    /// A run's runbook, its paused step (1-based), step count and step kind.
    pub runbook: String,
    pub step: usize,
    pub total: usize,
    pub step_kind: String,
    /// A lease request's hours.
    pub hours: u32,
    pub code: String,
    /// When it lapses (Unix seconds).
    pub expires: i64,
}

fn run_open(r: &Run, now: i64) -> bool {
    r.state == RunState::Paused && !r.approval.is_empty() && !r.url.is_empty() && r.expires > now
}

fn lease_open(p: &Pending, now: i64) -> bool {
    !p.rid.is_empty() && !p.url.is_empty() && p.expires > now
}

/// The open approvals, soonest to lapse first.
pub fn list(runs: &[Run], leases: &[Pending], now: i64) -> Vec<Waiting> {
    let mut out: Vec<Waiting> = runs
        .iter()
        .filter(|r| run_open(r, now))
        .map(|r| Waiting {
            rid: r.approval.clone(),
            kind: "run".into(),
            client: r.client.clone(),
            runbook: r.name.clone(),
            step: r.step,
            total: r.total,
            step_kind: r.step_kind.clone(),
            hours: 0,
            code: r.code.clone(),
            expires: r.expires,
        })
        .chain(leases.iter().filter(|p| lease_open(p, now)).map(|p| Waiting {
            rid: p.rid.clone(),
            kind: "lease".into(),
            client: p.client.clone(),
            runbook: String::new(),
            step: 0,
            total: 0,
            step_kind: String::new(),
            hours: p.hours,
            code: p.code.clone(),
            expires: p.expires,
        }))
        .collect();
    out.sort_by(|a, b| a.expires.cmp(&b.expires).then_with(|| a.rid.cmp(&b.rid)));
    out
}

/// The approval link of the open request `rid`, if there is one. Only a link
/// Glim made itself: Settings never hands in text to encode.
pub fn url_for(runs: &[Run], leases: &[Pending], rid: &str, now: i64) -> Option<String> {
    if rid.is_empty() {
        return None;
    }
    runs.iter()
        .find(|r| run_open(r, now) && r.approval == rid)
        .map(|r| r.url.clone())
        .or_else(|| leases.iter().find(|p| lease_open(p, now) && p.rid == rid).map(|p| p.url.clone()))
}

/// The open approvals now.
pub fn now_list() -> Vec<Waiting> {
    let now = super::now_s();
    let runs = super::runs::with(|r| r.runs.clone());
    let leases = super::lease::with(|l| l.pending.clone());
    list(&runs, &leases, now)
}

/// The QR code (SVG) of open request `rid`'s link.
pub fn qr(rid: &str) -> Result<String, String> {
    let now = super::now_s();
    let runs = super::runs::with(|r| r.runs.clone());
    let leases = super::lease::with(|l| l.pending.clone());
    let url = url_for(&runs, &leases, rid, now).ok_or("not-waiting")?;
    super::approve::qr_svg(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, state: RunState, rid: &str, expires: i64) -> Run {
        Run { id: id.into(), name: "deploy".into(), client: "muse".into(), total: 3, state, step: 2, step_kind: "run".into(), expires, approval: rid.into(), code: "ABC-DEF".into(), url: format!("https://x/#r={rid}"), ..Default::default() }
    }

    fn lease(rid: &str, expires: i64) -> Pending {
        Pending { rid: rid.into(), code: "GHJ-KMN".into(), client: "agent".into(), hours: 4, expires, url: format!("https://x/#r={rid}"), lock: "l".into(), key: "ab".repeat(32) }
    }

    #[test]
    fn lists_only_open_requests_soonest_first() {
        let runs = vec![
            run("a", RunState::Paused, "r-run", 500),
            run("b", RunState::Paused, "", 500),
            run("c", RunState::Done, "r-done", 500),
            run("d", RunState::Paused, "r-old", 100),
        ];
        let leases = vec![lease("r-lease", 300), lease("r-gone", 50)];
        let l = list(&runs, &leases, 200);
        assert_eq!(l.iter().map(|w| w.rid.as_str()).collect::<Vec<_>>(), ["r-lease", "r-run"]);
        assert_eq!(l[0].kind, "lease");
        assert_eq!(l[0].hours, 4);
        assert_eq!(l[0].runbook, "");
        assert_eq!(l[0].code, "GHJ-KMN");
        assert_eq!(l[1].kind, "run");
        assert_eq!((l[1].runbook.as_str(), l[1].step, l[1].total, l[1].step_kind.as_str(), l[1].hours), ("deploy", 2, 3, "run", 0));
        assert_eq!(l[1].client, "muse");
    }

    #[test]
    fn url_only_for_an_open_request() {
        let runs = vec![run("a", RunState::Paused, "r-run", 500), run("c", RunState::Done, "r-done", 500)];
        let leases = vec![lease("r-lease", 300), lease("r-gone", 50)];
        assert_eq!(url_for(&runs, &leases, "r-run", 200).as_deref(), Some("https://x/#r=r-run"));
        assert_eq!(url_for(&runs, &leases, "r-lease", 200).as_deref(), Some("https://x/#r=r-lease"));
        assert_eq!(url_for(&runs, &leases, "r-done", 200), None);
        assert_eq!(url_for(&runs, &leases, "r-gone", 200), None);
        assert_eq!(url_for(&runs, &leases, "", 200), None);
        assert_eq!(url_for(&runs, &leases, "https://evil/", 200), None);
    }
}
