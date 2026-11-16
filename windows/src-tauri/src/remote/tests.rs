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
    assert_eq!(RemoteSettings::default(), RemoteSettings { on: false, consented: false, killed: false, allowed_since: 0, public_key: String::new(), clients: Vec::new(), relay: String::new(), pc_public_key: String::new() });
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
    // No key saved: no fingerprint (it once showed E3B0…, SHA-256 of nothing).
    assert_eq!(fingerprint(""), "");
    assert_eq!(fingerprint("abcd"), "");
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

/// Owner 2026-11-14 (kill-switch test from the cloud): a task sent while
/// stopped waits at the relay and is handed over on reconnect after "Allow
/// again". It must be refused as killed, its id noted, and never run; one
/// issued after "Allow again" goes through.
#[test]
fn a_task_queued_while_stopped_is_refused_after_allow_again() {
    let kp = signer(7);
    let allowed_at = NOW;
    let s = RemoteSettings { allowed_since: allowed_at, ..settings_for(&kp) };
    let mut g = Gate::default();
    let queued = envelope(&kp, &json!({ "id": id(40), "iat": allowed_at - 30, "exp": allowed_at + 270, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &queued, allowed_at + 2), Err(Rejection::Killed));
    let last = g.activity.last().unwrap();
    assert_eq!((last.ok, last.reason.as_str(), last.id.as_str()), (false, "killed", id(40).as_str()));
    // Issued one second before Allow again: still queued while stopped.
    let edge = envelope(&kp, &json!({ "id": id(41), "iat": allowed_at - 1, "exp": allowed_at + 299, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &edge, allowed_at + 2), Err(Rejection::Killed));
    // Issued at or after Allow again: accepted.
    let fresh = envelope(&kp, &json!({ "id": id(42), "iat": allowed_at, "exp": allowed_at + 300, "cmd": "session-status", "args": {} }));
    assert!(g.accept(&s, &fresh, allowed_at + 2).is_ok());
    // Never switched off and on: allowed_since 0 changes nothing.
    let mut g = Gate::default();
    assert!(g.accept(&settings_for(&kp), &queued, allowed_at + 2).is_ok());
}

/// A bad signature is still refused as such (the queued-task check comes
/// only after the signature verified, so its id is never kept).
#[test]
fn a_forged_queued_task_is_a_bad_signature_not_killed() {
    let kp = signer(7);
    let s = RemoteSettings { allowed_since: NOW, ..settings_for(&kp) };
    let mut g = Gate::default();
    let forged = envelope(&signer(9), &json!({ "id": id(43), "iat": NOW - 30, "exp": NOW + 270, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &forged, NOW + 2), Err(Rejection::BadSignature));
    assert_eq!(g.activity.last().unwrap().id, "");
}

/// keepalive-restart (owner 2026-11-15): a folder's name or none, nothing
/// else; three an hour; the kill switch refuses it like everything else.
#[test]
fn keepalive_restart_parses_is_rate_limited_and_obeys_the_kill_switch() {
    let parse = |args: Value| command::parse("keepalive-restart", args.as_object().unwrap());
    assert_eq!(parse(json!({})), Ok(Command::KeepaliveRestart { folder: None }));
    assert_eq!(parse(json!({ "folder": " local-companion " })), Ok(Command::KeepaliveRestart { folder: Some("local-companion".into()) }));
    for bad in [json!({ "folder": r"C:\Users\x\repo" }), json!({ "folder": "" }), json!({ "folder": 3 }), json!({ "repo": "local-companion" }), json!({ "folder": "a/b" })] {
        assert_eq!(parse(bad.clone()), Err(Rejection::BadArgs), "{bad}");
    }
    assert_eq!(Command::KeepaliveRestart { folder: None }.kind(), "keepalive-restart");
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    for n in 0..3 {
        assert!(g.accept(&s, &envelope(&kp, &task(&id(500 + n), "keepalive-restart", json!({}))), NOW + n as i64).is_ok());
    }
    assert_eq!(g.accept(&s, &envelope(&kp, &task(&id(510), "keepalive-restart", json!({}))), NOW + 5), Err(Rejection::RateLimited), "the 4th in an hour");
    assert!(g.accept(&s, &envelope(&kp, &task(&id(511), "session-status", json!({}))), NOW + 6).is_ok(), "other commands still go");
    let killed = RemoteSettings { killed: true, ..settings_for(&kp) };
    let mut g = Gate::default();
    assert_eq!(g.accept(&killed, &envelope(&kp, &task(&id(520), "keepalive-restart", json!({}))), NOW), Err(Rejection::Killed));
}

/// remote-state.json from before keepalive-restart (no "restarts"): its seen
/// ids still load, so nothing becomes replayable after an update.
#[test]
fn an_older_gate_file_still_keeps_its_seen_ids() {
    let dir = std::env::temp_dir().join(format!("glim-gate-old-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("remote-state.json");
    std::fs::write(&path, format!(r#"{{"seen":{{"{}":{}}},"recent":[],"activity":[]}}"#, id(600), NOW + 300)).unwrap();
    let kp = signer(7);
    let mut g = Gate::load(path.clone());
    assert_eq!(g.accept(&settings_for(&kp), &envelope(&kp, &task(&id(600), "session-status", json!({}))), NOW), Err(Rejection::Replay));
    let _ = std::fs::remove_dir_all(&dir);
}

fn client(name: &str, kp: &Ed25519KeyPair) -> CloudClient {
    CloudClient { name: name.into(), public_key: hex(kp.public_key().as_ref()), token_sha256: "ab".repeat(32), added: NOW }
}

#[test]
fn the_old_single_key_is_account_1_and_keeps_working_without_re_entering_anything() {
    let kp = signer(7);
    let s = settings_for(&kp);
    assert_eq!(s.clients(), vec![CloudClient { name: "account-1".into(), public_key: hex(kp.public_key().as_ref()), token_sha256: "legacy".into(), added: 0 }]);
    assert_eq!(s.relay_list(), "account-1=legacy");
    let mut g = Gate::default();
    let a = g.accept_from(&s, "account-1", &envelope(&kp, &task(&id(400), "session-status", json!({}))), NOW).unwrap();
    assert_eq!(a.client, "account-1");
    assert_eq!(g.activity[0].client, "account-1");
    // Adding account-2 keeps account-1 as it was.
    let mut s2 = s.clone();
    s2.add_client(client("account-2", &signer(8))).unwrap();
    assert_eq!(s2.clients().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["account-1", "account-2"]);
    assert_eq!(s2.relay_list(), format!("account-1=legacy,account-2={}", "ab".repeat(32)));
    assert!(g.accept_from(&s2, "account-1", &envelope(&kp, &task(&id(401), "session-status", json!({}))), NOW).is_ok());
}

#[test]
fn each_task_must_be_signed_by_the_key_of_the_client_whose_token_queued_it() {
    let (k1, k2) = (signer(7), signer(8));
    let mut s = RemoteSettings { on: true, consented: true, ..Default::default() };
    s.add_client(client("account-1", &k1)).unwrap();
    s.add_client(client("account-2", &k2)).unwrap();
    let mut g = Gate::default();
    assert_eq!(g.accept_from(&s, "account-2", &envelope(&k2, &task(&id(410), "session-status", json!({}))), NOW).unwrap().client, "account-2");
    // account-1's key, queued with account-2's token (and the other way).
    assert_eq!(g.accept_from(&s, "account-2", &envelope(&k1, &task(&id(411), "session-status", json!({}))), NOW), Err(Rejection::WrongClient));
    assert_eq!(g.accept_from(&s, "account-1", &envelope(&k2, &task(&id(412), "session-status", json!({}))), NOW), Err(Rejection::WrongClient));
    // A stranger's key: a plain bad signature. A name nobody added: unknown.
    assert_eq!(g.accept_from(&s, "account-1", &envelope(&signer(9), &task(&id(413), "session-status", json!({}))), NOW), Err(Rejection::BadSignature));
    assert_eq!(g.accept_from(&s, "account-3", &envelope(&k1, &task(&id(414), "session-status", json!({}))), NOW), Err(Rejection::UnknownClient));
    assert_eq!(g.accept_from(&s, "Bad Name!", &envelope(&k1, &task(&id(415), "session-status", json!({}))), NOW), Err(Rejection::UnknownClient));
    // A refused id was never kept: the right client can still send it.
    assert!(g.accept_from(&s, "account-1", &envelope(&k1, &task(&id(411), "session-status", json!({}))), NOW).is_ok());
    // Names in the activity list are checked ones only.
    assert!(g.activity.iter().all(|a| a.client.is_empty() || a.client.starts_with("account-")));
    // Removing one leaves the other working.
    assert!(s.remove_client("account-1"));
    assert_eq!(g.accept_from(&s, "account-1", &envelope(&k1, &task(&id(416), "session-status", json!({}))), NOW), Err(Rejection::UnknownClient));
    assert!(g.accept_from(&s, "account-2", &envelope(&k2, &task(&id(417), "session-status", json!({}))), NOW).is_ok());
    assert!(!s.remove_client("account-1"));
    s.remove_client("account-2");
    assert_eq!(s.relay_list(), "none");
}

#[test]
fn clients_are_refused_with_a_taken_name_or_key_a_bad_name_or_past_ten() {
    let mut s = RemoteSettings::default();
    s.add_client(client("account-1", &signer(1))).unwrap();
    assert_eq!(s.add_client(client("account-1", &signer(2))), Err("name-taken"));
    assert_eq!(s.add_client(client("account-2", &signer(1))), Err("key-taken"));
    for i in 2..=MAX_CLIENTS {
        s.add_client(client(&format!("account-{i}"), &signer(i as u8))).unwrap();
    }
    assert_eq!(s.add_client(client("account-99", &signer(99))), Err("too-many"));
    assert_eq!(parse_client_name("  Account-2 "), Ok("account-2".into()));
    let long = "x".repeat(33);
    for bad in ["", "-x", "has space", "a_b", long.as_str()] {
        assert!(parse_client_name(bad).is_err(), "{bad}");
    }
}

#[test]
fn rate_limits_are_per_client_with_a_cap_for_the_whole_pc() {
    let (k1, k2, k3) = (signer(7), signer(8), signer(9));
    let mut s = RemoteSettings { on: true, consented: true, ..Default::default() };
    for (n, k) in [("account-1", &k1), ("account-2", &k2), ("account-3", &k3)] {
        s.add_client(client(n, k)).unwrap();
    }
    let mut g = Gate::default();
    let mut n = 500;
    let mut send = |g: &mut Gate, who: &str, k: &Ed25519KeyPair, cmd: &str, args: Value| {
        n += 1;
        g.accept_from(&s, who, &envelope(k, &task(&id(n), cmd, args)), NOW)
    };
    let start = || json!({ "repo": "local-companion" });
    // account-1: 5 start-session, then its 6th is refused...
    for _ in 0..MAX_START_PER_HOUR {
        send(&mut g, "account-1", &k1, "start-session", start()).unwrap();
    }
    assert_eq!(send(&mut g, "account-1", &k1, "start-session", start()), Err(Rejection::RateLimited));
    // ...while account-2 still may: up to the PC's 8 in all.
    for _ in 0..(MAX_START_PER_HOUR_PC - MAX_START_PER_HOUR) {
        send(&mut g, "account-2", &k2, "start-session", start()).unwrap();
    }
    assert_eq!(send(&mut g, "account-2", &k2, "start-session", start()), Err(Rejection::RateLimited));
    // Commands: 30 per client (account-1 has 5 already).
    for _ in 0..(MAX_COMMANDS_PER_HOUR - MAX_START_PER_HOUR) {
        send(&mut g, "account-1", &k1, "session-status", json!({})).unwrap();
    }
    assert_eq!(send(&mut g, "account-1", &k1, "session-status", json!({})), Err(Rejection::RateLimited));
    // 30 + 3 so far: account-2 gets the PC's remaining 17 of 50 (under its own 30)...
    let mut more = 0;
    while send(&mut g, "account-2", &k2, "session-status", json!({})).is_ok() {
        more += 1;
    }
    assert_eq!(more, MAX_COMMANDS_PER_HOUR_PC - MAX_COMMANDS_PER_HOUR - (MAX_START_PER_HOUR_PC - MAX_START_PER_HOUR));
    // ...and account-3, with none of its own yet, is held by the PC-wide cap.
    assert_eq!(send(&mut g, "account-3", &k3, "session-status", json!({})), Err(Rejection::RateLimited));
    // An hour later, all clear.
    assert!(g.accept_from(&s, "account-3", &envelope(&k3, &json!({ "id": id(999), "iat": NOW + 3595, "exp": NOW + 3700, "cmd": "session-status", "args": {} })), NOW + 3601).is_ok());
}

#[test]
fn an_old_state_file_keeps_its_counts_as_account_1() {
    let dir = std::env::temp_dir().join(format!("glim-remote-clients-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("remote-state.json");
    let recent: Vec<(i64, bool)> = (0..MAX_COMMANDS_PER_HOUR as i64).map(|i| (NOW - i, false)).collect();
    std::fs::write(&path, serde_json::to_vec(&json!({ "seen": { "old-task-id-000001": NOW + 100 }, "recent": recent, "activity": [] })).unwrap()).unwrap();
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::load(path.clone());
    assert_eq!(g.accept_from(&s, "account-1", &envelope(&kp, &task("old-task-id-000001", "session-status", json!({}))), NOW), Err(Rejection::Replay));
    assert_eq!(g.accept_from(&s, "account-1", &envelope(&kp, &task(&id(600), "session-status", json!({}))), NOW), Err(Rejection::RateLimited));
    // Saved in the new form, it loads the same.
    let mut again = Gate::load(path.clone());
    assert_eq!(again.accept_from(&s, "account-1", &envelope(&kp, &task(&id(601), "session-status", json!({}))), NOW), Err(Rejection::RateLimited));
    let _ = std::fs::remove_dir_all(&dir);
}
