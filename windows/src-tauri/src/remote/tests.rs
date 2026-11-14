use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};

use super::*;

const NOW: i64 = 1_763_140_000;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A made-up signer (a fixed test seed; never a real key).
fn signer(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn settings_for(kp: &Ed25519KeyPair) -> RemoteSettings {
    RemoteSettings { on: true, consented: true, killed: false, public_key: hex(kp.public_key().as_ref()), ..Default::default() }
}

fn envelope(kp: &Ed25519KeyPair, payload: &Value) -> Vec<u8> {
    let p = payload.to_string();
    let sig = kp.sign(p.as_bytes());
    serde_json::to_vec(&json!({ "v": 1, "payload": p, "sig": hex(sig.as_ref()) })).unwrap()
}

fn task(id: &str, cmd: &str, args: Value) -> Value {
    json!({ "id": id, "iat": NOW - 5, "exp": NOW + 295, "cmd": cmd, "args": args })
}

fn id(n: usize) -> String {
    format!("task-{n:016}")
}

#[test]
fn a_signed_fresh_task_is_accepted_once_and_noted_without_its_payload() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    let raw = envelope(&kp, &task(&id(1), "send-to-session", json!({ "repo": "local-companion", "text": "SECRET-ish words to send" })));
    let a = g.accept(&s, &raw, NOW).unwrap();
    assert_eq!(a.command, Command::SendToSession { repo: "local-companion".into(), text: "SECRET-ish words to send".into() });
    assert_eq!(g.accept(&s, &raw, NOW + 1), Err(Rejection::Replay));
    let listed = format!("{:?}", g.activity);
    assert!(!listed.contains("SECRET") && !listed.contains("words"), "{listed}");
    assert_eq!((g.activity[0].ok, g.activity[0].kind.as_str()), (true, "send-to-session"));
    assert_eq!((g.activity[1].ok, g.activity[1].reason.as_str(), g.activity[1].id.as_str()), (false, "replay", id(1).as_str()));
}

#[test]
fn bad_signatures_other_keys_and_tampering_are_refused_and_their_id_is_never_kept() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    // Signed by another key.
    let other = envelope(&signer(9), &task(&id(2), "session-status", json!({})));
    assert_eq!(g.accept(&s, &other, NOW), Err(Rejection::BadSignature));
    // The payload changed after signing.
    let good = envelope(&kp, &task(&id(3), "session-status", json!({})));
    let mut v: Value = serde_json::from_slice(&good).unwrap();
    v["payload"] = Value::String(v["payload"].as_str().unwrap().replace("session-status", "start-session"));
    assert_eq!(g.accept(&s, &serde_json::to_vec(&v).unwrap(), NOW), Err(Rejection::BadSignature));
    // Not hex, wrong length, not JSON, wrong version, too big.
    v["sig"] = Value::String("zz".into());
    assert_eq!(g.accept(&s, &serde_json::to_vec(&v).unwrap(), NOW), Err(Rejection::BadSignature));
    assert_eq!(g.accept(&s, b"not json", NOW), Err(Rejection::BadEnvelope));
    let mut v2: Value = serde_json::from_slice(&good).unwrap();
    v2["v"] = json!(2);
    assert_eq!(g.accept(&s, &serde_json::to_vec(&v2).unwrap(), NOW), Err(Rejection::BadEnvelope));
    assert_eq!(g.accept(&s, &vec![b' '; MAX_ENVELOPE_BYTES + 1], NOW), Err(Rejection::TooBig));
    assert!(g.activity.iter().all(|a| !a.ok && a.id.is_empty()), "an unverified id is never noted: {:?}", g.activity);
    // The real one still goes.
    assert!(g.accept(&s, &good, NOW).is_ok());
}

#[test]
fn expiry_lifetime_and_clock_skew() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    let at = |iat: i64, exp: i64, n: usize| envelope(&kp, &json!({ "id": id(n), "iat": iat, "exp": exp, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &at(NOW - 700, NOW - 100, 10), NOW), Err(Rejection::Expired));
    assert_eq!(g.accept(&s, &at(NOW, NOW + 601, 11), NOW), Err(Rejection::TooLong), "at most 10 minutes");
    assert_eq!(g.accept(&s, &at(NOW + 120, NOW + 300, 12), NOW), Err(Rejection::NotYetValid));
    assert_eq!(g.accept(&s, &at(NOW, NOW - 1, 13), NOW), Err(Rejection::TooLong), "expires before it was issued");
    assert!(g.accept(&s, &at(NOW + 30, NOW + 600, 14), NOW).is_ok(), "30 s ahead is skew");
    assert!(g.accept(&s, &at(NOW - 600, NOW - 30, 15), NOW).is_ok(), "30 s past expiry is skew");
    assert_eq!(g.activity.iter().find(|a| a.reason == "expired").unwrap().id, id(10), "a verified id is noted");
}

#[test]
fn replay_protection_outlives_a_restart_until_expiry() {
    let dir = std::env::temp_dir().join(format!("glim-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("remote-state.json");
    let kp = signer(7);
    let s = settings_for(&kp);
    let raw = envelope(&kp, &task(&id(20), "session-status", json!({})));
    assert!(Gate::load(path.clone()).accept(&s, &raw, NOW).is_ok());
    let mut again = Gate::load(path.clone());
    assert_eq!(again.accept(&s, &raw, NOW + 10), Err(Rejection::Replay), "kept on disk");
    assert_eq!(again.activity.len(), 2, "the list survives too");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("payload") && !saved.contains("sig"), "{saved}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn off_by_default_needs_consent_a_key_and_the_kill_switch_wins() {
    let kp = signer(7);
    let raw = envelope(&kp, &task(&id(30), "session-status", json!({})));
    let mut g = Gate::default();
    assert_eq!(RemoteSettings::default(), RemoteSettings { on: false, consented: false, killed: false, public_key: String::new(), relay: String::new(), pc_public_key: String::new() });
    assert_eq!(g.accept(&RemoteSettings::default(), &raw, NOW), Err(Rejection::Off));
    let not_consented = RemoteSettings { consented: false, ..settings_for(&kp) };
    assert_eq!(g.accept(&not_consented, &raw, NOW), Err(Rejection::Off));
    let no_key = RemoteSettings { public_key: String::new(), ..settings_for(&kp) };
    assert_eq!(g.accept(&no_key, &raw, NOW), Err(Rejection::NoKey));
    let killed = RemoteSettings { killed: true, ..settings_for(&kp) };
    assert_eq!(g.accept(&killed, &raw, NOW), Err(Rejection::Killed));
    assert!(g.accept(&settings_for(&kp), &raw, NOW).is_ok(), "never counted as seen while refused");
    g.note_kill(NOW, true);
    assert_eq!(g.activity.last().unwrap().kind, "kill-switch");
}

#[test]
fn rate_limits_five_starts_and_thirty_commands_an_hour() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    for n in 0..5 {
        assert!(g.accept(&s, &envelope(&kp, &task(&id(100 + n), "start-session", json!({ "repo": "local-companion" }))), NOW + n as i64).is_ok());
    }
    let sixth = envelope(&kp, &task(&id(105), "start-session", json!({ "repo": "local-companion" })));
    assert_eq!(g.accept(&s, &sixth, NOW + 10), Err(Rejection::RateLimited));
    for n in 0..25 {
        assert!(g.accept(&s, &envelope(&kp, &task(&id(200 + n), "session-status", json!({}))), NOW + 20).is_ok(), "{n}");
    }
    assert_eq!(g.accept(&s, &envelope(&kp, &task(&id(300), "session-status", json!({}))), NOW + 30), Err(Rejection::RateLimited), "31st");
    // An hour later the window has moved (fresh tasks, fresh times).
    let later = NOW + 3700;
    let t = json!({ "id": id(301), "iat": later, "exp": later + 60, "cmd": "start-session", "args": { "repo": "local-companion" } });
    assert!(g.accept(&s, &envelope(&kp, &t), later).is_ok());
}

#[test]
fn the_command_parser_checks_every_argument() {
    let m = |v: Value| v.as_object().unwrap().clone();
    use command::parse;
    assert_eq!(parse("session-status", &m(json!({}))), Ok(Command::SessionStatus { repo: None }));
    assert_eq!(parse("session-status", &m(json!({ "repo": "Local-Companion" }))), Ok(Command::SessionStatus { repo: Some("local-companion".into()) }));
    assert_eq!(parse("transcript-tail", &m(json!({ "repo": "local-companion" }))), Ok(Command::TranscriptTail { repo: "local-companion".into(), lines: 20 }));
    assert_eq!(parse("transcript-tail", &m(json!({ "repo": "local-companion", "lines": 50 }))).unwrap().kind(), "transcript-tail");
    assert_eq!(parse("transcript-tail", &m(json!({ "repo": "local-companion", "lines": 51 }))), Err(Rejection::BadArgs));
    assert_eq!(parse("transcript-tail", &m(json!({ "repo": "local-companion", "lines": "5" }))), Err(Rejection::BadArgs));
    assert_eq!(parse("start-session", &m(json!({ "repo": "local-companion", "prompt": "Read HANDOFF.md and continue." }))).unwrap().kind(), "start-session");
    assert_eq!(parse("start-session", &m(json!({ "repo": "harbor-tiles" }))), Err(Rejection::RepoNotAllowed), "allowlist: local-companion only");
    assert_eq!(parse("start-session", &m(json!({ "repo": "..\\local-companion" }))), Err(Rejection::RepoNotAllowed));
    assert_eq!(parse("start-session", &m(json!({}))), Err(Rejection::BadArgs));
    assert_eq!(parse("start-session", &m(json!({ "repo": "local-companion", "prompt": "x".repeat(2001) }))), Err(Rejection::BadArgs));
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion" }))), Err(Rejection::BadArgs), "text required");
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "text": "   " }))), Err(Rejection::BadArgs));
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "text": "a\u{1b}[2J" }))), Err(Rejection::BadArgs), "no control characters");
    assert!(parse("send-to-session", &m(json!({ "repo": "local-companion", "text": "line one\nline two" }))).is_ok());
    assert_eq!(parse("runbook", &m(json!({ "name": "glim-update" }))), Ok(Command::Runbook { name: "glim-update".into() }));
    for bad in [json!({ "name": "Glim Update" }), json!({ "name": "" }), json!({ "name": "a&b" }), json!({ "name": "glim-update", "extra": 1 })] {
        assert_eq!(parse("runbook", &m(bad.clone())), Err(Rejection::BadArgs), "{bad}");
    }
    assert_eq!(parse("session-status", &m(json!({ "cmd": "rm" }))), Err(Rejection::BadArgs), "no unknown keys");
    assert_eq!(parse("shell", &m(json!({ "line": "whoami" }))), Err(Rejection::UnknownCommand));
}

#[test]
fn keys_and_fingerprints() {
    let kp = signer(7);
    let pk = hex(kp.public_key().as_ref());
    assert_eq!(parse_public_key(&pk.to_uppercase()), Ok(pk.clone()));
    let spaced: String = pk.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ");
    assert_eq!(parse_public_key(&spaced), Ok(pk.clone()));
    assert!(parse_public_key("abc").is_err() && parse_public_key(&"g".repeat(64)).is_err());
    let fp = fingerprint(&pk);
    assert_eq!(fp.len(), 8 * 4 + 7);
    assert_eq!(fp.split(' ').count(), 8);
    assert_ne!(fp, fingerprint(&hex(signer(9).public_key().as_ref())));
    assert_eq!(fingerprint("nothex"), "");
}

/// Only remote/ (courier.rs) calls the gate: no other path into it.
#[test]
fn only_the_courier_calls_the_gate() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut callers = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path != src.join("remote") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") && std::fs::read_to_string(&path).unwrap().contains(concat!(".acc", "ept(&")) {
                callers.push(path.display().to_string());
            }
        }
    }
    // tokio/std listeners' accept() live in net/ (excluded by name below).
    callers.retain(|p| !p.contains("net"));
    assert!(callers.is_empty(), "{callers:?}");
}
