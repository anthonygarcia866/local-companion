// remote/approve.rs: assertions, pairing and requests (owner 2026-11-21).

use super::*;
use crate::remote::runs::{Run, RunState};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

const NOW: i64 = 1_795_238_795;
const RID: &str = "0123456789abcdef0123456789abcdef";

struct Phone {
    kp: EcdsaKeyPair,
}

impl Phone {
    fn new() -> Phone {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        Phone { kp: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap() }
    }
    fn point(&self) -> Vec<u8> {
        self.kp.public_key().as_ref().to_vec()
    }
    fn spki(&self) -> Vec<u8> {
        let mut v = vec![0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00];
        v.extend_from_slice(&self.point());
        v
    }
    /// An assertion as Safari makes one: our RP id, the flags, the client data.
    fn assert_with(&self, challenge: &[u8; 32], origin: &str, rp: &str, flags: u8, kind: &str) -> Assertion {
        let mut ad = Sha256::digest(rp.as_bytes()).to_vec();
        ad.push(flags);
        ad.extend_from_slice(&[0, 0, 0, 0]);
        let cd = json!({ "type": kind, "challenge": b64url(challenge), "origin": origin, "crossOrigin": false }).to_string();
        let mut msg = ad.clone();
        msg.extend_from_slice(&Sha256::digest(cd.as_bytes()));
        let sig = self.kp.sign(&ring::rand::SystemRandom::new(), &msg).unwrap();
        Assertion { cred: "Y3JlZC0x".into(), ad: b64url(&ad), cd: b64url(cd.as_bytes()), sig: b64url(sig.as_ref()) }
    }
    fn assert(&self, challenge: &[u8; 32]) -> Assertion {
        // UP | UV | BE | BS: a synced passkey.
        self.assert_with(challenge, PAGE_ORIGIN, RP_ID, 0x1d, "webauthn.get")
    }
}

fn paused_run() -> Run {
    Run { id: "fedcba9876543210fedcba9876543210".into(), name: "glim-check".into(), client: "account-2".into(), hash: "ab".repeat(32), total: 5, done: 2, state: RunState::Paused, step: 3, step_kind: "run".into(), expires: NOW + 3600, ..Run::default() }
}

#[test]
fn base64url_round_trips_and_refuses_junk() {
    for n in 0..70u8 {
        let b: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
        assert_eq!(from_b64url(&b64url(&b)).unwrap(), b);
    }
    assert_eq!(b64url(b"hi?"), "aGk_");
    assert!(from_b64url("a+b/").is_none(), "standard base64 isn't base64url");
    assert!(from_b64url("A").is_none(), "a lone character");
}

#[test]
fn a_good_assertion_verifies_and_every_wrong_part_is_refused() {
    let phone = Phone::new();
    let ch = [7u8; 32];
    assert_eq!(verify(&phone.point(), &phone.assert(&ch), &ch), Ok(true), "synced passkey");
    assert_eq!(verify(&phone.point(), &phone.assert(&ch), &[8u8; 32]), Err("bad-challenge"));
    assert_eq!(verify(&phone.point(), &phone.assert_with(&ch, "https://evil.example", RP_ID, 0x05, "webauthn.get"), &ch), Err("bad-origin"));
    assert_eq!(verify(&phone.point(), &phone.assert_with(&ch, PAGE_ORIGIN, "evil.example", 0x05, "webauthn.get"), &ch), Err("bad-rp"));
    assert_eq!(verify(&phone.point(), &phone.assert_with(&ch, PAGE_ORIGIN, RP_ID, 0x01, "webauthn.get"), &ch), Err("no-user-verification"), "Face ID required");
    assert_eq!(verify(&phone.point(), &phone.assert_with(&ch, PAGE_ORIGIN, RP_ID, 0x05, "webauthn.create"), &ch), Err("bad-type"));
    assert_eq!(verify(&phone.point(), &phone.assert_with(&ch, PAGE_ORIGIN, RP_ID, 0x05, "webauthn.get"), &ch), Ok(false), "device-bound key (security key)");
    assert_eq!(verify(&Phone::new().point(), &phone.assert(&ch), &ch), Err("bad-signature"), "another key");
    let mut tampered = phone.assert(&ch);
    tampered.ad = b64url(&[from_b64url(&tampered.ad).unwrap(), vec![1]].concat());
    assert_eq!(verify(&phone.point(), &tampered, &ch), Err("bad-signature"));
    assert_eq!(point_from_spki(&phone.spki()), Some(phone.point()));
    assert_eq!(point_from_spki(&phone.spki()[1..]), None);
}

#[test]
fn pairing_needs_the_qr_codes_key_and_a_face_id_assertion_and_the_owner_confirms() {
    let sent = std::sync::Mutex::new(Vec::<String>::new());
    let send = |f: String| sent.lock().unwrap().push(f);
    let view = pair_start("task-relay.example.workers.dev", NOW, &send).unwrap();
    assert_eq!(view.state, "waiting");
    assert!(view.url.starts_with(PAGE_URL) && view.url.contains("#pair=") && view.qr_svg.starts_with("<?xml"));
    let frag: std::collections::HashMap<String, String> = view.url.split_once('#').unwrap().1.split('&').map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap()).collect();
    let id = frag["pair"].clone();
    let pc_pub = from_b64url(&frag["pk"]).unwrap();
    assert!(sent.lock().unwrap().iter().any(|f| f.contains("\"t\":\"pair\"") && f.contains(&id)));

    let phone = Phone::new();
    let key = [9u8; 32];
    let a = phone.assert(&pair_challenge(&id));
    let plain = json!({ "cred": a.cred, "spki": b64url(&phone.spki()), "key": b64url(&key), "name": "Example phone", "ad": a.ad, "cd": a.cd, "sig": a.sig }).to_string();
    let (epk, n, ct) = seal::pair_seal_for_test(&pc_pub, &id, plain.as_bytes()).unwrap();
    let answer = json!({ "epk": b64url(&epk), "n": b64url(&n), "ct": b64url(&ct) }).to_string();
    on_pair_answer(&id, &answer, NOW + 5, &send);
    let ready = pair_view(NOW + 6);
    assert_eq!(ready.state, "ready", "waits for the owner's fingerprint compare");
    assert_eq!(ready.fingerprint, fingerprint_of(&phone.point()));
    assert!(ready.synced);
    assert!(sent.lock().unwrap().iter().any(|f| f.contains("pair-ack")));
    let (approver, k) = pair_take(&send).unwrap();
    assert_eq!((approver.name.as_str(), approver.kind.as_str(), k), ("Example phone", "passkey", key));
    assert_eq!(pair_view(NOW + 7).state, "none");

    // A forged answer (not sealed to the QR's key, or the wrong challenge) ends the pairing.
    let view = pair_start("task-relay.example.workers.dev", NOW, &send).unwrap();
    let id2: String = view.url.split("pair=").nth(1).unwrap().chars().take(32).collect();
    let other = seal::pair_key().unwrap();
    let wrong_challenge = phone.assert(&pair_challenge("not-this-one"));
    let plain = json!({ "cred": "Y3JlZC0x", "spki": b64url(&phone.spki()), "key": b64url(&key), "ad": wrong_challenge.ad, "cd": wrong_challenge.cd, "sig": wrong_challenge.sig }).to_string();
    let (epk, n, ct) = seal::pair_seal_for_test(&other.public, &id2, plain.as_bytes()).unwrap();
    on_pair_answer(&id2, &json!({ "epk": b64url(&epk), "n": b64url(&n), "ct": b64url(&ct) }).to_string(), NOW + 5, &send);
    assert_eq!(pair_view(NOW + 6).state, "failed");
    assert!(pair_take(&send).is_none());
    pair_cancel(&send);
}

#[test]
fn a_request_is_sealed_per_approver_and_only_an_assertion_over_its_details_approves() {
    let phone = Phone::new();
    let run = paused_run();
    let approver = Approver { id: "Y3JlZC0x".into(), name: "Example phone".into(), public_key: seal::to_hex(&phone.point()), kind: "passkey".into(), synced: true, added: NOW };
    let key = [5u8; 32];
    let code = new_match_code().unwrap();
    assert!(code.len() == 7 && code.as_bytes()[3] == b'-');
    let req = request(RID, &code, &run, &[(approver.clone(), key)], "task-relay.example.workers.dev").unwrap();
    // Pushover sees the runbook, step, client and match code; never the relay's host.
    assert!(req.text.contains("glim-check") && req.text.contains("step 3 of 5") && req.text.contains("account-2") && req.text.contains(&code));
    assert!(!req.text.contains("workers.dev") && !req.url.contains("workers.dev") && !req.url.contains("task-relay"));
    assert!(req.url.starts_with(&format!("{PAGE_URL}#r={RID}&h=")));
    // The relay's frame carries only sealed details.
    let frame: Value = serde_json::from_str(&req.frame).unwrap();
    assert_eq!(frame["t"], "approval");
    let body: Value = serde_json::from_str(frame["body"].as_str().unwrap()).unwrap();
    assert!(!frame["body"].as_str().unwrap().contains("glim-check"), "details sealed");
    let item = &body["items"][0];
    let n: [u8; 12] = from_b64url(item["n"].as_str().unwrap()).unwrap().try_into().unwrap();
    let details = seal::approve_open(&key, &format!("glim-approve-v1|{RID}"), &n, &from_b64url(item["ct"].as_str().unwrap()).unwrap()).unwrap();
    let d: Value = serde_json::from_slice(&details).unwrap();
    assert_eq!((d["runbook"].as_str(), d["step"].as_u64(), d["code"].as_str()), (Some("glim-check"), Some(3), Some(code.as_str())));

    let ch = challenge(RID, &run, &code);
    let approve = |a: Assertion| json!({ "answer": "approve", "cred": a.cred, "ad": a.ad, "cd": a.cd, "sig": a.sig }).to_string();
    assert_eq!(judge(&approve(phone.assert(&ch)), RID, &run, &code, std::slice::from_ref(&approver)), Verdict::Approve);
    assert_eq!(judge(r#"{"answer":"stop"}"#, RID, &run, &code, std::slice::from_ref(&approver)), Verdict::Stop);
    // Signed for another request, another step, or a changed runbook: refused.
    let other_step = Run { step: 4, ..run.clone() };
    let other_hash = Run { hash: "cd".repeat(32), ..run.clone() };
    for (name, signed_over) in [("another request", challenge("ffffffffffffffffffffffffffffffff", &run, &code)), ("another step", challenge(RID, &other_step, &code)), ("another hash", challenge(RID, &other_hash, &code)), ("another code", challenge(RID, &run, "XXX-XXX"))] {
        assert_eq!(judge(&approve(phone.assert(&signed_over)), RID, &run, &code, std::slice::from_ref(&approver)), Verdict::Refused("bad-challenge"), "{name}");
    }
    // A key no approver has, or no approvers at all.
    assert_eq!(judge(&approve(Phone::new().assert(&ch)), RID, &run, &code, std::slice::from_ref(&approver)), Verdict::Refused("bad-signature"));
    assert_eq!(judge(&approve(phone.assert(&ch)), RID, &run, &code, &[]), Verdict::Refused("unknown-approver"));
    assert_eq!(judge("{\"answer\":\"yes\"}", RID, &run, &code, std::slice::from_ref(&approver)), Verdict::Refused("bad-answer"));
    assert!(request(RID, &code, &run, &[], "x").is_none(), "no approver, no request");
}

#[test]
fn fingerprints_are_four_groups_of_four() {
    let f = fingerprint_of(&[4u8; 65]);
    assert_eq!(f.len(), 19);
    assert_eq!(f.split(' ').count(), 4);
}

#[test]
fn a_lease_is_approved_only_over_its_own_binding_never_with_a_runbook_approval() {
    use super::super::lease::Pending;
    let phone = Phone::new();
    let approver = Approver { id: "Y3JlZC0x".into(), name: "Example phone".into(), public_key: seal::to_hex(&phone.point()), kind: "passkey".into(), synced: true, added: NOW };
    let key = [5u8; 32];
    let p = Pending { rid: RID.into(), code: "ABC-DEF".into(), client: "muse".into(), hours: 4, expires: NOW + 1800, url: String::new(), lock: "ee".repeat(16), key: "ab".repeat(32) };
    let many: Vec<String> = (0..LEASE_RUNBOOKS_SHOWN + 3).map(|i| format!("book-{i}")).collect();
    let req = lease_request(&p, &many, &[(approver.clone(), key)], "task-relay.example.workers.dev").unwrap();
    // Pushover: client, length, code; never the relay host. The relay: sealed details only.
    assert!(req.text.contains("muse") && req.text.contains("4 h") && req.text.contains("ABC-DEF"));
    assert!(!req.text.contains("workers.dev") && !req.url.contains("workers.dev"));
    let frame: Value = serde_json::from_str(&req.frame).unwrap();
    assert!(!frame["body"].as_str().unwrap().contains("muse"), "details sealed");
    let body: Value = serde_json::from_str(frame["body"].as_str().unwrap()).unwrap();
    let item = &body["items"][0];
    let n: [u8; 12] = from_b64url(item["n"].as_str().unwrap()).unwrap().try_into().unwrap();
    let d: Value = serde_json::from_slice(&seal::approve_open(&key, &format!("glim-approve-v1|{RID}"), &n, &from_b64url(item["ct"].as_str().unwrap()).unwrap()).unwrap()).unwrap();
    assert_eq!((d["kind"].as_str(), d["client"].as_str(), d["hours"].as_u64(), d["lock"].as_str()), (Some("lease"), Some("muse"), Some(4), Some(p.lock.as_str())));
    // The card lists the remote-level runbooks it could start as the owner
    // (owner 2026-11-22), capped, the rest counted; never part of the binding.
    assert_eq!(d["runbooks"].as_array().unwrap().len(), LEASE_RUNBOOKS_SHOWN);
    assert_eq!((d["runbooks"][0].as_str(), d["more"].as_u64()), (Some("book-0"), Some(3)));
    assert!(lease_binding(&p).starts_with("glim-lease-v1\n"));
    assert!(!lease_binding(&p).contains("book-0"));

    let approve = |a: Assertion| json!({ "answer": "approve", "cred": a.cred, "ad": a.ad, "cd": a.cd, "sig": a.sig }).to_string();
    assert_eq!(judge_lease(&approve(phone.assert(&lease_challenge(&p))), &p, std::slice::from_ref(&approver)), Verdict::Approve);
    assert_eq!(judge_lease(r#"{"answer":"stop"}"#, &p, std::slice::from_ref(&approver)), Verdict::Stop);
    // Signed for more hours, another client, another request or lock slot: refused.
    for (name, other) in [("hours", Pending { hours: 12, ..p.clone() }), ("client", Pending { client: "bot".into(), ..p.clone() }), ("rid", Pending { rid: "ff".repeat(16), ..p.clone() }), ("lock", Pending { lock: "dd".repeat(16), ..p.clone() }), ("code", Pending { code: "XXX-XXX".into(), ..p.clone() })] {
        assert_eq!(judge_lease(&approve(phone.assert(&lease_challenge(&other))), &p, std::slice::from_ref(&approver)), Verdict::Refused("bad-challenge"), "{name}");
    }
    // A runbook approval's signature never grants a lease, and a lease's never resumes a run.
    let run = paused_run();
    assert_eq!(judge_lease(&approve(phone.assert(&challenge(RID, &run, "ABC-DEF"))), &p, std::slice::from_ref(&approver)), Verdict::Refused("bad-challenge"));
    assert_eq!(judge(&approve(phone.assert(&lease_challenge(&p))), RID, &run, "ABC-DEF", std::slice::from_ref(&approver)), Verdict::Refused("bad-challenge"));
    // Another passkey, or none enrolled.
    assert_eq!(judge_lease(&approve(Phone::new().assert(&lease_challenge(&p))), &p, std::slice::from_ref(&approver)), Verdict::Refused("bad-signature"));
    assert_eq!(judge_lease(&approve(phone.assert(&lease_challenge(&p))), &p, &[]), Verdict::Refused("unknown-approver"));
    assert!(lease_request(&p, &[], &[], "x").is_none());
    // The lock slot: sealed, lives until the lease's end.
    let (lf, lurl) = lock_slot(&p.lock, "muse", NOW + 4 * 3600, &[(approver, key)], "task-relay.example.workers.dev").unwrap();
    let lv: Value = serde_json::from_str(&lf).unwrap();
    assert_eq!((lv["id"].as_str(), lv["exp"].as_i64()), (Some(p.lock.as_str()), Some(NOW + 4 * 3600)));
    assert!(!lf.contains("muse") && lurl.contains(&p.lock));
}
