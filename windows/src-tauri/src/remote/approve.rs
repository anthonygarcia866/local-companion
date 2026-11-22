// Phone approval of a paused remote runbook step (owner 2026-11-21,
// docs/remote-runbooks.md §8). SECURITY-CRITICAL: changes to this file (the
// PC's verification) merge only with CI green and a fresh-agent security
// review finding nothing critical or high (CLAUDE.md, CODEOWNERS).
//
// - Approvers are WebAuthn credentials (a synced Face ID passkey now; a
//   hardware security key can be added the same way later, no rebuild):
//   their P-256 public key and id live in settings.json, the 32-byte key
//   their requests are sealed with only in Credential Manager
//   (Glim/RemoteApproverKeys). Enrolled and removed ONLY on this PC
//   (Settings → Remote): no courier command reaches any of this.
// - Pairing: this PC opens a one-time slot at the relay and shows a QR code
//   (the approval page's address, the slot id, the relay's host and this
//   PC's one-time P-256 key). The page creates the passkey, derives the key
//   from its PRF output, proves the passkey with an assertion, and seals all
//   of it to the PC's key. The owner compares the fingerprint on both
//   screens and confirms here.
// - A request: a paused run's details (request id, run id, runbook, its
//   approved hash, step, step kind, client, expiry, a random match code),
//   sealed per approver; Pushover gets the runbook, step, client and match
//   code as text and a link carrying the request id plus the relay's host
//   SEALED to each approver (never the host in clear). The passkey signs
//   SHA-256 of exactly those details; this PC rebuilds them from its own
//   checkpoint and verifies everything (origin, RP id, user presence and
//   verification, challenge, signature). One answer per request; it expires
//   with the run; the kill switch voids it.
// - Leases (owner 2026-11-21, remote/lease.rs): an agent client asks for a
//   1/4/8/12 h lease; the request (kind "lease": request id, client, hours,
//   its expiry, match code, the lock slot) is sealed and signed the same way,
//   over its own binding (`glim-lease-v1`, never `glim-approve-v1`: one can't
//   stand in for the other). Granting needs a verified Approve. A lease also
//   gets a LOCK slot: any answer to it ends the lease (like Stop, it needs no
//   signature: whoever has that link can only lock).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::runs::Run;
use super::seal;

/// The approval page (GitHub Pages from this repo, `approve/`).
pub const PAGE_ORIGIN: &str = "https://anthonygarcia866.github.io";
pub const RP_ID: &str = "anthonygarcia866.github.io";
pub const PAGE_URL: &str = "https://anthonygarcia866.github.io/local-companion/";
pub const MAX_APPROVERS: usize = 3;
pub const PAIR_TTL_S: i64 = 10 * 60;
pub const KEYS_TARGET: &str = "Glim/RemoteApproverKeys";
/// Match codes: no 0/O, 1/I/L.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// One approver: a WebAuthn credential this PC trusts to OK a paused step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Approver {
    /// The credential id, base64url.
    pub id: String,
    /// A name the owner gave it ("iPhone").
    pub name: String,
    /// Its P-256 public key, uncompressed point, hex (130 characters).
    pub public_key: String,
    /// "passkey" or "security-key".
    pub kind: String,
    /// Backup-eligible (a synced passkey: on every device of that Apple ID).
    pub synced: bool,
    /// Unix seconds.
    pub added: i64,
}

impl Approver {
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&seal::from_hex(&self.public_key).unwrap_or_default())
    }
}

/// SHA-256 of the public point, the first 8 bytes as 4 groups of 4 hex
/// (upper case): what the page shows at pairing too.
pub fn fingerprint_of(point: &[u8]) -> String {
    let h = Sha256::digest(point);
    let hex: String = h[..8].iter().map(|b| format!("{b:02X}")).collect();
    hex.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
}

// ── base64url (no padding) ────────────────────────────────────────────────────

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64url(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

pub fn from_b64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = B64.iter().position(|x| *x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // Leftover bits must be zero padding only.
    if bits >= 6 || acc & ((1 << bits) - 1) != 0 {
        return None;
    }
    Some(out)
}

fn random<const N: usize>() -> Option<[u8; N]> {
    use ring::rand::SecureRandom;
    let mut b = [0u8; N];
    ring::rand::SystemRandom::new().fill(&mut b).ok()?;
    Some(b)
}

pub fn new_slot_id() -> Option<String> {
    random::<16>().map(|b| seal::to_hex(&b))
}

/// "ABC-DEF": 6 characters from 31 (about 30 bits), for the owner's eyes.
pub fn new_match_code() -> Option<String> {
    let b = random::<6>()?;
    let c: String = b.iter().map(|x| CODE_ALPHABET[*x as usize % CODE_ALPHABET.len()] as char).collect();
    Some(format!("{}-{}", &c[..3], &c[3..]))
}

// ── WebAuthn assertions ───────────────────────────────────────────────────────

/// What the page posts: base64url authenticator data, client data JSON and
/// the DER signature, and the credential id.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Assertion {
    pub cred: String,
    pub ad: String,
    pub cd: String,
    pub sig: String,
}

/// Why an assertion was refused (codes for the log, never the data).
pub type Refused = &'static str;

/// Checks a WebAuthn assertion against `point` (uncompressed P-256) for
/// exactly `challenge`: client data type webauthn.get, the challenge, the
/// page's origin, not cross-origin; authenticator data for our RP id with
/// user presence AND user verification (Face ID); then the ES256 signature
/// over authenticatorData || SHA-256(clientDataJSON). Returns whether the
/// credential is backup-eligible (synced).
pub fn verify(point: &[u8], a: &Assertion, challenge: &[u8; 32]) -> Result<bool, Refused> {
    let ad = from_b64url(&a.ad).ok_or("bad-encoding")?;
    let cd = from_b64url(&a.cd).ok_or("bad-encoding")?;
    let sig = from_b64url(&a.sig).ok_or("bad-encoding")?;
    let client: Value = serde_json::from_slice(&cd).map_err(|_| "bad-client-data")?;
    if client.get("type").and_then(Value::as_str) != Some("webauthn.get") {
        return Err("bad-type");
    }
    if client.get("challenge").and_then(Value::as_str) != Some(b64url(challenge).as_str()) {
        return Err("bad-challenge");
    }
    if client.get("origin").and_then(Value::as_str) != Some(PAGE_ORIGIN) {
        return Err("bad-origin");
    }
    if client.get("crossOrigin").and_then(Value::as_bool) == Some(true) {
        return Err("cross-origin");
    }
    if ad.len() < 37 || ad[..32] != Sha256::digest(RP_ID.as_bytes())[..] {
        return Err("bad-rp");
    }
    let flags = ad[32];
    if flags & 0x01 == 0 || flags & 0x04 == 0 {
        return Err("no-user-verification");
    }
    let mut msg = ad.clone();
    msg.extend_from_slice(&Sha256::digest(&cd));
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point).verify(&msg, &sig).map_err(|_| "bad-signature")?;
    Ok(flags & 0x08 != 0)
}

/// The uncompressed P-256 point inside a SubjectPublicKeyInfo (what the
/// page's `getPublicKey()` gives): the fixed 26-byte EC P-256 header, then
/// the 65-byte point.
pub fn point_from_spki(spki: &[u8]) -> Option<Vec<u8>> {
    const HEAD: [u8; 26] = [0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00];
    (spki.len() == 91 && spki[..26] == HEAD && spki[26] == 4).then(|| spki[26..].to_vec())
}

// ── Pairing (this PC only) ────────────────────────────────────────────────────

/// What Settings shows about a pairing in progress.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairView {
    /// "none" | "waiting" | "ready" (compare the fingerprint, then confirm) | "failed" | "expired".
    pub state: String,
    /// The page's address for the QR code (the slot id, relay host and the
    /// PC's one-time key are in its fragment, which never leaves the phone).
    pub url: String,
    pub qr_svg: String,
    pub expires: i64,
    pub fingerprint: String,
    pub synced: bool,
}

struct Pending {
    approver: Approver,
    key: [u8; 32],
}

struct Pairing {
    id: String,
    exp: i64,
    key: Option<seal::PairKey>,
    url: String,
    qr_svg: String,
    pending: Option<Pending>,
    failed: bool,
}

static PAIRING: std::sync::Mutex<Option<Pairing>> = std::sync::Mutex::new(None);

fn pairing<R>(f: impl FnOnce(&mut Option<Pairing>) -> R) -> R {
    f(&mut PAIRING.lock().unwrap_or_else(|e| e.into_inner()))
}

pub fn pair_challenge(id: &str) -> [u8; 32] {
    Sha256::digest(format!("glim-pair-v1|{id}").as_bytes()).into()
}

/// Settings → Remote → "Pair a phone": opens a 10-minute slot at the relay
/// (`send`: the courier's outbox) and returns the QR code to show.
pub fn pair_start(relay_host: &str, now: i64, send: &dyn Fn(String)) -> Result<PairView, String> {
    if relay_host.is_empty() {
        return Err("no-relay".into());
    }
    let id = new_slot_id().ok_or("no-randomness")?;
    let key = seal::pair_key().ok_or("no-randomness")?;
    let url = format!("{PAGE_URL}#pair={id}&relay={relay_host}&pk={}", b64url(&key.public));
    let qr_svg = qr_svg(&url)?;
    let exp = now + PAIR_TTL_S;
    pairing(|p| {
        if let Some(old) = p.take() {
            send(json!({ "t": "pair-void", "id": old.id }).to_string());
        }
        *p = Some(Pairing { id: id.clone(), exp, key: Some(key), url: url.clone(), qr_svg: qr_svg.clone(), pending: None, failed: false });
    });
    send(json!({ "t": "pair", "id": id, "exp": exp }).to_string());
    crate::log::line("remote: phone pairing started");
    Ok(pair_view(now))
}

pub(crate) fn qr_svg(text: &str) -> Result<String, String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).map_err(|_| "qr-too-long".to_string())?;
    Ok(code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
}

pub fn pair_view(now: i64) -> PairView {
    pairing(|p| match p {
        None => PairView { state: "none".into(), ..PairView::default() },
        Some(x) => {
            let state = if x.pending.is_some() {
                "ready"
            } else if x.failed {
                "failed"
            } else if now > x.exp {
                "expired"
            } else {
                "waiting"
            };
            let (fingerprint, synced) = x.pending.as_ref().map(|q| (q.approver.fingerprint(), q.approver.synced)).unwrap_or_default();
            PairView { state: state.into(), url: x.url.clone(), qr_svg: x.qr_svg.clone(), expires: x.exp, fingerprint, synced }
        }
    })
}

/// The page's sealed pairing answer, as the relay hands it over.
pub fn on_pair_answer(id: &str, answer: &str, now: i64, send: &dyn Fn(String)) {
    let outcome = pairing(|p| {
        let Some(x) = p.as_mut().filter(|x| x.id == id) else { return Err("unknown-pairing") };
        if now > x.exp || x.pending.is_some() || x.failed {
            return Err("pairing-closed");
        }
        // One try: a bad answer ends this pairing (start again on the PC).
        let key = x.key.take().ok_or("pairing-closed")?;
        let r = open_pair_answer(key, id, answer, now);
        match r {
            Ok(pending) => {
                x.pending = Some(pending);
                Ok(())
            }
            Err(e) => {
                x.failed = true;
                Err(e)
            }
        }
    });
    send(json!({ "t": "pair-ack", "id": id }).to_string());
    match outcome {
        Ok(()) => crate::log::line("remote: phone pairing answered; waiting for the owner to compare fingerprints"),
        Err(e) => crate::log::line(format!("remote: phone pairing refused {e}")),
    }
}

#[derive(Deserialize)]
struct SealedAnswer {
    epk: String,
    n: String,
    ct: String,
}

#[derive(Deserialize)]
struct PairPlain {
    cred: String,
    spki: String,
    key: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    ad: String,
    cd: String,
    sig: String,
}

fn open_pair_answer(key: seal::PairKey, id: &str, answer: &str, now: i64) -> Result<Pending, Refused> {
    let s: SealedAnswer = serde_json::from_str(answer).map_err(|_| "bad-answer")?;
    let epk = from_b64url(&s.epk).ok_or("bad-answer")?;
    let nonce: [u8; 12] = from_b64url(&s.n).and_then(|n| n.try_into().ok()).ok_or("bad-answer")?;
    let ct = from_b64url(&s.ct).ok_or("bad-answer")?;
    let plain = seal::pair_open(key, id, &epk, &nonce, &ct).ok_or("bad-seal")?;
    let p: PairPlain = serde_json::from_slice(&plain).map_err(|_| "bad-answer")?;
    let point = point_from_spki(&from_b64url(&p.spki).ok_or("bad-answer")?).ok_or("not-p256")?;
    let shared: [u8; 32] = from_b64url(&p.key).and_then(|k| k.try_into().ok()).ok_or("bad-key")?;
    if from_b64url(&p.cred).is_none_or(|c| c.is_empty() || c.len() > 255) {
        return Err("bad-credential");
    }
    // The passkey must sign this pairing's challenge, with Face ID.
    let synced = verify(&point, &Assertion { cred: p.cred.clone(), ad: p.ad, cd: p.cd, sig: p.sig }, &pair_challenge(id))?;
    let name: String = p.name.chars().filter(|c| c.is_alphanumeric() || " -'".contains(*c)).take(30).collect();
    let kind = if p.kind == "security-key" { "security-key" } else { "passkey" };
    Ok(Pending { approver: Approver { id: p.cred, name: if name.trim().is_empty() { "Phone".into() } else { name.trim().into() }, public_key: seal::to_hex(&point), kind: kind.into(), synced, added: now }, key: shared })
}

/// The owner compared the fingerprints and confirmed: the approver to add
/// and its key (the caller saves both; at most MAX_APPROVERS).
pub fn pair_take(send: &dyn Fn(String)) -> Option<(Approver, [u8; 32])> {
    pairing(|p| {
        let x = p.take()?;
        send(json!({ "t": "pair-void", "id": x.id }).to_string());
        x.pending.map(|q| (q.approver, q.key))
    })
}

pub fn pair_cancel(send: &dyn Fn(String)) {
    pairing(|p| {
        if let Some(x) = p.take() {
            send(json!({ "t": "pair-void", "id": x.id }).to_string());
        }
    });
}

// ── Approver keys (Credential Manager) ────────────────────────────────────────

/// id → key (hex), one JSON line in Credential Manager.
pub fn read_keys(v: &dyn crate::ai::key::Vault) -> std::collections::BTreeMap<String, String> {
    v.read().ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_keys(v: &dyn crate::ai::key::Vault, keys: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    if keys.is_empty() {
        return v.delete();
    }
    v.write(&serde_json::to_string(keys).map_err(|e| e.to_string())?)
}

pub fn keys_vault() -> impl crate::ai::key::Vault {
    crate::ai::key::CredentialManager { target: KEYS_TARGET }
}

// ── Requests ──────────────────────────────────────────────────────────────────

/// A paused run's request, as signed: the challenge is SHA-256 of exactly
/// these lines (the page builds them from the details it shows).
pub fn binding(rid: &str, run: &Run, code: &str) -> String {
    format!("glim-approve-v1\n{rid}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{code}", run.id, run.name, run.hash, run.step, run.step_kind, run.total, run.client, run.expires)
}

pub fn challenge(rid: &str, run: &Run, code: &str) -> [u8; 32] {
    Sha256::digest(binding(rid, run, code).as_bytes()).into()
}

/// What goes out for one request.
pub struct Request {
    /// The frame for the relay: `{t:"approval", id, exp, body}`.
    pub frame: String,
    /// Pushover's text: runbook, step, client and match code only.
    pub text: String,
    /// The link: the page, the request id, the relay host sealed per approver.
    pub url: String,
}

/// Builds the request for a paused run: details sealed to each approver
/// with a key, the relay's host sealed too (for the link).
pub fn request(rid: &str, code: &str, run: &Run, approvers: &[(Approver, [u8; 32])], relay_host: &str) -> Option<Request> {
    if approvers.is_empty() {
        return None;
    }
    let details = json!({ "v": 1, "rid": rid, "run": run.id, "runbook": run.name, "hash": run.hash, "step": run.step, "kind": run.step_kind, "total": run.total, "client": run.client, "exp": run.expires, "code": code }).to_string();
    let mut items = Vec::new();
    let mut hints = Vec::new();
    for (a, key) in approvers {
        let (n, ct) = seal::approve_seal(key, &format!("glim-approve-v1|{rid}"), details.as_bytes())?;
        items.push(json!({ "c": a.id, "n": b64url(&n), "ct": b64url(&ct) }));
        let (hn, hct) = seal::approve_seal(key, &format!("glim-approve-v1 relay|{rid}"), relay_host.as_bytes())?;
        hints.push(json!({ "c": a.id.chars().take(12).collect::<String>(), "n": b64url(&hn), "ct": b64url(&hct) }));
    }
    let body = json!({ "v": 1, "rid": rid, "exp": run.expires, "items": items }).to_string();
    let frame = json!({ "t": "approval", "id": rid, "exp": run.expires, "body": body }).to_string();
    let url = format!("{PAGE_URL}#r={rid}&h={}", b64url(Value::Array(hints).to_string().as_bytes()));
    let text = format!("Glim: runbook {} step {} of {} ({}) for {} waits for your OK. Match code {code}. Expires in 24 h.", run.name, run.step, run.total, run.step_kind, run.client);
    Some(Request { frame, text, url })
}

// ── Leases ────────────────────────────────────────────────────────────────────

/// A lease request, as signed: its own first line, so a runbook approval can
/// never be replayed as a lease (or the other way round).
pub fn lease_binding(p: &super::lease::Pending) -> String {
    format!("glim-lease-v1\n{}\n{}\n{}\n{}\n{}\n{}", p.rid, p.client, p.hours, p.expires, p.lock, p.code)
}

pub fn lease_challenge(p: &super::lease::Pending) -> [u8; 32] {
    Sha256::digest(lease_binding(p).as_bytes()).into()
}

/// Seals `details` (and the relay host, for the link) to each approver for
/// slot `rid`, as `request` does for a paused run.
fn sealed_slot(rid: &str, exp: i64, details: &str, approvers: &[(Approver, [u8; 32])], relay_host: &str) -> Option<(String, String)> {
    if approvers.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    let mut hints = Vec::new();
    for (a, key) in approvers {
        let (n, ct) = seal::approve_seal(key, &format!("glim-approve-v1|{rid}"), details.as_bytes())?;
        items.push(json!({ "c": a.id, "n": b64url(&n), "ct": b64url(&ct) }));
        let (hn, hct) = seal::approve_seal(key, &format!("glim-approve-v1 relay|{rid}"), relay_host.as_bytes())?;
        hints.push(json!({ "c": a.id.chars().take(12).collect::<String>(), "n": b64url(&hn), "ct": b64url(&hct) }));
    }
    let body = json!({ "v": 1, "rid": rid, "exp": exp, "items": items }).to_string();
    let frame = json!({ "t": "approval", "id": rid, "exp": exp, "body": body }).to_string();
    let url = format!("{PAGE_URL}#r={rid}&h={}", b64url(Value::Array(hints).to_string().as_bytes()));
    Some((frame, url))
}

/// Runbook names a lease card lists at most (the rest as "+N more").
pub const LEASE_RUNBOOKS_SHOWN: usize = 20;

/// A lease request's slot: the frame, Pushover's text and the link.
/// `runbooks`: the remote-level runbooks ("name (cloud|phone)") a leased agent could start as the
/// owner right now (PC-built, owner 2026-11-22: the card must say so); shown
/// on the card, not part of the signed binding.
pub fn lease_request(p: &super::lease::Pending, runbooks: &[String], approvers: &[(Approver, [u8; 32])], relay_host: &str) -> Option<Request> {
    let shown: Vec<&String> = runbooks.iter().take(LEASE_RUNBOOKS_SHOWN).collect();
    let more = runbooks.len().saturating_sub(shown.len());
    let details = json!({ "v": 1, "kind": "lease", "rid": p.rid, "client": p.client, "hours": p.hours, "exp": p.expires, "lock": p.lock, "code": p.code, "runbooks": shown, "more": more }).to_string();
    let (frame, url) = sealed_slot(&p.rid, p.expires, &details, approvers, relay_host)?;
    let text = format!("Glim: agent client {} asks for a {} h lease. Match code {}. The request lapses in 30 min.", p.client, p.hours, p.code);
    Some(Request { frame, text, url })
}

/// A granted lease's lock slot: lives until the lease ends; any answer to it
/// locks the client now. Its details say which client (sealed, as ever).
pub fn lock_slot(lock: &str, client: &str, until: i64, approvers: &[(Approver, [u8; 32])], relay_host: &str) -> Option<(String, String)> {
    let details = json!({ "v": 1, "kind": "lease-lock", "rid": lock, "client": client, "exp": until }).to_string();
    sealed_slot(lock, until, &details, approvers, relay_host)
}

/// Judges an answer to a lease request: Approve needs a known approver's
/// assertion over exactly this request (`lease_binding`).
pub fn judge_lease(answer: &str, p: &super::lease::Pending, approvers: &[Approver]) -> Verdict {
    judge_with(answer, &lease_challenge(p), approvers)
}

/// How an answer went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    Stop,
    /// Anything that didn't verify (its code): the request is used up.
    Refused(Refused),
}

#[derive(Deserialize)]
struct Answer {
    answer: String,
    #[serde(flatten)]
    assertion: Assertion,
}

/// Judges the page's answer to request `rid` of the paused `run`: Stop
/// needs nothing (anyone with the link could only ever stop a run);
/// Approve needs an assertion from a known approver over exactly this
/// request's details.
pub fn judge(answer: &str, rid: &str, run: &Run, code: &str, approvers: &[Approver]) -> Verdict {
    judge_with(answer, &challenge(rid, run, code), approvers)
}

/// Stop needs nothing; Approve needs a known approver's assertion over
/// `challenge` (SHA-256 of exactly the details the page showed).
fn judge_with(answer: &str, challenge: &[u8; 32], approvers: &[Approver]) -> Verdict {
    let Ok(a) = serde_json::from_str::<Answer>(answer) else { return Verdict::Refused("bad-answer") };
    match a.answer.as_str() {
        "stop" => Verdict::Stop,
        "approve" => {
            let Some(who) = approvers.iter().find(|x| x.id == a.assertion.cred) else { return Verdict::Refused("unknown-approver") };
            let Some(point) = seal::from_hex(&who.public_key) else { return Verdict::Refused("bad-key") };
            match verify(&point, &a.assertion, challenge) {
                Ok(_) => Verdict::Approve,
                Err(e) => Verdict::Refused(e),
            }
        }
        _ => Verdict::Refused("bad-answer"),
    }
}

#[cfg(test)]
#[path = "approve_tests.rs"]
mod tests;
