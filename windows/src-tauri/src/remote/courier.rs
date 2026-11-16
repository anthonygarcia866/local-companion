// The remote courier (Phase 1, owner 2026-11-14): Glim's end of the relay.
// docs/remote-courier.md has the whole picture; in short:
//
// - The PC connects OUT to the relay (net::relay, the one WebSocket Glim
//   opens) with its own relay token, only while Remote tasks are on,
//   consented, not stopped, and set up (relay address, token, keys).
// - Each task the relay hands over goes through `Gate::accept` (signature,
//   expiry, replay, command, repo, rate limits, kill switch), then exec.rs.
// - Every answer, accepted or refused, is a RESULT signed with the PC's own
//   Ed25519 key (made here, its private half only in Credential Manager as
//   Glim/RemotePcSigningKey): cloud Glim checks it with the PC's public key,
//   so the relay can't forge or alter one.
// - Reconnects: on a lost line (closed, 60 s silent, woke from sleep, or
//   kicked by Windows' resume / network-change events, wake.rs) right away,
//   then 2, 4 … 60 s apart; a kick or a wall-clock jump also ends a backoff
//   wait early. The relay keeps unanswered tasks until expiry and hands them
//   over on connect; results not yet sent wait here; a task seen again (same
//   bytes) gets its first result again, not a replay refusal, and a task id
//   accepted once is refused as a replay until it expires (never run twice).
// - Outages (owner 2026-11-15): with Remote tasks on and set up, a line down
//   for 10 minutes pings the phone once and its recovery once (`Outage`;
//   quiet hours respected, a stop by the owner is no outage).
// - The kill switch ends the connection within a tick and nothing reconnects
//   until "Allow again".
// - Logged: connection states and fixed codes only (the gate logs decisions);
//   never a task, a result, a token or a key.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use ring::rand::SecureRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::exec::{cap, Exec};
use super::{Accepted, Rejection, RemoteSettings};
use crate::ai::key::Vault;
use crate::net::relay::{self, End, Timing};
use crate::net::Url;

/// Glim's own credentials (secrets::RESERVED lists them).
pub const PC_TOKEN_TARGET: &str = "Glim/RemoteRelayPcToken";
pub const PC_KEY_TARGET: &str = "Glim/RemotePcSigningKey";
/// Ordinary secrets (Settings → Secrets) holding the tokens' SHA-256, so an
/// approved runbook (relay-tokens) can hand them to `wrangler secret put` on
/// stdin. Not secret in themselves (a hash of 32 random bytes).
pub const PC_TOKEN_HASH_SECRET: &str = "REMOTE_PC_TOKEN_SHA256";
/// The single cloud token's hash from before cloud clients (still the
/// relay's CLOUD_TOKEN_SHA256, "account-1"'s "legacy" entry). No longer
/// written.
pub const CLOUD_TOKEN_HASH_SECRET: &str = "REMOTE_CLOUD_TOKEN_SHA256";
/// The cloud clients' list for the relay ("name=hash,...", or "none"):
/// RemoteSettings::relay_list, rewritten on every add and remove.
pub const CLOUD_CLIENTS_SECRET: &str = "REMOTE_CLOUD_CLIENTS";
/// Results remembered for a task delivered twice.
const ANSWERED_KEPT: usize = 50;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn random_hex32() -> Result<String, String> {
    let mut b = [0u8; 32];
    ring::rand::SystemRandom::new().fill(&mut b).map_err(|_| "no randomness".to_string())?;
    Ok(hex(&b))
}

pub fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// What Settings shows after "Set up the relay".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    /// The PC's result key (public, 64 hex), for cloud Glim's environments.
    pub pc_public_key: String,
}

/// A new PC relay token and a new PC result key. The token and the private
/// key go to Credential Manager, the token's hash to Settings → Secrets.
/// Cloud tokens are each cloud client's own (`new_cloud_token`), untouched.
pub fn setup(pc_token: &dyn Vault, pc_key: &dyn Vault, secrets: &dyn crate::secrets::Store) -> Result<Setup, String> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| "no randomness".to_string())?;
    let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "bad key".to_string())?;
    let pc = random_hex32()?;
    pc_key.write(&hex(pkcs8.as_ref()))?;
    pc_token.write(&pc)?;
    crate::secrets::save(secrets, PC_TOKEN_HASH_SECRET, &sha256_hex(&pc))?;
    Ok(Setup { pc_public_key: hex(kp.public_key().as_ref()) })
}

/// A new cloud client's relay token: (token, its SHA-256). The token is
/// shown once and never kept on this PC; the hash goes into the client's
/// entry and the relay's list.
pub fn new_cloud_token() -> Result<(String, String), String> {
    let token = random_hex32()?;
    let hash = sha256_hex(&token);
    Ok((token, hash))
}

/// Writes the relay's list of cloud clients to its secret (for the
/// relay-tokens runbook).
pub fn save_client_list(s: &RemoteSettings, secrets: &dyn crate::secrets::Store) -> Result<(), String> {
    crate::secrets::save(secrets, CLOUD_CLIENTS_SECRET, &s.relay_list()).map(|_| ())
}

/// The PC's result key from its vault entry (PKCS#8, hex).
pub fn load_signer(v: &dyn Vault) -> Option<Ed25519KeyPair> {
    let h = v.read().ok().flatten()?;
    let bytes: Option<Vec<u8>> = (0..h.len()).step_by(2).map(|i| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok())).collect();
    Ed25519KeyPair::from_pkcs8(&bytes?).ok()
}

pub fn pc_token_vault() -> impl Vault {
    crate::ai::key::CredentialManager { target: PC_TOKEN_TARGET }
}

pub fn pc_key_vault() -> impl Vault {
    crate::ai::key::CredentialManager { target: PC_KEY_TARGET }
}

/// A signed result envelope: `{"v":1,"payload":"<json>","sig":"<hex>"}`,
/// payload `{"v":1,"id","task","at","ok","kind","detail","error"}`. `task` is
/// the SHA-256 of the exact envelope answered: a refusal of an envelope the
/// relay made up (with a real task's id) can't pass for an answer to the
/// real task, because cloud Glim checks `task` against what it sent.
pub fn sign_result(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>) -> String {
    let (ok, kind, detail, error) = match outcome {
        Ok((kind, detail)) => (true, kind.as_str(), cap(detail.clone()), ""),
        Err(code) => (false, "", String::new(), code.as_str()),
    };
    let payload = json!({ "v": 1, "id": id, "task": task, "at": at, "ok": ok, "kind": kind, "detail": detail, "error": error }).to_string();
    let sig = kp.sign(payload.as_bytes());
    json!({ "v": 1, "payload": payload, "sig": hex(sig.as_ref()) }).to_string()
}

/// The frame that carries a result back to the relay.
pub fn result_frame(id: &str, envelope: &str) -> String {
    json!({ "t": "result", "id": id, "env": envelope }).to_string()
}

/// The id a relay frame's envelope names, before anything is verified (the
/// result must name it for the relay to match it up). Only a well-formed id.
fn peek_id(env: &str) -> Option<String> {
    let v: Value = serde_json::from_str(env).ok()?;
    let p: Value = serde_json::from_str(v.get("payload")?.as_str()?).ok()?;
    let id = p.get("id")?.as_str()?;
    let ok = (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| id.to_string())
}

/// (settings, the client the relay named, the envelope, now).
pub type AcceptFn = dyn Fn(&RemoteSettings, &str, &[u8], i64) -> Result<Accepted, Rejection> + Send + Sync;

/// Turns relay frames into signed results on `tx`.
pub struct Courier {
    pub signer: Arc<Ed25519KeyPair>,
    pub exec: Arc<dyn Exec>,
    pub accept: Arc<AcceptFn>,
    pub tx: tokio::sync::mpsc::UnboundedSender<String>,
    answered: Answered,
}

/// sha256(envelope) → the result frame sent for it (the last 50).
pub type Answered = Arc<Mutex<VecDeque<(String, String)>>>;

impl Courier {
    /// `answered` outlives connections (a task redelivered after a reconnect
    /// must find its first result).
    pub fn new(signer: Arc<Ed25519KeyPair>, exec: Arc<dyn Exec>, accept: Arc<AcceptFn>, tx: tokio::sync::mpsc::UnboundedSender<String>, answered: Answered) -> Courier {
        Courier { signer, exec, accept, tx, answered }
    }

    fn remember(answered: &Mutex<VecDeque<(String, String)>>, key: String, frame: &str) {
        let mut a = answered.lock().unwrap_or_else(|e| e.into_inner());
        a.push_back((key, frame.to_string()));
        while a.len() > ANSWERED_KEPT {
            a.pop_front();
        }
    }

    /// One frame from the relay. Refusals are answered at once; accepted
    /// commands run on a thread of their own and answer when done.
    pub fn on_frame(&self, settings: &RemoteSettings, frame: &str, now: i64) {
        let Ok(f) = serde_json::from_str::<Value>(frame) else { return };
        if f.get("t").and_then(Value::as_str) != Some("task") {
            return;
        }
        let Some(env) = f.get("env").and_then(Value::as_str) else { return };
        let key = sha256_hex(env);
        let again = self.answered.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key).map(|(_, f)| f.clone());
        if let Some(frame) = again {
            let _ = self.tx.send(frame);
            return;
        }
        let Some(id) = peek_id(env) else { return };
        // A relay from before cloud clients names none: the old single one.
        let client = f.get("client").and_then(Value::as_str).unwrap_or(super::LEGACY_CLIENT);
        match (self.accept)(settings, client, env.as_bytes(), now) {
            Err(why) => {
                let frame = result_frame(&id, &sign_result(&self.signer, &id, &key, now, &Err(why.code().to_string())));
                Self::remember(&self.answered, key, &frame);
                let _ = self.tx.send(frame);
            }
            Ok(task) => {
                let (signer, exec, tx, answered) = (self.signer.clone(), self.exec.clone(), self.tx.clone(), self.answered.clone());
                std::thread::spawn(move || {
                    let kind = task.command.kind().to_string();
                    let out = exec.run(&task.command).map(|d| (kind, d));
                    if let Err(code) = &out {
                        crate::log::line(format!("remote: task failed {}", code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect::<String>()));
                    }
                    let frame = result_frame(&task.id, &sign_result(&signer, &task.id, &key, super::now_s(), &out));
                    Self::remember(&answered, key, &frame);
                    let _ = tx.send(frame);
                });
            }
        }
    }
}

/// Where and how to connect, read afresh for each connection.
pub struct Link {
    pub url: Url,
    pub token: String,
    pub signer: Arc<Ed25519KeyPair>,
}

/// The connection's state for Settings (a fixed code).
pub static STATE: Mutex<&'static str> = Mutex::new("off");

fn set_state(s: &'static str) {
    let mut cur = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if *cur != s {
        crate::log::line(format!("remote: relay {s}"));
        *cur = s;
    }
}

pub fn state() -> &'static str {
    *STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Allowed to connect at all (and nothing reconnects otherwise).
pub fn wanted(s: &RemoteSettings) -> bool {
    s.on && s.consented && !s.killed && !s.clients().is_empty() && !s.relay.is_empty()
}

pub struct Loop {
    pub settings: Arc<dyn Fn() -> RemoteSettings + Send + Sync>,
    pub link: Arc<dyn Fn(&RemoteSettings) -> Result<Link, &'static str> + Send + Sync>,
    pub accept: Arc<AcceptFn>,
    pub exec: Arc<dyn Exec>,
    pub timing: Timing,
    pub backoff_max: Duration,
    /// How often an idle loop looks at the settings again.
    pub idle: Duration,
    /// false ends the loop (tests); the app's is always true.
    pub running: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Runs until `running` says stop.
pub async fn run(l: Loop) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut unsent: Vec<String> = Vec::new();
    let answered: Answered = Arc::default();
    let mut failures: u32 = 0;
    while (l.running)() {
        let s = (l.settings)();
        relay::set_relay_allowed(wanted(&s));
        if !wanted(&s) {
            set_state("off");
            failures = 0;
            tokio::time::sleep(l.idle).await;
            continue;
        }
        let link = match (l.link)(&s) {
            Ok(link) => link,
            Err(code) => {
                set_state(code);
                tokio::time::sleep(l.idle).await;
                continue;
            }
        };
        set_state("connecting");
        match relay::connect(&link.url, &link.token).await {
            Err(code) => {
                set_state(code);
                failures += 1;
            }
            Ok(ws) => {
                set_state("connected");
                failures = 0;
                let courier = Courier::new(link.signer.clone(), l.exec.clone(), l.accept.clone(), tx.clone(), answered.clone());
                let settings = l.settings.clone();
                let mut on_frame = |f: &str| courier.on_frame(&settings(), f, super::now_s());
                let keep = || wanted(&(l.settings)()) && (l.running)();
                let io = relay::Io { outbox: &mut rx, unsent: &mut unsent, on_frame: &mut on_frame, keep_going: &keep };
                let began = std::time::Instant::now();
                match relay::session(ws, io, l.timing).await {
                    End::Stopped => {
                        set_state("off");
                        continue;
                    }
                    // Straight back after a drop, a sleep or a kick; backoff
                    // when a connection keeps dying young.
                    End::Lost(why) => {
                        crate::log::line(format!("remote: relay line lost ({why})"));
                        set_state(if matches!(why, "slept" | "resumed") { "woke-reconnecting" } else { "reconnecting" });
                        if began.elapsed() >= Duration::from_secs(5) || matches!(why, "resumed" | "network-changed") {
                            continue;
                        }
                        failures += 1;
                    }
                }
            }
        }
        let wait = Duration::from_secs(1u64 << failures.min(6)).min(l.backoff_max);
        let until = Instant::now() + wait;
        // Resumed or the network changed while waiting: try again now.
        let kicked = relay::kicks();
        let (mut wall, mut mono) = (SystemTime::now(), Instant::now());
        while Instant::now() < until && (l.running)() && wanted(&(l.settings)()) && relay::kicks() == kicked {
            tokio::time::sleep(l.idle.min(Duration::from_millis(500))).await;
            if relay::woke_from_sleep(wall, SystemTime::now(), mono.elapsed(), &l.timing) {
                break;
            }
            (wall, mono) = (SystemTime::now(), Instant::now());
        }
    }
    relay::set_relay_allowed(false);
}

/// A relay line down this long pings the phone (owner 2026-11-15).
pub const OUTAGE_ALERT_AFTER: Duration = Duration::from_secs(10 * 60);

/// What the outage watch says on the phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ping {
    Down,
    Back,
}

/// One relay outage at a time: at most one Down and one Back each.
#[derive(Debug, Default)]
pub struct Outage {
    down_since: Option<Instant>,
    told: bool,
}

impl Outage {
    /// One look: `wanted` (Remote tasks on, consented, not stopped, set up),
    /// `connected` (the line is up), `may_ping` (phone on, outside quiet
    /// hours). A Down not allowed yet waits for `may_ping`; so does its Back.
    pub fn tick(&mut self, wanted: bool, connected: bool, now: Instant, after: Duration, may_ping: bool) -> Option<Ping> {
        if !wanted {
            // Switched off or stopped by the owner: not an outage.
            *self = Outage::default();
            return None;
        }
        if connected {
            self.down_since = None;
            if self.told && may_ping {
                self.told = false;
                return Some(Ping::Back);
            }
            return None;
        }
        let since = *self.down_since.get_or_insert(now);
        if !self.told && may_ping && now.duration_since(since) >= after {
            self.told = true;
            return Some(Ping::Down);
        }
        None
    }
}

/// The outage watch's thread: every 15 s, the line's state against the
/// settings; `may_ping` and `ping` come from the app (phone.rs).
pub fn watch_outages(settings: Arc<dyn Fn() -> RemoteSettings + Send + Sync>, may_ping: Box<dyn Fn() -> bool + Send>, ping: Box<dyn Fn(Ping) + Send>) {
    let _ = std::thread::Builder::new().name("remote-outage-watch".into()).spawn(move || {
        let mut o = Outage::default();
        loop {
            std::thread::sleep(Duration::from_secs(15));
            let s = settings();
            if let Some(p) = o.tick(wanted(&s), state() == "connected", Instant::now(), OUTAGE_ALERT_AFTER, may_ping()) {
                crate::log::line(format!("remote: relay {} ping", if p == Ping::Down { "down" } else { "back" }));
                ping(p);
            }
        }
    });
}

/// The app's courier: one thread with its own runtime, for Glim's life.
pub fn start(settings: Arc<dyn Fn() -> RemoteSettings + Send + Sync>, keepalive_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>) {
    let link = Arc::new(|s: &RemoteSettings| -> Result<Link, &'static str> {
        let url = relay::endpoint(&s.relay).map_err(|_| "no-relay")?;
        let token = pc_token_vault().read().ok().flatten().filter(|t| t.len() == 64).ok_or("not-set-up")?;
        let signer = load_signer(&pc_key_vault()).ok_or("not-set-up")?;
        Ok(Link { url, token, signer: Arc::new(signer) })
    });
    let l = Loop {
        settings,
        link,
        accept: Arc::new(|s: &RemoteSettings, client: &str, raw: &[u8], now: i64| super::with(|g| g.accept_from(s, client, raw, now))),
        exec: Arc::new(super::exec::RealExec { keepalive_folders }),
        timing: Timing::default(),
        backoff_max: Duration::from_secs(60),
        idle: Duration::from_secs(2),
        running: Arc::new(|| true),
    };
    super::wake::start();
    let _ = std::thread::Builder::new().name("remote-courier".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(run(l));
    });
}
