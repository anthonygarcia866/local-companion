// Sealing for the courier's store-and-forward (owner 2026-11-17,
// docs/courier-store-and-forward.md). The relay keeps tasks for up to 72 h
// and results until their client acknowledges them, so both are sealed: a
// task to this PC's encryption key (X25519, `Glim/RemotePcEncKey` in
// Credential Manager, public half in Settings → Remote as GLIM_PC_ENC_KEY),
// a result to the per-task result key the (signed) task names (`rk`).
//
// One construction both ways, the same bytes as relay/courier.mjs (shared
// vectors in courier_tests.rs and relay/test/relay.test.mjs):
//   shared = X25519(ephemeral, recipient)          (refused if all zero)
//   key    = HKDF-SHA256(shared, salt = epk || recipient,
//                        info = "glim-courier-v2 <task|result>|<id>")
//   ct     = ChaCha20-Poly1305(key, 12-byte random nonce,
//                              aad = the same info string, plaintext)
// The plaintext is always a SIGNED v1 envelope: sealing hides it, the
// signature inside says who wrote it (anyone can seal to a public key).
// The labels keep task and result keys apart; the cloud side derives its
// per-task keys from its signing seed with labels of their own and never
// uses the seed itself as a key.
//
// The only code using x25519-dalek and ring's AEAD / HKDF (a test scans).
// Nothing here is logged.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::hkdf;
use ring::rand::SecureRandom;
use serde_json::{json, Value};
use x25519_dalek::{PublicKey, StaticSecret};

/// An X25519 secret (the PC's encryption key), for code outside this file.
pub type Secret = StaticSecret;

/// The HKDF salt for derived keys and the prefix of every label.
pub const LABEL: &str = "glim-courier-v2";
/// A sealed task, read at all (a signed one is 16 KB at most, in hex).
pub const MAX_SEALED_TASK_BYTES: usize = 34 * 1024;
/// The relay keeps results up to this size (relay/src/core.js).
pub const MAX_SEALED_RESULT_BYTES: usize = 66 * 1024;

struct Len32;

impl hkdf::KeyType for Len32 {
    fn len(&self) -> usize {
        32
    }
}

/// HKDF-SHA256, 32 bytes.
pub fn hkdf32(ikm: &[u8], salt: &[u8], info: &str) -> [u8; 32] {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let info = [info.as_bytes()];
    let mut out = [0u8; 32];
    // 32 bytes is always a valid HKDF-SHA256 output length.
    if let Ok(okm) = prk.expand(&info, Len32) {
        let _ = okm.fill(&mut out);
    }
    out
}

fn info(what: &str, id: &str) -> String {
    format!("{LABEL} {what}|{id}")
}

fn aead_key(shared: &[u8; 32], epk: &[u8; 32], recipient: &[u8; 32], what: &str, id: &str) -> Option<LessSafeKey> {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(epk);
    salt[32..].copy_from_slice(recipient);
    let k = hkdf32(shared, &salt, &info(what, id));
    UnboundKey::new(&CHACHA20_POLY1305, &k).ok().map(LessSafeKey::new)
}

/// What a seal carries besides the ciphertext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub epk: [u8; 32],
    pub nonce: [u8; 12],
    /// Ciphertext with its 16-byte tag at the end.
    pub ct: Vec<u8>,
}

/// Seals `plain` to `recipient` with the ephemeral secret `eph` and `nonce`
/// (fresh random ones in the app, `seal`; fixed ones in the vectors).
pub fn seal_with(eph: [u8; 32], nonce: [u8; 12], recipient: &[u8; 32], what: &str, id: &str, plain: &[u8]) -> Option<Sealed> {
    let e = StaticSecret::from(eph);
    let epk = PublicKey::from(&e).to_bytes();
    let shared = e.diffie_hellman(&PublicKey::from(*recipient));
    if !shared.was_contributory() {
        return None;
    }
    let key = aead_key(shared.as_bytes(), &epk, recipient, what, id)?;
    let mut buf = plain.to_vec();
    let aad = info(what, id);
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad.as_bytes()), &mut buf).ok()?;
    Some(Sealed { epk, nonce, ct: buf })
}

/// Seals with a fresh ephemeral key and nonce.
pub fn seal(recipient: &[u8; 32], what: &str, id: &str, plain: &[u8]) -> Option<Sealed> {
    let rng = ring::rand::SystemRandom::new();
    let (mut eph, mut nonce) = ([0u8; 32], [0u8; 12]);
    rng.fill(&mut eph).ok()?;
    rng.fill(&mut nonce).ok()?;
    seal_with(eph, nonce, recipient, what, id, plain)
}

/// Opens what was sealed to `me` from `epk`; None on any failure (wrong key,
/// altered bytes, another id or label, a low-order `epk`).
pub fn open(me: &StaticSecret, epk: &[u8; 32], nonce: &[u8; 12], what: &str, id: &str, ct: &[u8]) -> Option<Vec<u8>> {
    let shared = me.diffie_hellman(&PublicKey::from(*epk));
    if !shared.was_contributory() {
        return None;
    }
    let mine = PublicKey::from(me).to_bytes();
    let key = aead_key(shared.as_bytes(), epk, &mine, what, id)?;
    let mut buf = ct.to_vec();
    let aad = info(what, id);
    let plain = key.open_in_place(Nonce::assume_unique_for_key(*nonce), Aad::from(aad.as_bytes()), &mut buf).ok()?;
    Some(plain.to_vec())
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn fixed<const N: usize>(s: &str) -> Option<[u8; N]> {
    from_hex(s)?.try_into().ok()
}

/// A raw X25519 secret from its hex (the vault's form).
pub fn secret_from_hex(s: &str) -> Option<StaticSecret> {
    fixed::<32>(s).map(StaticSecret::from)
}

pub fn public_hex(s: &StaticSecret) -> String {
    to_hex(PublicKey::from(s).as_bytes())
}

/// A sealed task's outside: what the relay reads. The PC checks it matches
/// the signed payload inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outer {
    pub id: String,
    pub iat: i64,
    pub exp: i64,
}

/// Why a sealed task couldn't be opened (answered unsealed, as "bad-seal").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadSeal;

fn id_ok(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The task envelope's version: Some(2) for a sealed one.
pub fn version(raw: &str) -> Option<u64> {
    serde_json::from_str::<Value>(raw).ok()?.get("v")?.as_u64()
}

/// The well-formed id outside a sealed task (unverified), for answering it.
pub fn outer_id(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let id = v.get("id")?.as_str()?;
    id_ok(id).then(|| id.to_string())
}

/// Opens a sealed task (`{v:2, id, iat, exp, epk, n, ct}`) with the PC's
/// key: the signed v1 envelope inside, and the outside to check against it.
pub fn open_task(me: Option<&StaticSecret>, raw: &str) -> Result<(String, Outer), BadSeal> {
    if raw.len() > MAX_SEALED_TASK_BYTES {
        return Err(BadSeal);
    }
    let me = me.ok_or(BadSeal)?;
    let v: Value = serde_json::from_str(raw).map_err(|_| BadSeal)?;
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    let id = s("id").filter(|i| id_ok(i)).ok_or(BadSeal)?;
    let iat = v.get("iat").and_then(Value::as_i64).ok_or(BadSeal)?;
    let exp = v.get("exp").and_then(Value::as_i64).ok_or(BadSeal)?;
    let epk = s("epk").and_then(fixed::<32>).ok_or(BadSeal)?;
    let nonce = s("n").and_then(fixed::<12>).ok_or(BadSeal)?;
    let ct = s("ct").and_then(from_hex).ok_or(BadSeal)?;
    let plain = open(me, &epk, &nonce, "task", id, &ct).ok_or(BadSeal)?;
    let inner = String::from_utf8(plain).map_err(|_| BadSeal)?;
    Ok((inner, Outer { id: id.to_string(), iat, exp }))
}

/// The id, iat and exp of a v1 envelope's payload, unverified (the gate
/// verifies it next): to check a sealed task's outside against.
pub fn inner_times(inner: &str) -> Option<Outer> {
    let v: Value = serde_json::from_str(inner).ok()?;
    let p: Value = serde_json::from_str(v.get("payload")?.as_str()?).ok()?;
    Some(Outer { id: p.get("id")?.as_str()?.to_string(), iat: p.get("iat")?.as_i64()?, exp: p.get("exp")?.as_i64()? })
}

/// A signed result sealed to the task's result key: `{v:2, id, epk, n, ct}`.
pub fn result_envelope(id: &str, s: &Sealed) -> String {
    json!({ "v": 2, "id": id, "epk": to_hex(&s.epk), "n": to_hex(&s.nonce), "ct": to_hex(&s.ct) }).to_string()
}
