// Phone notifications (phone companion step 1, docs/phone-plan.md): a
// one-way push to the owner's phone through Pushover, behind Settings → Phone,
// off by default.
//
// What may leave the PC is decided here, not by the page: the island sends a
// kind, a pill id and a project name; the text comes from fixed templates
// (`message`), the agent's name from a fixed list (`agent_label`), and the
// project name only when Settings allows it, cleaned and cut short. Never a
// command, a file, a prompt or anything typed or captured. The templates are
// English on purpose: short, fixed, and easy to audit.
//
// The request goes through net/mod.rs, which refuses api.pushover.net unless
// the setting is on (`apply_settings`). The user key and app token live in
// Windows Credential Manager (`Glim/PushoverKeys`); Settings only sees a hint,
// and neither key is ever logged. The log says which kind was sent or failed,
// nothing else.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ai::key::{CredentialManager, Vault};
use crate::i18n::{t, tf};
use crate::net::{self, Method, Url};
use crate::settings::Settings;

/// Pushover's message endpoint (https on 443, net::PUSH_HOST).
pub const ENDPOINT: &str = "https://api.pushover.net/1/messages.json";
/// The credential's name in Credential Manager.
pub const TARGET: &str = "Glim/PushoverKeys";
const TIMEOUT: Duration = Duration::from_secs(15);
/// At most one notification this often…
pub const MIN_GAP: Duration = Duration::from_secs(10);
/// …and this many an hour (docs/phone-plan.md §5).
pub const PER_HOUR: usize = 30;
/// The longest project name sent.
pub const MAX_PROJECT: usize = 40;
/// Pushover's user keys and app tokens: 30 letters and digits.
const KEY_LEN: usize = 30;

/// What happened. Each has its own fixed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Approval,
    Finished,
    UsageLimit,
    SpendLimit,
    Test,
}

impl Kind {
    /// The log's word for it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Approval => "approval",
            Kind::Finished => "finished",
            Kind::UsageLimit => "usage limit",
            Kind::SpendLimit => "spend limit",
            Kind::Test => "test",
        }
    }
}

/// The agent's name for a pill: Claude Code's pill, or an agent from
/// agents.rs's list; anything else is "An agent".
pub fn agent_label(pill_id: &str) -> String {
    if pill_id == "integration_claude" {
        return "Claude Code".into();
    }
    pill_id
        .strip_prefix("agent_")
        .and_then(|id| crate::agents::Agent::ALL.iter().find(|a| a.id() == id))
        .map_or_else(|| "An agent".into(), |a| a.name().to_string())
}

/// The project as it may be sent: the last part of a folder name, no
/// control characters or path separators, at most MAX_PROJECT characters.
pub fn clean_project(raw: &str) -> Option<String> {
    let last = raw.trim().rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| !c.is_control()).take(MAX_PROJECT).collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty()).then_some(clean)
}

/// The notification's text: a fixed template, the agent, and the project
/// when there is one.
pub fn message(kind: Kind, agent: &str, project: Option<&str>) -> String {
    let who = match project {
        Some(p) => format!("{agent} in {p}"),
        None => agent.to_string(),
    };
    match kind {
        Kind::Approval => format!("{who} needs approval"),
        Kind::Finished => format!("{who} finished"),
        Kind::UsageLimit => format!("{who} hit a usage limit"),
        Kind::SpendLimit => format!("{who} hit a spend limit"),
        Kind::Test => "Test notification from Glim".into(),
    }
}

/// Whether Settings lets this kind through at all.
pub fn wanted(settings: &Settings, kind: Kind) -> bool {
    let p = &settings.phone;
    p.enabled
        && match kind {
            Kind::Approval => p.approval,
            Kind::Finished => p.finished,
            Kind::UsageLimit | Kind::SpendLimit => p.limit,
            Kind::Test => true,
        }
}

/// Settings were loaded or saved: the choke point follows the switch.
pub fn apply_settings(settings: &Settings) {
    net::set_push_allowed(settings.phone.enabled);
}

// ── Rate limit ────────────────────────────────────────────────────────────────

/// One every MIN_GAP, PER_HOUR an hour.
#[derive(Default)]
pub struct Limiter {
    sent: VecDeque<Instant>,
}

impl Limiter {
    /// True (and counted) when one may go now.
    pub fn take(&mut self, now: Instant) -> bool {
        while self.sent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600)) {
            self.sent.pop_front();
        }
        if self.sent.back().is_some_and(|t| now.duration_since(*t) < MIN_GAP) || self.sent.len() >= PER_HOUR {
            return false;
        }
        self.sent.push_back(now);
        true
    }
}

static LIMITER: Mutex<Option<Limiter>> = Mutex::new(None);

fn take_slot() -> bool {
    LIMITER.lock().unwrap().get_or_insert_with(Limiter::default).take(Instant::now())
}

// ── Keys ──────────────────────────────────────────────────────────────────────

/// The user key and app token, as only the request may see them.
#[derive(Clone, PartialEq, Eq)]
pub struct Keys {
    user: String,
    token: String,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(hidden)")
    }
}

/// What Settings may know: set or not, and the last four characters of each.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysStatus {
    pub set: bool,
    pub user_hint: String,
    pub token_hint: String,
}

fn hint(k: &str) -> String {
    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

fn valid(k: &str) -> bool {
    k.len() == KEY_LEN && k.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn vault() -> impl Vault {
    CredentialManager { target: TARGET }
}

/// The saved keys, if both are there.
pub fn load(vault: &dyn Vault) -> Result<Option<Keys>, String> {
    Ok(vault.read()?.and_then(|s| {
        let (user, token) = s.split_once('\n')?;
        (valid(user) && valid(token)).then(|| Keys { user: user.into(), token: token.into() })
    }))
}

pub fn status(vault: &dyn Vault) -> Result<KeysStatus, String> {
    Ok(load(vault)?.map_or_else(KeysStatus::default, |k| KeysStatus { set: true, user_hint: hint(&k.user), token_hint: hint(&k.token) }))
}

/// Saves both keys, pasted from pushover.net. A bad paste keeps what was saved.
pub fn save(vault: &dyn Vault, user: &str, token: &str) -> Result<KeysStatus, String> {
    let (user, token) = (user.trim(), token.trim());
    if !valid(user) {
        return Err(t("That doesn't look like a Pushover user key (30 letters and digits)."));
    }
    if !valid(token) {
        return Err(t("That doesn't look like a Pushover app token (30 letters and digits)."));
    }
    vault.write(&format!("{user}\n{token}"))?;
    status(vault)
}

pub fn remove(vault: &dyn Vault) -> Result<KeysStatus, String> {
    vault.delete()?;
    Ok(KeysStatus::default())
}

// ── Sending ───────────────────────────────────────────────────────────────────

/// One POST to `endpoint` with exactly these fields; the answer's body is
/// only read for an error.
pub async fn send(endpoint: &Url, keys: &Keys, text: &str) -> Result<(), String> {
    let body = json!({ "token": keys.token, "user": keys.user, "title": "Glim", "message": text, "priority": 0 });
    let response = net::request(Method::POST, endpoint, TIMEOUT)?
        .json(&body)
        .send()
        .await
        .map_err(|e| tf("Network error: {error}", &[("error", &e.without_url().to_string())]))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let detail = net::read_capped(response, net::MAX_ERROR_BODY).await.map(|b| net::error_detail(&b)).unwrap_or_default();
    // Pushover's errors are a list of strings under `errors`.
    let detail = serde_json::from_str::<serde_json::Value>(&detail)
        .ok()
        .and_then(|v| v.pointer("/errors/0").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(detail);
    Err(tf("Pushover said {status}: {detail}", &[("status", status.as_str()), ("detail", &detail)]))
}

/// A notification from the island (or the test button): checked against
/// Settings and the rate limit, then sent. The log gets the kind and the
/// outcome only.
pub async fn notify(settings: &Settings, kind: Kind, pill_id: &str, project: &str) -> Result<(), String> {
    if !wanted(settings, kind) {
        return Err(t("Phone notifications are off."));
    }
    let Some(keys) = load(&vault())? else {
        return Err(t("Save your Pushover keys first."));
    };
    if kind != Kind::Test && !take_slot() {
        crate::log::line(format!("phone: dropped {} (rate limit)", kind.name()));
        return Ok(());
    }
    let project = settings.phone.include_project.then(|| clean_project(project)).flatten();
    let text = message(kind, &agent_label(pill_id), project.as_deref());
    let endpoint = Url::parse(ENDPOINT).map_err(|e| e.to_string())?;
    let sent = send(&endpoint, &keys, &text).await;
    match &sent {
        Ok(()) => crate::log::line(format!("phone: sent {}", kind.name())),
        Err(_) => crate::log::line(format!("phone: failed {}", kind.name())),
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::key::MemoryVault;
    use crate::net::tests::{serve_capture, serve_once};

    const USER: &str = "uQiRzpo4DXghDmr9QzzfQu27cmVRsG";
    const TOKEN: &str = "azGDORePK8gMaC0QOYAMyEEuzJnyUi";

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    #[test]
    fn the_texts_are_fixed_templates() {
        let p = Some("local-companion");
        assert_eq!(message(Kind::Approval, "Claude Code", p), "Claude Code in local-companion needs approval");
        assert_eq!(message(Kind::Finished, "Claude Code", p), "Claude Code in local-companion finished");
        assert_eq!(message(Kind::UsageLimit, "Codex", p), "Codex in local-companion hit a usage limit");
        assert_eq!(message(Kind::SpendLimit, "Claude Code", None), "Claude Code hit a spend limit");
        assert_eq!(message(Kind::Approval, "Claude Code", None), "Claude Code needs approval");
        assert_eq!(message(Kind::Test, "whatever", p), "Test notification from Glim");
    }

    #[test]
    fn the_agent_comes_from_a_fixed_list() {
        assert_eq!(agent_label("integration_claude"), "Claude Code");
        assert_eq!(agent_label("agent_codex"), crate::agents::Agent::Codex.name());
        for odd in ["agent_rm -rf /", "integration_ollama", "", "Claude Code needs approval: DROP TABLE"] {
            assert_eq!(agent_label(odd), "An agent", "{odd}");
        }
    }

    #[test]
    fn only_the_folder_name_goes_cleaned_and_short() {
        assert_eq!(clean_project("local-companion").as_deref(), Some("local-companion"));
        assert_eq!(clean_project("C:\\Users\\antho\\secret-client\\repo").as_deref(), Some("repo"));
        assert_eq!(clean_project("/home/a/b/proj\n").as_deref(), Some("proj"));
        assert_eq!(clean_project("a\u{0007}b").as_deref(), Some("ab"));
        assert_eq!(clean_project(&"x".repeat(100)).map(|s| s.chars().count()), Some(MAX_PROJECT));
        assert_eq!(clean_project("  "), None);
        assert_eq!(clean_project("C:\\"), None);
    }

    #[test]
    fn settings_let_each_kind_through_only_when_on() {
        let mut s = Settings::default();
        assert!(!s.phone.enabled, "off by default");
        for k in [Kind::Approval, Kind::Finished, Kind::UsageLimit, Kind::SpendLimit, Kind::Test] {
            assert!(!wanted(&s, k), "{k:?}");
        }
        s.phone.enabled = true;
        assert!(wanted(&s, Kind::Approval) && wanted(&s, Kind::Finished) && wanted(&s, Kind::UsageLimit) && wanted(&s, Kind::Test));
        s.phone.finished = false;
        s.phone.limit = false;
        assert!(!wanted(&s, Kind::Finished) && !wanted(&s, Kind::SpendLimit));
        assert!(wanted(&s, Kind::Approval));
    }

    #[test]
    fn at_most_one_every_ten_seconds_and_thirty_an_hour() {
        let mut l = Limiter::default();
        let t0 = Instant::now();
        assert!(l.take(t0));
        assert!(!l.take(t0 + Duration::from_secs(9)));
        assert!(l.take(t0 + Duration::from_secs(10)));
        let mut sent = 2;
        let mut t = t0 + Duration::from_secs(10);
        while t + Duration::from_secs(11) < t0 + Duration::from_secs(3600) {
            t += Duration::from_secs(11);
            if l.take(t) {
                sent += 1;
            }
        }
        assert_eq!(sent, PER_HOUR, "30 in the first hour");
        assert!(l.take(t0 + Duration::from_secs(3600 + 11)), "an hour on, a slot frees up");
    }

    #[test]
    fn keys_are_checked_kept_together_and_only_hinted() {
        let v = MemoryVault::default();
        assert_eq!(status(&v).unwrap(), KeysStatus::default());
        assert!(save(&v, "short", TOKEN).is_err());
        assert!(save(&v, USER, "has spaces in it 0123456789012").is_err());
        assert!(v.0.borrow().is_none(), "a bad paste saves nothing");
        let st = save(&v, &format!("  {USER} "), TOKEN).unwrap();
        assert_eq!(st, KeysStatus { set: true, user_hint: "…VRsG".into(), token_hint: "…nyUi".into() });
        let k = load(&v).unwrap().unwrap();
        assert_eq!(format!("{k:?}"), "Keys(hidden)");
        assert!(!format!("{st:?}").contains(USER) && !format!("{st:?}").contains(TOKEN));
        assert_eq!(remove(&v).unwrap(), KeysStatus::default());
        assert!(load(&v).unwrap().is_none());
    }

    #[test]
    fn the_request_carries_exactly_these_fields() {
        let (url, seen) = serve_capture("200 OK", "Content-Type: application/json\r\n", br#"{"status":1,"request":"x"}"#.to_vec());
        let keys = Keys { user: USER.into(), token: TOKEN.into() };
        let endpoint = Url::parse(&format!("{url}/1/messages.json")).unwrap();
        block_on(send(&endpoint, &keys, "Claude Code in local-companion needs approval")).unwrap();
        let request = String::from_utf8(seen.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert!(request.starts_with("POST /1/messages.json "), "{request}");
        let body: serde_json::Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            json!({ "token": TOKEN, "user": USER, "title": "Glim", "message": "Claude Code in local-companion needs approval", "priority": 0 })
        );
    }

    #[test]
    fn an_error_answer_is_pushovers_own_message() {
        let url = serve_once("400 Bad Request", "Content-Type: application/json\r\n", br#"{"user":"invalid","errors":["user identifier is invalid"],"status":0}"#.to_vec());
        let keys = Keys { user: USER.into(), token: TOKEN.into() };
        let err = block_on(send(&Url::parse(&url).unwrap(), &keys, "x")).unwrap_err();
        assert!(err.contains("400") && err.contains("user identifier is invalid"), "{err}");
        assert!(!err.contains(USER) && !err.contains(TOKEN));
    }

    #[test]
    fn keys_and_texts_never_reach_the_log() {
        let src = include_str!("phone.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for line in code.lines().filter(|l| l.contains("log::line")) {
            for banned in ["text", "keys", "token", "user", "project", "pill_id"] {
                assert!(!line.contains(banned), "{banned} in: {line}");
            }
        }
    }
}
