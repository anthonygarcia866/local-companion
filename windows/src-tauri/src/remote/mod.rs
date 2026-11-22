// Remote courier (owner 2026-11-14; roadmap: remote courier Phase 1 = Phase
// 3 delegation). This file is the gate every task passes (`Gate::accept`)
// before anything happens; courier.rs is the relay connection that feeds it
// (net::relay, the transport the owner approved 2026-11-14) and signs every
// result with the PC's own key; exec.rs is what the five commands do.
// docs/remote-courier.md has the formats and the setup.
//
// A task is a signed envelope, `{"v":1,"payload":"<json>","sig":"<hex>"}`:
// - `sig` is an Ed25519 signature over the exact UTF-8 bytes of `payload`
//   (so there is nothing to canonicalize). Glim keeps only the PUBLIC key,
//   pasted in Settings → Remote (64 hex characters), with its fingerprint
//   shown; the private key never touches this PC.
// - `payload` is `{"id","iat","exp","cmd","args"}`: a unique id (16-64 of
//   A-Z a-z 0-9 _ -), issued-at and expires-at in Unix seconds, expiry at
//   most 72 h after issue (24 h for start-session and send-to-session;
//   owner 2026-11-17), 60 s clock skew allowed on the issue time but NONE
//   after the expiry (a task expired at the relay never runs), one of six
//   commands with its arguments (command.rs), and optionally `rk`: the
//   X25519 key (64 hex) the result is sealed to (seal.rs).
// - Store-and-forward (owner 2026-11-17): the task may arrive sealed to this
//   PC (`{"v":2,…}`, seal.rs); courier.rs opens it before this gate, which
//   always checks the signed v1 envelope inside.
// - Replay protection: an id is accepted once; seen ids are kept (on disk,
//   so a restart doesn't reopen them) until their expiry has passed.
// - Cloud clients (owner 2026-11-15): one per Claude account ("account-1",
//   "account-2" ...), each with its own relay token and its own signer key.
//   The relay stamps each task with the client whose token queued it; the
//   task must verify with THAT client's key (signed with another client's
//   key: "wrong-client"). The single signer key from before clients is
//   "account-1" (`public_key`, read as that client: nothing to re-enter).
// - Per-client commands (owner 2026-11-21, for non-Claude agents such as
//   "muse"): each client may send only the commands ticked for it in
//   Settings → Remote → Cloud clients (`CloudClient::commands`; none saved =
//   all seven, the clients from before; a new client starts with
//   session-status and run-status). Any other is refused `not-allowed`.
//   transcript-tail, start-session and send-to-session also need a Claude
//   account mapped to the client (Settings → Claude Code & Agents →
//   Accounts, or the client's row): refused `no-account` otherwise.
// - Rate limits, per rolling hour: per client at most 5 start-session and
//   30 commands; on this PC in all, 8 start-session and 50 commands; 3
//   keepalive-restart.
// - Repo allowlist: commands that name a repo only for REPOS
//   (local-companion to start).
// - The channel is OFF by default and needs its one-time consent card; the
//   kill switch (tray + Settings, one click) refuses everything until it is
//   switched back in Settings. A task issued before that "Allow again" (one
//   queued at the relay while stopped, handed over on reconnect) is refused
//   as killed too, never run (owner 2026-11-14).
// - Every decision goes to the "Remote activity" list (Settings) and
//   glim.log: time, outcome, the command kind or the rejection's reason
//   code. Never a payload, an argument, a signature or a key; for a task
//   whose signature didn't verify, not even its id.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod approve;
pub mod command;
pub mod courier;
pub mod exec;
pub mod runs;
pub mod seal;
pub mod wake;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod courier_tests;

pub use command::Command;

/// Repos a remote command may name (owner 2026-11-14: Glim's own repo only).
pub const REPOS: &[&str] = &["local-companion"];
/// A task lives at most this long (seconds; owner 2026-11-17, was 600).
pub const MAX_LIFETIME_S: i64 = 72 * 3600;
/// start-session and send-to-session act into live work: 24 h at most.
pub const MAX_LIVE_LIFETIME_S: i64 = 24 * 3600;
/// Clock skew tolerated between the signer and this PC (seconds).
pub const SKEW_S: i64 = 60;
/// Per cloud client, per rolling hour.
pub const MAX_START_PER_HOUR: usize = 5;
pub const MAX_COMMANDS_PER_HOUR: usize = 30;
/// On this PC, all clients together, per rolling hour (owner 2026-11-15).
pub const MAX_START_PER_HOUR_PC: usize = 8;
pub const MAX_COMMANDS_PER_HOUR_PC: usize = 50;
/// keepalive-restart, per rolling hour, on this PC (owner 2026-11-15).
pub const MAX_KEEPALIVE_RESTARTS_PER_HOUR: usize = 3;
/// Remote runbook tasks an hour (owner 2026-11-20, D6): defaults, and the
/// ranges Settings → Remote may set (on the PC only; no command carries them).
pub const DEFAULT_RUNBOOKS_PER_HOUR: u32 = 5;
pub const DEFAULT_RUNBOOKS_PER_HOUR_PC: u32 = 10;
pub const RUNBOOKS_PER_HOUR_RANGE: std::ops::RangeInclusive<u32> = 1..=20;
pub const RUNBOOKS_PER_HOUR_PC_RANGE: std::ops::RangeInclusive<u32> = 1..=50;
/// The client a task from a relay without client names (or the signer key
/// from before clients) belongs to.
pub const LEGACY_CLIENT: &str = "account-1";
/// Cloud clients at most (the relay's list fits one secret).
pub const MAX_CLIENTS: usize = 10;
/// The relay's token-hash value for the client whose hash is the relay's
/// old single CLOUD_TOKEN_SHA256 (relay/src/core.js cloudClients).
pub const LEGACY_TOKEN: &str = "legacy";
/// The biggest envelope read at all (bytes).
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024;
/// Activity entries kept.
pub const ACTIVITY_KEPT: usize = 50;
/// Every command kind, in Settings' order.
pub const COMMANDS: &[&str] = &["session-status", "run-status", "transcript-tail", "start-session", "send-to-session", "runbook", "keepalive-restart"];
/// What a new cloud client may send until the owner ticks more (owner 2026-11-21).
pub const DEFAULT_COMMANDS: &[&str] = &["session-status", "run-status"];
/// Commands that act in a Claude account's sessions: refused `no-account`
/// for a client no account is mapped to (owner 2026-11-21).
pub const ACCOUNT_COMMANDS: &[&str] = &["transcript-tail", "start-session", "send-to-session"];

/// Settings → Remote's "Remote tasks", as settings.json keeps it. Rust-owned
/// (a page's save never changes it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteSettings {
    /// The channel is on (off by default).
    pub on: bool,
    /// The one-time consent card was answered "Turn on".
    pub consented: bool,
    /// The kill switch: everything is refused until it is switched back.
    pub killed: bool,
    /// When the kill switch was last switched back ("Allow again"), Unix
    /// seconds (0: never). A task issued before it was queued while stopped:
    /// refused as killed, never run (owner 2026-11-14).
    pub allowed_since: i64,
    /// The signer key from before cloud clients (64 lower-case hex, "" none):
    /// read as client "account-1" while `clients` is empty (`clients()`).
    pub public_key: String,
    /// Cloud clients, one per Claude account (owner 2026-11-15).
    pub clients: Vec<CloudClient>,
    /// The relay's address, "https://<name>.<subdomain>.workers.dev" ("" none).
    pub relay: String,
    /// The PC's result key (public half, 64 hex) once set up ("" none); the
    /// private half is only in Credential Manager (courier::PC_KEY_TARGET).
    pub pc_public_key: String,
    /// The PC's encryption key (X25519, public half, 64 hex) that tasks are
    /// sealed to: cloud Glim's GLIM_PC_ENC_KEY ("" none). The private half
    /// is only in Credential Manager (courier::PC_ENC_TARGET).
    pub pc_enc_key: String,
    /// "Require sealed tasks" (owner 2026-11-19; off by default): refuse
    /// any task not sealed to the PC's encryption key, or not naming a key
    /// its result is sealed to, as `not-sealed`. The owner turns it on once
    /// every cloud environment has GLIM_PC_ENC_KEY and the async cloud test
    /// passed for each account.
    pub require_sealed: bool,
    /// Remote runbook tasks an hour per cloud client; 0 = the default (5).
    pub runbooks_per_hour: u32,
    /// Remote runbook tasks an hour on this PC; 0 = the default (10).
    pub runbooks_per_hour_pc: u32,
    /// Phone approvers (owner 2026-11-21): WebAuthn credentials allowed to
    /// OK a paused remote runbook step. Added and removed on this PC only
    /// (Settings → Remote); their keys in Credential Manager.
    pub approvers: Vec<approve::Approver>,
}

impl RemoteSettings {
    /// (per client, on the PC), defaults filled in and kept in range.
    pub fn runbook_limits(&self) -> (u32, u32) {
        let pick = |v: u32, d: u32, r: &std::ops::RangeInclusive<u32>| if v == 0 { d } else { v.clamp(*r.start(), *r.end()) };
        (pick(self.runbooks_per_hour, DEFAULT_RUNBOOKS_PER_HOUR, &RUNBOOKS_PER_HOUR_RANGE), pick(self.runbooks_per_hour_pc, DEFAULT_RUNBOOKS_PER_HOUR_PC, &RUNBOOKS_PER_HOUR_PC_RANGE))
    }
}

/// One cloud client: its name, its signer's PUBLIC key, the SHA-256 of its
/// relay token (not secret; "legacy" = the relay's old single cloud token)
/// and when it was added (Unix seconds; 0 = from before clients).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudClient {
    pub name: String,
    pub public_key: String,
    pub token_sha256: String,
    pub added: i64,
    /// The commands it may send (owner 2026-11-21). None (nothing saved): all
    /// of them, as for every client from before the list existed; a client
    /// added since starts with DEFAULT_COMMANDS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
    /// A token-less agent client (owner 2026-11-21): no relay token at all;
    /// the relay checks a signature with `public_key` on each of its
    /// requests (relay/src/agentauth.js). `token_sha256` is "".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub agent: bool,
}

impl CloudClient {
    /// It may send a command of this kind.
    pub fn allows(&self, kind: &str) -> bool {
        match &self.commands {
            None => COMMANDS.contains(&kind),
            Some(list) => COMMANDS.contains(&kind) && list.iter().any(|k| k == kind),
        }
    }

    /// The kinds it may send, in Settings' order.
    pub fn allowed(&self) -> Vec<String> {
        COMMANDS.iter().filter(|k| self.allows(k)).map(|k| k.to_string()).collect()
    }
}

/// A command list as Settings sends it: only known kinds, each once, in
/// Settings' order; anything else refuses the whole list.
pub fn parse_commands(list: &[String]) -> Result<Vec<String>, &'static str> {
    if list.iter().any(|k| !COMMANDS.contains(&k.as_str())) {
        return Err("unknown-command");
    }
    Ok(COMMANDS.iter().filter(|k| list.iter().any(|x| x == *k)).map(|k| k.to_string()).collect())
}

/// A client's name as typed: 1-32 of a-z 0-9 -, starting with a letter or
/// digit (lower-cased, trimmed).
pub fn parse_client_name(raw: &str) -> Result<String, &'static str> {
    let n = raw.trim().to_ascii_lowercase();
    let ok = (1..=32).contains(&n.len()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !n.starts_with('-');
    if ok {
        Ok(n)
    } else {
        Err("bad-name")
    }
}

impl RemoteSettings {
    /// The cloud clients: the list, or (before any was added) the old single
    /// signer key as "account-1".
    pub fn clients(&self) -> Vec<CloudClient> {
        if self.clients.is_empty() && !self.public_key.is_empty() {
            return vec![CloudClient { name: LEGACY_CLIENT.into(), public_key: self.public_key.clone(), token_sha256: LEGACY_TOKEN.into(), added: 0, commands: None, agent: false }];
        }
        self.clients.clone()
    }

    /// Adds a client (the list replaces the old single key from now on).
    pub fn add_client(&mut self, c: CloudClient) -> Result<(), &'static str> {
        let mut list = self.clients();
        if list.iter().any(|x| x.name == c.name) {
            return Err("name-taken");
        }
        if list.iter().any(|x| x.public_key == c.public_key) {
            return Err("key-taken");
        }
        if list.len() >= MAX_CLIENTS {
            return Err("too-many");
        }
        list.push(c);
        self.clients = list;
        self.public_key.clear();
        Ok(())
    }

    /// The commands a client may send (Settings); false when there is no
    /// client by that name.
    pub fn set_client_commands(&mut self, name: &str, commands: Vec<String>) -> bool {
        let mut list = self.clients();
        let Some(c) = list.iter_mut().find(|c| c.name == name) else { return false };
        c.commands = Some(commands);
        self.clients = list;
        self.public_key.clear();
        true
    }

    /// Removes a client; false when there was none by that name.
    pub fn remove_client(&mut self, name: &str) -> bool {
        let mut list = self.clients();
        let before = list.len();
        list.retain(|x| x.name != name);
        let gone = list.len() != before;
        self.clients = list;
        if gone {
            self.public_key.clear();
        }
        gone
    }

    /// The relay's CLOUD_CLIENTS value ("name=hash,..."; "none" for nobody),
    /// which Glim keeps in the REMOTE_CLOUD_CLIENTS secret for the
    /// relay-tokens runbook.
    pub fn relay_list(&self) -> String {
        let list = self.clients();
        if list.is_empty() {
            return "none".into();
        }
        list.iter()
            .map(|c| if c.agent { format!("{}=key:{}", c.name, c.public_key) } else { format!("{}={}", c.name, c.token_sha256) })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The channel opens again (the kill switch's "Allow again", or Remote
    /// tasks switched back on): tasks issued before `now` waited at the relay
    /// while it was shut and are refused as killed (owner 2026-11-14). Every
    /// path that re-opens the channel calls this.
    pub fn reopen(&mut self, now: i64) {
        self.allowed_since = now;
    }
}

/// Why a task was refused: a fixed code, never text from the task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Off,
    Killed,
    NoKey,
    TooBig,
    BadEnvelope,
    BadSignature,
    BadPayload,
    BadId,
    Expired,
    NotYetValid,
    TooLong,
    Replay,
    UnknownCommand,
    BadArgs,
    RepoNotAllowed,
    RateLimited,
    /// The relay named no client this PC knows (removed, or never added).
    UnknownClient,
    /// Signed by another client's key than the one whose token queued it.
    WrongClient,
    /// A sealed task this PC couldn't open, or whose outside doesn't match
    /// the signed task inside.
    BadSeal,
    /// "Require sealed tasks" is on and the task came unsealed, or named no
    /// result key.
    NotSealed,
    /// A command not ticked for this client (owner 2026-11-21).
    NotAllowed,
    /// A session command from a client no Claude account is mapped to.
    NoAccount,
}

impl Rejection {
    pub fn code(self) -> &'static str {
        match self {
            Rejection::Off => "off",
            Rejection::Killed => "killed",
            Rejection::NoKey => "no-key",
            Rejection::TooBig => "too-big",
            Rejection::BadEnvelope => "bad-envelope",
            Rejection::BadSignature => "bad-signature",
            Rejection::BadPayload => "bad-payload",
            Rejection::BadId => "bad-id",
            Rejection::Expired => "expired",
            Rejection::NotYetValid => "not-yet-valid",
            Rejection::TooLong => "lifetime-too-long",
            Rejection::Replay => "replay",
            Rejection::UnknownCommand => "unknown-command",
            Rejection::BadArgs => "bad-args",
            Rejection::RepoNotAllowed => "repo-not-allowed",
            Rejection::RateLimited => "rate-limited",
            Rejection::UnknownClient => "unknown-client",
            Rejection::WrongClient => "wrong-client",
            Rejection::BadSeal => "bad-seal",
            Rejection::NotSealed => "not-sealed",
            Rejection::NotAllowed => "not-allowed",
            Rejection::NoAccount => "no-account",
        }
    }
}

/// A public key as typed: 64 hex characters (spaces and colons ignored),
/// lower-cased; refused otherwise.
pub fn parse_public_key(raw: &str) -> Result<String, &'static str> {
    let hex: String = raw.chars().filter(|c| !c.is_whitespace() && *c != ':').collect::<String>().to_ascii_lowercase();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not-a-key");
    }
    Ok(hex)
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// What Settings shows for a key: the first 16 bytes of its SHA-256, as
/// eight groups of four hex characters ("3F2A 91C0 …"). Nothing for no key
/// (or anything but a 32-byte one): not the hash of nothing ("E3B0 C442 …").
pub fn fingerprint(public_key_hex: &str) -> String {
    let Some(bytes) = from_hex(public_key_hex).filter(|b| b.len() == 32) else { return String::new() };
    let digest = Sha256::digest(&bytes);
    digest[..16].chunks(2).map(|c| format!("{:02X}{:02X}", c[0], c[1])).collect::<Vec<_>>().join(" ")
}

#[derive(Deserialize)]
struct Envelope {
    v: u32,
    payload: String,
    sig: String,
}

#[derive(Deserialize)]
struct Payload {
    id: String,
    iat: i64,
    exp: i64,
    cmd: String,
    #[serde(default)]
    args: serde_json::Map<String, serde_json::Value>,
    /// The X25519 key (64 hex) the result is sealed to (store-and-forward).
    #[serde(default)]
    rk: Option<String>,
}

/// A task that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub id: String,
    pub expires: i64,
    /// Its signed `iat` (the result's `created`).
    pub iat: i64,
    pub command: Command,
    /// The cloud client that sent it (its token queued it, its key signed it).
    pub client: String,
    /// The key its result is sealed to: named in the signed payload, so the
    /// relay can't swap it. None: an older courier, answered unsealed.
    pub rk: Option<[u8; 32]>,
}

/// One line of the Remote activity list: no payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Unix seconds.
    pub at: i64,
    pub ok: bool,
    /// The command's kind when accepted ("session-status" …), else "".
    pub kind: String,
    /// The rejection's code, else "".
    pub reason: String,
    /// The task id, only once its signature verified.
    pub id: String,
    /// The cloud client the relay said sent it (a checked name), "" none.
    #[serde(default)]
    pub client: String,
    /// The runbook a `runbook` task named (owner 2026-11-20, D4), "" else.
    /// The Remote activity list only: glim.log keeps kinds and codes.
    #[serde(default)]
    pub runbook: String,
}

/// One accepted command, for the rate limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub at: i64,
    pub start: bool,
    pub client: String,
    /// A `runbook` task (added 2026-11-20; older entries weren't).
    #[serde(default)]
    pub runbook: bool,
}

/// Replay cache, rate counters and the activity list. `path` keeps the
/// seen ids and the activity across restarts (None in tests that don't
/// need it).
#[derive(Debug, Default)]
pub struct Gate {
    /// id → its expiry.
    seen: BTreeMap<String, i64>,
    /// Accepted commands (start-session flagged, with their client).
    recent: Vec<Call>,
    /// Accepted keepalive-restart times.
    restarts: Vec<i64>,
    pub activity: Vec<Activity>,
    path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    seen: BTreeMap<String, i64>,
    /// Before cloud clients: (time, start-session); all account-1's.
    #[serde(default)]
    recent: Vec<(i64, bool)>,
    activity: Vec<Activity>,
    /// Added 2026-11-15: an older file has none (its seen ids still load).
    #[serde(default)]
    restarts: Vec<i64>,
    /// Added 2026-11-15 (cloud clients).
    #[serde(default)]
    calls: Vec<Call>,
}

impl Gate {
    /// The gate kept in `path` (`%LOCALAPPDATA%\Glim\remote-state.json`).
    pub fn load(path: std::path::PathBuf) -> Gate {
        let saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut recent: Vec<Call> = saved.recent.iter().map(|(at, start)| Call { at: *at, start: *start, client: LEGACY_CLIENT.into(), runbook: false }).collect();
        recent.extend(saved.calls);
        Gate { seen: saved.seen, recent, restarts: saved.restarts, activity: saved.activity, path: Some(path) }
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::settings::local_dir().join("remote-state.json")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let saved = Saved { seen: self.seen.clone(), recent: Vec::new(), activity: self.activity.clone(), restarts: self.restarts.clone(), calls: self.recent.clone() };
        if let Ok(bytes) = serde_json::to_vec(&saved) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    fn note(&mut self, entry: Activity) {
        let line = if entry.ok { format!("remote: accepted {}", entry.kind) } else { format!("remote: rejected {}", entry.reason) };
        crate::log::line(line);
        self.activity.push(entry);
        let extra = self.activity.len().saturating_sub(ACTIVITY_KEPT);
        self.activity.drain(..extra);
    }

    /// A task from the single signer of before cloud clients ("account-1"),
    /// as if a Claude account were mapped to it (tests only).
    #[cfg(test)]
    pub fn accept(&mut self, s: &RemoteSettings, raw: &[u8], now: i64) -> Result<Accepted, Rejection> {
        self.accept_from(s, LEGACY_CLIENT, raw, now)
    }

    /// A task as the relay hands it over: sealed to this PC (`{"v":2,…}`,
    /// opened with `enc`, its outside checked against the signed task
    /// inside) or not; then every check of `accept_from` on the signed task.
    /// A seal that doesn't open is noted and refused as `bad-seal` (or, as
    /// for any task, `killed` / `off` first). `mapped`: a Claude account is
    /// mapped to `client` (accounts::for_client); the app's only entry.
    pub fn accept_sealed(&mut self, s: &RemoteSettings, client: &str, raw: &[u8], now: i64, enc: Option<&seal::Secret>, mapped: bool) -> Result<Accepted, Rejection> {
        let text = std::str::from_utf8(raw).unwrap_or("");
        if seal::version(text) != Some(2) {
            return self.accept_as(s, client, raw, now, false, mapped);
        }
        let opened = seal::open_task(enc, text).ok().filter(|(inner, outer)| seal::inner_times(inner).as_ref() == Some(outer));
        match opened {
            Some((inner, _)) => self.accept_as(s, client, inner.as_bytes(), now, true, mapped),
            None => {
                let why = if s.killed {
                    Rejection::Killed
                } else if !s.on || !s.consented {
                    Rejection::Off
                } else {
                    Rejection::BadSeal
                };
                let named = parse_client_name(client).ok().filter(|n| n == client).unwrap_or_default();
                self.note(Activity { at: now, ok: false, kind: String::new(), reason: why.code().into(), id: String::new(), client: named, runbook: String::new() });
                self.save();
                Err(why)
            }
        }
    }

    /// Every check, in order, for a task the relay says `client` queued; the
    /// outcome is noted either way. As if a Claude account were mapped to
    /// `client` (tests only; the app goes through `accept_sealed`).
    #[cfg(test)]
    pub fn accept_from(&mut self, s: &RemoteSettings, client: &str, raw: &[u8], now: i64) -> Result<Accepted, Rejection> {
        self.accept_as(s, client, raw, now, false, true)
    }

    /// `accept_from` for a client with (`mapped`) or without a Claude account.
    #[cfg(test)]
    pub fn accept_mapped(&mut self, s: &RemoteSettings, client: &str, raw: &[u8], now: i64, mapped: bool) -> Result<Accepted, Rejection> {
        self.accept_as(s, client, raw, now, false, mapped)
    }

    /// `accept_from` for a task that came `sealed` (opened by accept_sealed)
    /// or not.
    fn accept_as(&mut self, s: &RemoteSettings, client: &str, raw: &[u8], now: i64, sealed: bool, mapped: bool) -> Result<Accepted, Rejection> {
        // Forget what has expired (plus the skew) and counts over an hour old.
        self.seen.retain(|_, exp| *exp + SKEW_S >= now);
        self.recent.retain(|c| c.at > now - 3600);
        self.restarts.retain(|t| *t > now - 3600);
        let named = parse_client_name(client).ok().filter(|n| n == client).unwrap_or_default();
        let r = self.check(s, &named, raw, now, sealed, mapped);
        let entry = match &r {
            Ok(a) => Activity { at: now, ok: true, kind: a.command.kind().into(), reason: String::new(), id: a.id.clone(), client: named.clone(), runbook: runbook_of(&a.command) },
            Err((why, id)) => Activity { at: now, ok: false, kind: String::new(), reason: why.code().into(), id: id.clone().unwrap_or_default(), client: named.clone(), runbook: String::new() },
        };
        self.note(entry);
        // A verified task's id gets exactly one outcome (review 2026-11-17):
        // one refused for now (rate-limited, not yet valid …) is burned too,
        // so a copy the relay kept can't run later, after its signer was
        // told it didn't (and likely sent it again under a new id). Kept as
        // long as any task can live.
        if let Err((_, Some(id))) = &r {
            self.seen.entry(id.clone()).or_insert(now + MAX_LIFETIME_S);
        }
        if let Ok(a) = &r {
            self.seen.insert(a.id.clone(), a.expires);
            self.recent.push(Call { at: now, start: matches!(a.command, Command::StartSession { .. }), client: a.client.clone(), runbook: matches!(a.command, Command::Runbook { .. }) });
            if matches!(a.command, Command::KeepaliveRestart { .. }) {
                self.restarts.push(now);
            }
        }
        self.save();
        r.map_err(|(why, _)| why)
    }

    /// The checks. The id comes back with a rejection only once the
    /// signature verified.
    fn check(&self, s: &RemoteSettings, client: &str, raw: &[u8], now: i64, sealed: bool, mapped: bool) -> Result<Accepted, (Rejection, Option<String>)> {
        if s.killed {
            return Err((Rejection::Killed, None));
        }
        if !s.on || !s.consented {
            return Err((Rejection::Off, None));
        }
        let clients = s.clients();
        if clients.is_empty() {
            return Err((Rejection::NoKey, None));
        }
        let mine = clients.iter().find(|c| c.name == client).ok_or((Rejection::UnknownClient, None))?;
        let key = from_hex(&mine.public_key).filter(|k| k.len() == 32).ok_or((Rejection::NoKey, None))?;
        if raw.len() > MAX_ENVELOPE_BYTES {
            return Err((Rejection::TooBig, None));
        }
        let env: Envelope = serde_json::from_slice(raw).map_err(|_| (Rejection::BadEnvelope, None))?;
        if env.v != 1 {
            return Err((Rejection::BadEnvelope, None));
        }
        let sig = from_hex(&env.sig).filter(|s| s.len() == 64).ok_or((Rejection::BadSignature, None))?;
        let verifies = |k: &[u8]| ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, k).verify(env.payload.as_bytes(), &sig).is_ok();
        if !verifies(key.as_slice()) {
            // Another client's key: a task one client signed, queued with
            // another's token. Refused all the same, with its own code.
            let other = clients.iter().filter(|c| c.name != client).any(|c| from_hex(&c.public_key).filter(|k| k.len() == 32).is_some_and(|k| verifies(k.as_slice())));
            return Err((if other { Rejection::WrongClient } else { Rejection::BadSignature }, None));
        }
        // Signed by this client's key from here on.
        let p: Payload = serde_json::from_str(&env.payload).map_err(|_| (Rejection::BadPayload, None))?;
        let id_ok = (16..=64).contains(&p.id.len()) && p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !id_ok {
            return Err((Rejection::BadId, None));
        }
        let id = Some(p.id.clone());
        // Queued while the kill switch was on: the relay hands such tasks
        // over when the PC reconnects after "Allow again". Answered as
        // killed, never run. No clock-skew allowance here: a fresh task from
        // a clock running behind is refused and simply sent again.
        if p.iat < s.allowed_since {
            return Err((Rejection::Killed, id));
        }
        if p.exp - p.iat > MAX_LIFETIME_S || p.exp < p.iat {
            return Err((Rejection::TooLong, id));
        }
        if p.iat > now + SKEW_S {
            return Err((Rejection::NotYetValid, id));
        }
        // No skew after the expiry (owner 2026-11-17): a task the relay
        // already let go of as expired never runs here.
        if now > p.exp {
            return Err((Rejection::Expired, id));
        }
        if self.seen.contains_key(&p.id) {
            return Err((Rejection::Replay, id));
        }
        // "Require sealed tasks" (owner 2026-11-19): only a sealed task that
        // names its result key. Checked once the signature verified and after
        // killed / expired / replay (their codes say more), so the id is burned (one outcome per id) and the refusal is signed.
        if s.require_sealed && (!sealed || p.rk.is_none()) {
            return Err((Rejection::NotSealed, id));
        }
        let rk = match p.rk.as_deref() {
            None => None,
            Some(h) => Some(seal::from_hex(h).and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or((Rejection::BadPayload, id.clone()))?),
        };
        let command = command::parse(&p.cmd, &p.args).map_err(|e| (e, id.clone()))?;
        // Per-client commands (owner 2026-11-21): signed, so the id is burned
        // and the refusal is signed and listed like any other.
        if !mine.allows(command.kind()) {
            return Err((Rejection::NotAllowed, id));
        }
        if ACCOUNT_COMMANDS.contains(&command.kind()) && !mapped {
            return Err((Rejection::NoAccount, id));
        }
        if matches!(command, Command::StartSession { .. } | Command::SendToSession { .. }) && p.exp - p.iat > MAX_LIVE_LIFETIME_S {
            return Err((Rejection::TooLong, id));
        }
        let own = || self.recent.iter().filter(|c| c.client == client);
        if self.recent.len() >= MAX_COMMANDS_PER_HOUR_PC || own().count() >= MAX_COMMANDS_PER_HOUR {
            return Err((Rejection::RateLimited, id));
        }
        if matches!(command, Command::StartSession { .. })
            && (self.recent.iter().filter(|c| c.start).count() >= MAX_START_PER_HOUR_PC || own().filter(|c| c.start).count() >= MAX_START_PER_HOUR)
        {
            return Err((Rejection::RateLimited, id));
        }
        if matches!(command, Command::KeepaliveRestart { .. }) && self.restarts.len() >= MAX_KEEPALIVE_RESTARTS_PER_HOUR {
            return Err((Rejection::RateLimited, id));
        }
        // Remote runbooks (owner 2026-11-20, D6): the owner's limits from
        // Settings, inside the command caps above.
        if matches!(command, Command::Runbook { .. }) {
            let (mine, pc) = s.runbook_limits();
            if self.recent.iter().filter(|c| c.runbook).count() >= pc as usize || own().filter(|c| c.runbook).count() >= mine as usize {
                return Err((Rejection::RateLimited, id));
            }
        }
        Ok(Accepted { id: p.id, expires: p.exp, iat: p.iat, command, client: client.to_string(), rk })
    }

    /// The kill switch was used: noted in the list (it refuses on its own,
    /// through the settings).
    pub fn note_kill(&mut self, now: i64, killed: bool) {
        self.note(Activity { at: now, ok: !killed, kind: if killed { "kill-switch".into() } else { "allowed-again".into() }, reason: String::new(), id: String::new(), client: String::new(), runbook: String::new() });
        self.save();
    }
}

/// The runbook a task names, for the Remote activity list.
fn runbook_of(c: &Command) -> String {
    match c {
        Command::Runbook { name } => name.clone(),
        _ => String::new(),
    }
}

/// The process's gate, loaded on first use.
pub fn with<R>(f: impl FnOnce(&mut Gate) -> R) -> R {
    static GATE: std::sync::Mutex<Option<Gate>> = std::sync::Mutex::new(None);
    let mut g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let gate = g.get_or_insert_with(|| Gate::load(Gate::default_path()));
    f(gate)
}

pub fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
