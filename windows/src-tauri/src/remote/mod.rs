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
//   most 10 minutes after issue (60 s clock skew allowed), one of five
//   commands with its arguments (command.rs).
// - Replay protection: an id is accepted once; seen ids are kept (on disk,
//   so a restart doesn't reopen them) until their expiry has passed.
// - Rate limits, per rolling hour: at most 5 start-session, 30 commands.
// - Repo allowlist: commands that name a repo only for REPOS
//   (local-companion to start).
// - The channel is OFF by default and needs its one-time consent card; the
//   kill switch (tray + Settings, one click) refuses everything until it is
//   switched back in Settings.
// - Every decision goes to the "Remote activity" list (Settings) and
//   glim.log: time, outcome, the command kind or the rejection's reason
//   code. Never a payload, an argument, a signature or a key; for a task
//   whose signature didn't verify, not even its id.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod command;
pub mod courier;
pub mod exec;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod courier_tests;

pub use command::Command;

/// Repos a remote command may name (owner 2026-11-14: Glim's own repo only).
pub const REPOS: &[&str] = &["local-companion"];
/// A task lives at most this long (seconds).
pub const MAX_LIFETIME_S: i64 = 600;
/// Clock skew tolerated between the signer and this PC (seconds).
pub const SKEW_S: i64 = 60;
pub const MAX_START_PER_HOUR: usize = 5;
pub const MAX_COMMANDS_PER_HOUR: usize = 30;
/// The biggest envelope read at all (bytes).
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024;
/// Activity entries kept.
pub const ACTIVITY_KEPT: usize = 50;

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
    /// The signer's Ed25519 public key, 64 lower-case hex characters ("" none).
    pub public_key: String,
    /// The relay's address, "https://<name>.<subdomain>.workers.dev" ("" none).
    pub relay: String,
    /// The PC's result key (public half, 64 hex) once set up ("" none); the
    /// private half is only in Credential Manager (courier::PC_KEY_TARGET).
    pub pc_public_key: String,
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
}

/// A task that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub id: String,
    pub expires: i64,
    pub command: Command,
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
}

/// Replay cache, rate counters and the activity list. `path` keeps the
/// seen ids and the activity across restarts (None in tests that don't
/// need it).
#[derive(Debug, Default)]
pub struct Gate {
    /// id → its expiry.
    seen: BTreeMap<String, i64>,
    /// Accepted commands' times (start-session flagged).
    recent: Vec<(i64, bool)>,
    pub activity: Vec<Activity>,
    path: Option<std::path::PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    seen: BTreeMap<String, i64>,
    recent: Vec<(i64, bool)>,
    activity: Vec<Activity>,
}

impl Gate {
    /// The gate kept in `path` (`%LOCALAPPDATA%\Glim\remote-state.json`).
    pub fn load(path: std::path::PathBuf) -> Gate {
        let saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Gate { seen: saved.seen, recent: saved.recent, activity: saved.activity, path: Some(path) }
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::settings::local_dir().join("remote-state.json")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let saved = Saved { seen: self.seen.clone(), recent: self.recent.clone(), activity: self.activity.clone() };
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

    /// Every check, in order; the outcome is noted either way.
    pub fn accept(&mut self, s: &RemoteSettings, raw: &[u8], now: i64) -> Result<Accepted, Rejection> {
        // Forget what has expired (plus the skew) and counts over an hour old.
        self.seen.retain(|_, exp| *exp + SKEW_S >= now);
        self.recent.retain(|(t, _)| *t > now - 3600);
        let r = self.check(s, raw, now);
        let entry = match &r {
            Ok(a) => Activity { at: now, ok: true, kind: a.command.kind().into(), reason: String::new(), id: a.id.clone() },
            Err((why, id)) => Activity { at: now, ok: false, kind: String::new(), reason: why.code().into(), id: id.clone().unwrap_or_default() },
        };
        self.note(entry);
        if let Ok(a) = &r {
            self.seen.insert(a.id.clone(), a.expires);
            self.recent.push((now, matches!(a.command, Command::StartSession { .. })));
        }
        self.save();
        r.map_err(|(why, _)| why)
    }

    /// The checks. The id comes back with a rejection only once the
    /// signature verified.
    fn check(&self, s: &RemoteSettings, raw: &[u8], now: i64) -> Result<Accepted, (Rejection, Option<String>)> {
        if s.killed {
            return Err((Rejection::Killed, None));
        }
        if !s.on || !s.consented {
            return Err((Rejection::Off, None));
        }
        let key = from_hex(&s.public_key).filter(|k| k.len() == 32).ok_or((Rejection::NoKey, None))?;
        if raw.len() > MAX_ENVELOPE_BYTES {
            return Err((Rejection::TooBig, None));
        }
        let env: Envelope = serde_json::from_slice(raw).map_err(|_| (Rejection::BadEnvelope, None))?;
        if env.v != 1 {
            return Err((Rejection::BadEnvelope, None));
        }
        let sig = from_hex(&env.sig).filter(|s| s.len() == 64).ok_or((Rejection::BadSignature, None))?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
            .verify(env.payload.as_bytes(), &sig)
            .map_err(|_| (Rejection::BadSignature, None))?;
        // Signed by the owner's key from here on.
        let p: Payload = serde_json::from_str(&env.payload).map_err(|_| (Rejection::BadPayload, None))?;
        let id_ok = (16..=64).contains(&p.id.len()) && p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !id_ok {
            return Err((Rejection::BadId, None));
        }
        let id = Some(p.id.clone());
        if p.exp - p.iat > MAX_LIFETIME_S || p.exp < p.iat {
            return Err((Rejection::TooLong, id));
        }
        if p.iat > now + SKEW_S {
            return Err((Rejection::NotYetValid, id));
        }
        if now > p.exp + SKEW_S {
            return Err((Rejection::Expired, id));
        }
        if self.seen.contains_key(&p.id) {
            return Err((Rejection::Replay, id));
        }
        let command = command::parse(&p.cmd, &p.args).map_err(|e| (e, id.clone()))?;
        if self.recent.len() >= MAX_COMMANDS_PER_HOUR {
            return Err((Rejection::RateLimited, id));
        }
        if matches!(command, Command::StartSession { .. }) && self.recent.iter().filter(|(_, start)| *start).count() >= MAX_START_PER_HOUR {
            return Err((Rejection::RateLimited, id));
        }
        Ok(Accepted { id: p.id, expires: p.exp, command })
    }

    /// The kill switch was used: noted in the list (it refuses on its own,
    /// through the settings).
    pub fn note_kill(&mut self, now: i64, killed: bool) {
        self.note(Activity { at: now, ok: !killed, kind: if killed { "kill-switch".into() } else { "allowed-again".into() }, reason: String::new(), id: String::new() });
        self.save();
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
