// The remote courier (Phase 1, owner 2026-11-14): Glim's end of the relay.
// docs/remote-courier.md has the whole picture; in short:
//
// - The PC connects OUT to the relay (net::relay, the one WebSocket Glim
//   opens) with its own relay token, only while Remote tasks are on,
//   consented, not stopped, and set up (relay address, token, keys).
// - Each task the relay hands over goes through `Gate::accept_sealed` (signature,
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
use super::seal;
use super::{Accepted, Rejection, RemoteSettings};
use crate::ai::key::Vault;
use crate::net::relay::{self, End, Timing};
use crate::net::Url;

/// Glim's own credentials (secrets::RESERVED lists them).
pub const PC_TOKEN_TARGET: &str = "Glim/RemoteRelayPcToken";
pub const PC_KEY_TARGET: &str = "Glim/RemotePcSigningKey";
/// The PC's encryption key (X25519, 64 hex): tasks are sealed to its public
/// half (store-and-forward, owner 2026-11-17).
pub const PC_ENC_TARGET: &str = "Glim/RemotePcEncKey";
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
    /// The PC's encryption key (public, 64 hex): GLIM_PC_ENC_KEY.
    pub pc_enc_key: String,
}

/// A new PC relay token, a new PC result key and a new PC encryption key.
/// The token and the private keys go to Credential Manager, the token's hash
/// to Settings → Secrets. Cloud tokens are each cloud client's own
/// (`new_cloud_token`), untouched.
pub fn setup(pc_token: &dyn Vault, pc_key: &dyn Vault, pc_enc: &dyn Vault, secrets: &dyn crate::secrets::Store) -> Result<Setup, String> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| "no randomness".to_string())?;
    let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "bad key".to_string())?;
    let pc = random_hex32()?;
    let enc = random_hex32()?;
    let enc_public = seal::secret_from_hex(&enc).map(|s| seal::public_hex(&s)).ok_or("bad key")?;
    pc_key.write(&hex(pkcs8.as_ref()))?;
    pc_enc.write(&enc)?;
    pc_token.write(&pc)?;
    crate::secrets::save(secrets, PC_TOKEN_HASH_SECRET, &sha256_hex(&pc))?;
    Ok(Setup { pc_public_key: hex(kp.public_key().as_ref()), pc_enc_key: enc_public })
}

/// The PC's encryption key from its vault entry.
pub fn load_enc(v: &dyn Vault) -> Option<seal::Secret> {
    v.read().ok().flatten().and_then(|h| seal::secret_from_hex(h.trim()))
}

/// The PC's encryption key's public half, made first if there is none yet
/// (a PC set up before 2026-11-17 keeps its token and result key). Made
/// ONLY when the vault says there is no entry: a read that fails, or an
/// entry that doesn't parse, is an error and nothing is written, so a
/// passing Credential Manager hiccup never silently replaces the key every
/// cloud environment seals to (review 2026-11-17).
pub fn ensure_enc_key(v: &dyn Vault) -> Result<String, String> {
    match v.read() {
        Ok(Some(h)) => return seal::secret_from_hex(h.trim()).map(|s| seal::public_hex(&s)).ok_or_else(|| "unreadable".to_string()),
        Ok(None) => {}
        Err(_) => return Err("unreadable".into()),
    }
    let enc = random_hex32()?;
    let s = seal::secret_from_hex(&enc).ok_or("bad key")?;
    v.write(&enc)?;
    Ok(seal::public_hex(&s))
}

pub fn pc_enc_vault() -> impl Vault {
    crate::ai::key::CredentialManager { target: PC_ENC_TARGET }
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

/// What a result says about its task besides the outcome (result format,
/// owner 2026-11-19): who sent it, when it was made, started and finished,
/// and the Claude Code session it started or continued.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// The cloud client the relay named (a refusal's too: unverified, it's
    /// the relay's word; a verified task's is the one whose key signed it).
    pub client: String,
    /// The task's own `iat`, once it verified; None for a refusal.
    pub created: Option<i64>,
    /// When its command began on this PC; None when it never ran.
    pub started: Option<i64>,
    /// The Claude Code session it started (start-session with a prompt) or
    /// continued (send-to-session).
    pub session: Option<String>,
    /// The detail was cut to fit (12,000 characters, or the relay's 66 KiB).
    pub capped: bool,
}

/// A signed result envelope: `{"v":1,"payload":"<json>","sig":"<hex>"}`,
/// payload `{"v":1,"id","task","at","ok","kind","detail","error","client",
/// "status","created","started","finished","session","capped"}`. `task` is
/// the SHA-256 of the exact envelope answered: a refusal of an envelope the
/// relay made up (with a real task's id) can't pass for an answer to the
/// real task, because cloud Glim checks `task` against what it sent.
/// `status`: done (ok), failed (it ran and failed) or refused (it never
/// ran); `finished` = `at`. Every detail is redacted (`exec::redact_detail`)
/// and capped at 12,000 characters.
pub fn sign_result(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>) -> String {
    sign_result_meta(kp, id, task, at, outcome, &Meta::default())
}

pub fn sign_result_meta(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>, m: &Meta) -> String {
    let (ok, kind, detail, error, capped) = match outcome {
        Ok((kind, detail)) => {
            let clean = super::exec::redact_detail(detail);
            // Cut here, or already cut before (tail_of's leading "…").
            let over = clean.chars().count() > super::exec::MAX_DETAIL_CHARS || clean.starts_with('…');
            (true, kind.as_str(), cap(clean), "", m.capped || over)
        }
        Err(code) => (false, "", String::new(), code.as_str(), false),
    };
    let status = if ok {
        "done"
    } else if m.started.is_some() {
        "failed"
    } else {
        "refused"
    };
    let payload = json!({
        "v": 1, "id": id, "task": task, "at": at, "ok": ok, "kind": kind, "detail": detail, "error": error,
        "client": m.client, "status": status, "created": m.created, "started": m.started, "finished": at,
        "session": m.session, "capped": capped,
    })
    .to_string();
    let sig = kp.sign(payload.as_bytes());
    json!({ "v": 1, "payload": payload, "sig": hex(sig.as_ref()) }).to_string()
}

/// A client name the relay gave, as a refusal may repeat it: a valid name
/// (`parse_client_name`, at most 32 characters) or "" (review 2026-11-19:
/// never an arbitrary relay string under the PC's signature).
pub fn relay_named(raw: &str) -> String {
    super::parse_client_name(raw).ok().filter(|n| n == raw).unwrap_or_default()
}

/// The frame that carries a result back to the relay.
pub fn result_frame(id: &str, envelope: &str) -> String {
    json!({ "t": "result", "id": id, "env": envelope }).to_string()
}

/// A result as it goes back: signed, then sealed to the task's result key
/// `rk` when the (verified) task named one (store-and-forward; the relay
/// keeps it until acknowledged and can't read it). A key that can't be
/// sealed to answers `bad-result-key`; a sealed result too big for the
/// relay answers with its newest part that fits (`fit_detail`), and only
/// when not even that fits, `result-too-big` (still sealed when it can be).
/// The 12,000-character cap is on characters, but quotes, backslashes and
/// control characters grow when escaped twice (payload, then envelope) and
/// sealing doubles every byte (hex): a quote-heavy transcript-tail used to
/// come back as `result-too-big` with nothing in it (review 2026-11-17).
pub fn answer(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>, rk: Option<&[u8; 32]>) -> String {
    answer_meta(kp, id, task, at, outcome, rk, &Meta::default())
}

pub fn answer_meta(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>, rk: Option<&[u8; 32]>, m: &Meta) -> String {
    let env = build(kp, id, task, at, outcome, rk, m);
    if env.len() <= MAX_RESULT_BYTES {
        return env;
    }
    if let Ok((kind, detail)) = outcome {
        let trimmed = Meta { capped: true, ..m.clone() };
        let clean = cap(super::exec::redact_detail(detail));
        if let Some(env) = fit_detail(&clean, |d| build(kp, id, task, at, &Ok((kind.clone(), d.to_string())), rk, &trimmed)) {
            return env;
        }
    }
    build(kp, id, task, at, &Err("result-too-big".into()), rk, m)
}

/// The most a result envelope may be at the relay (its MAX_RESULT_BYTES),
/// sealed or not.
pub const MAX_RESULT_BYTES: usize = seal::MAX_SEALED_RESULT_BYTES;

/// A result signed, then sealed when `rk` is given; no size check.
fn build(kp: &Ed25519KeyPair, id: &str, task: &str, at: i64, outcome: &Result<(String, String), String>, rk: Option<&[u8; 32]>, m: &Meta) -> String {
    let signed = sign_result_meta(kp, id, task, at, outcome, m);
    let Some(rk) = rk else { return signed };
    match seal::seal(rk, "result", id, signed.as_bytes()) {
        None => sign_result_meta(kp, id, task, at, &Err("bad-result-key".into()), m),
        Some(s) => seal::result_envelope(id, &s),
    }
}

/// The last `n` characters of `chars`, starting at a whole line when a line
/// break is in them (a transcript-tail keeps whole messages, newest last),
/// marked "…" when anything was left out.
pub fn tail_chars(chars: &[char], n: usize) -> String {
    if n >= chars.len() {
        return chars.iter().collect();
    }
    let start = chars.len() - n;
    let mut part = &chars[start..];
    // Already at a line's start (the character before is a break): keep it.
    let at_line_start = start > 0 && chars[start - 1] == '\n';
    if !at_line_start {
        if let Some(i) = part.iter().position(|&c| c == '\n') {
            if i + 1 < part.len() {
                part = &part[i + 1..];
            }
        }
    }
    format!("…{}", part.iter().collect::<String>())
}

/// The envelope for the longest tail of `detail` (`tail_chars`) whose
/// envelope `make` builds within MAX_RESULT_BYTES: a binary search on the
/// characters kept (about 16 builds for 12,000). None when not even the
/// marker alone fits.
pub fn fit_detail(detail: &str, make: impl Fn(&str) -> String) -> Option<String> {
    let chars: Vec<char> = detail.chars().collect();
    // `hi` is known too big; `lo` is the most known to fit.
    let (mut lo, mut hi) = (0usize, chars.len());
    let mut best = {
        let env = make(&tail_chars(&chars, 0));
        (env.len() <= MAX_RESULT_BYTES).then_some(env)
    };
    best.as_ref()?;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let env = make(&tail_chars(&chars, mid));
        if env.len() <= MAX_RESULT_BYTES {
            lo = mid;
            best = Some(env);
        } else {
            hi = mid;
        }
    }
    best
}

/// The id a relay frame's envelope names, before anything is verified (the
/// result must name it for the relay to match it up). Only a well-formed id;
/// a sealed task's is the one outside the seal.
fn peek_id(env: &str) -> Option<String> {
    if seal::version(env) == Some(2) {
        return seal::outer_id(env);
    }
    let v: Value = serde_json::from_str(env).ok()?;
    let p: Value = serde_json::from_str(v.get("payload")?.as_str()?).ok()?;
    let id = p.get("id")?.as_str()?;
    let ok = (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| id.to_string())
}

/// The answers this PC gave, kept on disk until each task's expiry
/// (`%LOCALAPPDATA%\Glim\remote-answers.json`; store-and-forward, review
/// 2026-11-17): a task the relay hands over again (after a restart, a lost
/// line, or while it still runs) gets its FIRST answer again, or nothing
/// while it runs, never a `replay` refusal for something that ran. Only
/// sealed results and fixed-code refusals are kept; an unsealed success
/// (an older courier's task) is kept as a marker only, answered
/// `already-ran`: no result text ever reaches the disk.
#[derive(Debug, Default)]
pub struct Ledger {
    path: Option<std::path::PathBuf>,
    entries: Vec<LedgerEntry>,
    /// Envelope hashes whose command is running now (memory only).
    running: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LedgerEntry {
    key: String,
    exp: i64,
    frame: Option<String>,
}

/// What the ledger knows about an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Running,
    /// The frame sent for it; None = it ran, its unsealed result not kept.
    Answered(Option<String>),
}

const LEDGER_KEPT: usize = 100;

impl Ledger {
    pub fn load(path: std::path::PathBuf) -> Ledger {
        let entries = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Ledger { path: Some(path), entries, running: Vec::new() }
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::settings::local_dir().join("remote-answers.json")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Ok(bytes) = serde_json::to_vec(&self.entries) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    pub fn lookup(&mut self, key: &str, now: i64) -> Option<Seen> {
        if self.running.iter().any(|k| k == key) {
            return Some(Seen::Running);
        }
        let before = self.entries.len();
        self.entries.retain(|e| e.exp >= now);
        if self.entries.len() != before {
            self.save();
        }
        self.entries.iter().find(|e| e.key == key).map(|e| Seen::Answered(e.frame.clone()))
    }

    pub fn start(&mut self, key: &str) {
        self.running.push(key.to_string());
    }

    /// `frame`: what to send again (None: only that it ran).
    pub fn finish(&mut self, key: &str, exp: i64, frame: Option<String>) {
        self.running.retain(|k| k != key);
        self.entries.retain(|e| e.key != key);
        self.entries.push(LedgerEntry { key: key.to_string(), exp, frame });
        let extra = self.entries.len().saturating_sub(LEDGER_KEPT);
        self.entries.drain(..extra);
        self.save();
    }
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
    ledger: Arc<Mutex<Ledger>>,
}

/// sha256(envelope) → the result frame sent for it (the last 50).
pub type Answered = Arc<Mutex<VecDeque<(String, String)>>>;

impl Courier {
    /// `answered` outlives connections (a task redelivered after a reconnect
    /// must find its first result).
    pub fn new(signer: Arc<Ed25519KeyPair>, exec: Arc<dyn Exec>, accept: Arc<AcceptFn>, tx: tokio::sync::mpsc::UnboundedSender<String>, answered: Answered) -> Courier {
        Courier { signer, exec, accept, tx, answered, ledger: Arc::default() }
    }

    /// The ledger that outlives restarts (the app's; tests keep theirs in memory).
    pub fn with_ledger(mut self, ledger: Arc<Mutex<Ledger>>) -> Courier {
        self.ledger = ledger;
        self
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
        // Phone approvals (owner 2026-11-21): the page's answers, which the
        // relay can't forge (approve.rs verifies them).
        let t = f.get("t").and_then(Value::as_str).unwrap_or("");
        if t == "approval-answer" || t == "pair-answer" {
            let id = f.get("id").and_then(Value::as_str).unwrap_or("");
            let answer = f.get("answer").and_then(Value::as_str).unwrap_or("");
            if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
                return;
            }
            if t == "approval-answer" {
                super::exec::on_approval_answer(id, answer, now);
            } else {
                super::approve::on_pair_answer(id, answer, now, &send_frame);
            }
            return;
        }
        if t != "task" {
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
        // Handed over again after a restart or a lost line: the first
        // answer again, or nothing while it still runs (its result follows).
        let seen = self.ledger.lock().unwrap_or_else(|e| e.into_inner()).lookup(&key, now);
        match seen {
            Some(Seen::Running) => return,
            Some(Seen::Answered(Some(frame))) => {
                let _ = self.tx.send(frame);
                return;
            }
            Some(Seen::Answered(None)) => {
                let m = Meta { client: relay_named(f.get("client").and_then(Value::as_str).unwrap_or(super::LEGACY_CLIENT)), ..Meta::default() };
                let _ = self.tx.send(result_frame(&id, &sign_result_meta(&self.signer, &id, &key, now, &Err("already-ran".into()), &m)));
                return;
            }
            None => {}
        }
        // A relay from before cloud clients names none: the old single one.
        let client = f.get("client").and_then(Value::as_str).unwrap_or(super::LEGACY_CLIENT);
        match (self.accept)(settings, client, env.as_bytes(), now) {
            Err(why) => {
                let m = Meta { client: relay_named(client), ..Meta::default() };
                let frame = result_frame(&id, &sign_result_meta(&self.signer, &id, &key, now, &Err(why.code().to_string()), &m));
                Self::remember(&self.answered, key.clone(), &frame);
                // A fixed code only: kept, so the same envelope gets the same answer.
                self.ledger.lock().unwrap_or_else(|e| e.into_inner()).finish(&key, now + super::MAX_LIFETIME_S, Some(frame.clone()));
                let _ = self.tx.send(frame);
            }
            Ok(task) => {
                self.ledger.lock().unwrap_or_else(|e| e.into_inner()).start(&key);
                let ledger = self.ledger.clone();
                let (signer, exec, tx, answered) = (self.signer.clone(), self.exec.clone(), self.tx.clone(), self.answered.clone());
                std::thread::spawn(move || {
                    let kind = task.command.kind().to_string();
                    let started = super::now_s();
                    let ran = exec.run(&task.command, &task.client);
                    let session = ran.as_ref().ok().and_then(|r| r.session.clone());
                    let out = ran.map(|r| (kind, r.detail));
                    let m = Meta { client: task.client.clone(), created: Some(task.iat), started: Some(started), session, capped: false };
                    if let Err(code) = &out {
                        crate::log::line(format!("remote: task failed {}", code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect::<String>()));
                    }
                    let frame = result_frame(&task.id, &answer_meta(&signer, &task.id, &key, super::now_s(), &out, task.rk.as_ref(), &m));
                    // Sealed results are kept to send again; an unsealed one only as "it ran".
                    let keep = task.rk.is_some().then(|| frame.clone());
                    ledger.lock().unwrap_or_else(|e| e.into_inner()).finish(&key, task.expires, keep);
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
    /// The answers given (on disk in the app, `Ledger::default_path`).
    pub ledger: Arc<Mutex<Ledger>>,
}

/// The courier's outbox, for frames the PC starts (approval requests,
/// pairing slots, voids); None until the courier runs.
static OUTBOX: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>> = Mutex::new(None);

/// Sends a frame to the relay now, or on the next connection.
pub fn send_frame(frame: String) {
    if let Some(tx) = OUTBOX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(frame);
    }
}

/// Runs until `running` says stop.
pub async fn run(l: Loop) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    *OUTBOX.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
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
                let courier = Courier::new(link.signer.clone(), l.exec.clone(), l.accept.clone(), tx.clone(), answered.clone()).with_ledger(l.ledger.clone());
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
pub fn start(
    settings: Arc<dyn Fn() -> RemoteSettings + Send + Sync>,
    keepalive_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    accounts: Box<dyn Fn() -> Vec<crate::accounts::ClaudeAccount> + Send + Sync>,
    phone: Arc<dyn Fn(String, String) + Send + Sync>,
) {
    super::exec::set_hooks(super::exec::Hooks { settings: settings.clone(), send: Arc::new(send_frame), phone });
    // The gate asks whether a Claude account is mapped to a task's client
    // (session commands, owner 2026-11-21); exec reads the same list.
    let accounts: Arc<dyn Fn() -> Vec<crate::accounts::ClaudeAccount> + Send + Sync> = Arc::from(accounts);
    let gate_accounts = accounts.clone();
    let link = Arc::new(|s: &RemoteSettings| -> Result<Link, &'static str> {
        let url = relay::endpoint(&s.relay).map_err(|_| "no-relay")?;
        let token = pc_token_vault().read().ok().flatten().filter(|t| t.len() == 64).ok_or("not-set-up")?;
        let signer = load_signer(&pc_key_vault()).ok_or("not-set-up")?;
        Ok(Link { url, token, signer: Arc::new(signer) })
    });
    let l = Loop {
        settings,
        link,
        // A sealed task is opened with the PC's encryption key, read from
        // Credential Manager when one arrives (never kept in memory between).
        accept: Arc::new(move |s: &RemoteSettings, client: &str, raw: &[u8], now: i64| {
            let enc = if seal::version(std::str::from_utf8(raw).unwrap_or("")) == Some(2) { load_enc(&pc_enc_vault()) } else { None };
            let mapped = !client.is_empty() && crate::accounts::for_client(&gate_accounts(), client).is_some();
            // An agent client's lease (owner 2026-11-21, lease.rs).
            let leased = super::lease::with(|l| l.active_for(client, now).is_some());
            let r = super::with(|g| g.accept_sealed(s, client, raw, now, enc.as_ref(), mapped, leased));
            if let Ok(a) = &r {
                let book = match &a.command {
                    super::Command::Runbook { name } => name.clone(),
                    _ => String::new(),
                };
                super::lease::with(|l| l.note(&a.client, a.command.kind(), &book, now));
            }
            r
        }),
        exec: Arc::new(super::exec::RealExec { keepalive_folders, accounts: Box::new(move || accounts()) }),
        timing: Timing::default(),
        backoff_max: Duration::from_secs(60),
        idle: Duration::from_secs(2),
        running: Arc::new(|| true),
        ledger: Arc::new(Mutex::new(Ledger::load(Ledger::default_path()))),
    };
    super::wake::start();
    // Leases end on time even when nobody sends anything (owner 2026-11-21):
    // each one's summary goes to Remote activity.
    let _ = std::thread::Builder::new().name("remote-leases".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_secs(30));
        super::lease_sweep(super::now_s());
    });
    let _ = std::thread::Builder::new().name("remote-courier".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(run(l));
    });
}
