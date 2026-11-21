use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};

use super::*;

const NOW: i64 = 1_763_140_000;
/// A made-up Claude Code session id.
const SESSION: &str = "0b6c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3";

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
    let raw = envelope(&kp, &task(&id(1), "send-to-session", json!({ "repo": "local-companion", "session": SESSION, "text": "SECRET-ish words to send" })));
    let a = g.accept(&s, &raw, NOW).unwrap();
    assert_eq!(a.command, Command::SendToSession { repo: "local-companion".into(), session: SESSION.into(), text: "SECRET-ish words to send".into() });
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
    assert_eq!(g.accept(&s, &at(NOW, NOW + 72 * 3600 + 1, 11), NOW), Err(Rejection::TooLong), "at most 72 h");
    assert!(g.accept(&s, &at(NOW, NOW + 72 * 3600, 16), NOW).is_ok(), "72 h is allowed (owner 2026-11-17)");
    assert_eq!(g.accept(&s, &at(NOW + 120, NOW + 300, 12), NOW), Err(Rejection::NotYetValid));
    assert_eq!(g.accept(&s, &at(NOW, NOW - 1, 13), NOW), Err(Rejection::TooLong), "expires before it was issued");
    assert!(g.accept(&s, &at(NOW + 30, NOW + 600, 14), NOW).is_ok(), "30 s ahead is skew");
    // No skew after the expiry (owner 2026-11-17): expired at the relay = never runs.
    assert_eq!(g.accept(&s, &at(NOW - 600, NOW - 1, 15), NOW), Err(Rejection::Expired), "1 s past its expiry");
    assert!(g.accept(&s, &at(NOW - 600, NOW, 17), NOW).is_ok(), "its expiry second itself");
    assert_eq!(g.activity.iter().find(|a| a.reason == "expired").unwrap().id, id(10), "a verified id is noted");
}

#[test]
fn start_session_and_send_to_session_live_at_most_24_hours() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    let at = |exp: i64, n: usize, cmd: &str, args: Value| envelope(&kp, &json!({ "id": id(n), "iat": NOW, "exp": exp, "cmd": cmd, "args": args }));
    let send = json!({ "repo": "local-companion", "session": SESSION, "text": "hi" });
    let start = json!({ "repo": "local-companion" });
    assert_eq!(g.accept(&s, &at(NOW + 24 * 3600 + 1, 1, "send-to-session", send.clone()), NOW), Err(Rejection::TooLong));
    assert_eq!(g.accept(&s, &at(NOW + 24 * 3600 + 1, 2, "start-session", start.clone()), NOW), Err(Rejection::TooLong));
    assert!(g.accept(&s, &at(NOW + 24 * 3600, 3, "send-to-session", send), NOW).is_ok());
    assert!(g.accept(&s, &at(NOW + 24 * 3600, 4, "start-session", start), NOW).is_ok());
    assert!(g.accept(&s, &at(NOW + 72 * 3600, 5, "session-status", json!({})), NOW).is_ok(), "read-only commands may wait 72 h");
}

#[test]
fn a_long_lived_task_still_obeys_replay_and_the_kill_switch() {
    let kp = signer(7);
    let mut g = Gate::default();
    let t = envelope(&kp, &json!({ "id": id(1), "iat": NOW, "exp": NOW + 72 * 3600, "cmd": "session-status", "args": {} }));
    assert!(g.accept(&settings_for(&kp), &t, NOW).is_ok());
    // Seen ids are kept until their own expiry, so 71 h later it is still a replay.
    assert_eq!(g.accept(&settings_for(&kp), &t, NOW + 71 * 3600), Err(Rejection::Replay));
    // Queued for a day while the kill switch was on, delivered after "Allow again": killed.
    let queued = envelope(&kp, &json!({ "id": id(2), "iat": NOW, "exp": NOW + 72 * 3600, "cmd": "session-status", "args": {} }));
    let reopened = RemoteSettings { allowed_since: NOW + 24 * 3600, ..settings_for(&kp) };
    assert_eq!(g.accept(&reopened, &queued, NOW + 25 * 3600), Err(Rejection::Killed));
}

#[test]
fn a_verified_task_refused_for_now_can_never_run_later() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    // Issued 2 min ahead of this PC's clock: not yet valid, and burned.
    let early = envelope(&kp, &json!({ "id": id(1), "iat": NOW + 120, "exp": NOW + 3600, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &early, NOW), Err(Rejection::NotYetValid));
    assert_eq!(g.accept(&s, &early, NOW + 200), Err(Rejection::Replay), "the relay's copy, later");
    // Rate-limited, then redelivered after the hour: still not run.
    for n in 0..MAX_COMMANDS_PER_HOUR {
        assert!(g.accept(&s, &envelope(&kp, &task(&id(100 + n), "session-status", json!({}))), NOW).is_ok());
    }
    let late = envelope(&kp, &json!({ "id": id(2), "iat": NOW, "exp": NOW + 72 * 3600, "cmd": "session-status", "args": {} }));
    assert_eq!(g.accept(&s, &late, NOW), Err(Rejection::RateLimited));
    assert_eq!(g.accept(&s, &late, NOW + 3700), Err(Rejection::Replay));
    // A task whose signature didn't verify burns nothing (its id isn't trusted).
    let forged = envelope(&signer(8), &task(&id(3), "session-status", json!({})));
    assert_eq!(g.accept(&s, &forged, NOW), Err(Rejection::BadSignature));
    let genuine = json!({ "id": id(3), "iat": NOW + 3690, "exp": NOW + 4000, "cmd": "session-status", "args": {} });
    assert!(g.accept(&s, &envelope(&kp, &genuine), NOW + 3700).is_ok());
}

#[test]
fn a_result_key_in_the_payload_is_kept_and_a_bad_one_refused() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    let rk = "ab".repeat(32);
    let ok = envelope(&kp, &json!({ "id": id(1), "iat": NOW, "exp": NOW + 300, "cmd": "session-status", "args": {}, "rk": rk }));
    assert_eq!(g.accept(&s, &ok, NOW).unwrap().rk, Some([0xab; 32]));
    let none = envelope(&kp, &task(&id(2), "session-status", json!({})));
    assert_eq!(g.accept(&s, &none, NOW).unwrap().rk, None, "an older courier: answered unsealed");
    for (n, bad) in [(3, json!("ab")), (4, json!("AB".repeat(32))), (5, json!(7))] {
        let raw = envelope(&kp, &json!({ "id": id(n), "iat": NOW, "exp": NOW + 300, "cmd": "session-status", "args": {}, "rk": bad }));
        assert!(matches!(g.accept(&s, &raw, NOW), Err(Rejection::BadPayload)), "{bad}");
    }
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
    assert_eq!(RemoteSettings::default(), RemoteSettings { on: false, consented: false, killed: false, allowed_since: 0, public_key: String::new(), clients: Vec::new(), relay: String::new(), pc_public_key: String::new(), pc_enc_key: String::new(), require_sealed: false, runbooks_per_hour: 0, runbooks_per_hour_pc: 0 });
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
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "session": SESSION }))), Err(Rejection::BadArgs), "text required");
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "session": SESSION, "text": "   " }))), Err(Rejection::BadArgs));
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "session": SESSION, "text": "a\u{1b}[2J" }))), Err(Rejection::BadArgs), "no control characters");
    assert!(parse("send-to-session", &m(json!({ "repo": "local-companion", "session": SESSION, "text": "line one\nline two" }))).is_ok());
    // The target session must be named (owner 2026-11-17), as a UUID only.
    assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "text": "hi" }))), Err(Rejection::BadArgs), "session required");
    for bad in ["0b6c1d2e", "..\\..\\x", "0B6C1D2E-3F40-4A5B-8C6D-7E8F90A1B2C3", "0b6c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3.jsonl", "0b6c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c", "0b6c1d2e/3f40-4a5b-8c6d-7e8f90a1b2c3"] {
        assert_eq!(parse("send-to-session", &m(json!({ "repo": "local-companion", "session": bad, "text": "hi" }))), Err(Rejection::BadArgs), "{bad}");
    }
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

/// keepalive-restart's result names the account whose servers it restarted.
#[test]
fn keepalive_restart_result_names_the_account() {
    assert_eq!(exec::restart_line(1, "Claude2"), "keep-alive: restarted 1 session(s) [Claude2]");
    assert_eq!(exec::restart_line(0, "Claude"), "keep-alive: restarted 0 session(s) [Claude]");
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

// ── Accounts (owner 2026-11-15): each client reaches only its account's sessions ──

fn transcript(config: &std::path::Path, slug: &str, name: &str) {
    let d = config.join("projects").join(slug);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(format!("{name}.jsonl")), "{}\n").unwrap();
}

#[test]
fn each_client_sees_only_its_own_accounts_sessions_and_the_others_are_not_yours() {
    let home = std::env::temp_dir().join(format!("glim-remote-accounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (one, two) = (home.join(".claude"), home.join(".claude2"));
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let repo = home.join("example-repo");
    let slug = exec::project_slug(&repo);
    // Claude has a session in the repo; Claude2 one in its own worktree.
    transcript(&one, &slug, "aaaaaaaa-one");
    transcript(&two, &format!("{slug}--claude-worktrees-quiet-fox"), "bbbbbbbb-two");
    let accounts = crate::accounts::list(&[], &home);
    let known = vec![one.clone(), two.clone()];
    let s1 = exec::scope(&accounts, "account-1", &known, &home).unwrap();
    let s2 = exec::scope(&accounts, "account-2", &known, &home).unwrap();
    assert_eq!(s1.account.name, "Claude");
    assert_eq!(s2.account.name, "Claude2");
    let stem = |s: &exec::Scope| s.latest(&repo).map(|(p, _)| p.file_stem().unwrap().to_string_lossy().into_owned());
    assert_eq!(stem(&s1).as_deref(), Some("aaaaaaaa-one"));
    assert_eq!(stem(&s2).as_deref(), Some("bbbbbbbb-two"), "a worktree's session is the account's own");
    // A repo where only the other account has sessions: not yours.
    let other = home.join("other-repo");
    transcript(&one, &exec::project_slug(&other), "cccccccc-one");
    assert!(s2.latest(&other).is_none());
    assert_eq!(s2.none_reason(&other), "not-your-session");
    assert_eq!(s1.none_reason(&home.join("nobody-repo")), "no-transcript");
    // A client no account acts for gets nothing.
    assert!(exec::scope(&accounts, "account-9", &known, &home).is_none());
    // A neighbouring repo whose name starts the same is not a worktree.
    transcript(&two, &format!("{slug}-copy"), "dddddddd-two");
    assert_eq!(stem(&s2).as_deref(), Some("bbbbbbbb-two"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn send_to_session_reaches_only_the_named_session_of_its_own_account() {
    let home = std::env::temp_dir().join(format!("glim-remote-named-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (one, two) = (home.join(".claude"), home.join(".claude2"));
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let repo = home.join("example-repo");
    let slug = exec::project_slug(&repo);
    let (mine, theirs) = ("1a2b3c4d-0000-4000-8000-00000000000a", "5e6f7a8b-0000-4000-8000-00000000000b");
    transcript(&two, &format!("{slug}--claude-worktrees-quiet-fox"), mine);
    transcript(&one, &slug, theirs);
    let accounts = crate::accounts::list(&[], &home);
    let known = vec![one.clone(), two.clone()];
    let s2 = exec::scope(&accounts, "account-2", &known, &home).unwrap();
    assert!(s2.session(&repo, mine).unwrap().ends_with(format!("{mine}.jsonl")), "its own, in a worktree");
    assert_eq!(s2.session(&repo, theirs), Err("not-your-session".to_string()), "another account's");
    assert_eq!(s2.session(&repo, "99999999-0000-4000-8000-000000000000"), Err("no-such-session".to_string()), "gone or never there");
    assert_eq!(s2.session(&home.join("other-repo"), mine), Err("no-such-session".to_string()), "another repo's folder");
    assert_eq!(exec::session_transcript(&known, &repo, "..\\.claude2"), None, "only a UUID names a session");
    // session-status shows the full id, which send-to-session names.
    let line = exec::status_line("example-repo", s2.latest(&repo), std::time::SystemTime::now());
    assert!(line.contains(&format!("(session {mine})")), "{line}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_second_accounts_remote_control_is_named_after_it_and_the_default_keeps_its_name() {
    let home = std::path::PathBuf::from(r"C:\Users\someone");
    let accounts = crate::accounts::list(&[], &home);
    assert_eq!(crate::keepalive::command_line_for(r"C:\x\example-repo", &accounts[0]), crate::keepalive::command_line(r"C:\x\example-repo"));
    assert_eq!(crate::keepalive::command_line_for(r"C:\x\example-repo", &accounts[1]), r#"claude remote-control --spawn worktree --capacity 1 --name "example-repo (PC, Claude2)""#);
}

#[test]
fn send_to_session_continues_only_in_the_repo_or_its_worktrees() {
    let root = std::env::temp_dir().join(format!("glim-remote-cwd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("example-repo");
    let tree = repo.join(".claude").join("worktrees").join("quiet-fox");
    let elsewhere = root.join("elsewhere");
    for d in [&tree, &elsewhere] {
        std::fs::create_dir_all(d).unwrap();
    }
    let line = |p: &std::path::Path| format!("{}\n", json!({ "type": "user", "cwd": p.to_string_lossy() }));
    let bytes = format!("{}{}", line(&repo), line(&tree));
    assert_eq!(exec::session_cwd(bytes.as_bytes()), Some(tree.clone()), "the last one");
    assert!(exec::within_repo(&tree, &repo).is_some());
    assert!(exec::within_repo(&repo, &repo).is_some());
    assert!(exec::within_repo(&elsewhere, &repo).is_none());
    assert!(exec::within_repo(&repo.join(".claude"), &repo).is_none());
    assert_eq!(exec::session_cwd(b"not json\n{}"), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn keep_alive_runs_one_server_per_account_and_restarts_only_the_asked_accounts() {
    use crate::keepalive::{wanted_for, KeepAliveSettings, Spec};
    let home = std::path::PathBuf::from(r"C:\Users\someone");
    let accounts = crate::accounts::list(&[], &home);
    let s = KeepAliveSettings { on: true, consented: true, folders: vec![r"C:\x\example-repo".into()], desktop_app: false };
    let want = wanted_for(&s, &accounts);
    assert_eq!(want, vec![Spec::Folder(r"C:\x\example-repo".into()), Spec::AccountFolder(r"C:\x\example-repo".into(), accounts[1].clone())]);
    assert_ne!(want[0].id(), want[1].id(), "side by side");
    assert!(wanted_for(&KeepAliveSettings { on: false, ..s.clone() }, &accounts).is_empty());
}

#[test]
fn remote_runbook_limits_come_from_settings_per_client_and_on_the_pc() {
    let (k1, k2) = (signer(7), signer(8));
    let mut s = RemoteSettings { on: true, consented: true, ..Default::default() };
    for (n, k) in [("account-1", &k1), ("account-2", &k2)] {
        s.add_client(client(n, k)).unwrap();
    }
    // Defaults: 5 per client, 10 on the PC (owner 2026-11-20, D6).
    assert_eq!(s.runbook_limits(), (DEFAULT_RUNBOOKS_PER_HOUR, DEFAULT_RUNBOOKS_PER_HOUR_PC));
    assert_eq!((DEFAULT_RUNBOOKS_PER_HOUR, DEFAULT_RUNBOOKS_PER_HOUR_PC), (5, 10));
    let mut g = Gate::default();
    let mut n = 700;
    let mut send = |g: &mut Gate, s: &RemoteSettings, who: &str, k: &Ed25519KeyPair, cmd: &str| {
        n += 1;
        let args = if cmd == "runbook" { json!({ "name": "glim-check" }) } else { json!({}) };
        g.accept_from(s, who, &envelope(k, &task(&id(n), cmd, args)), NOW)
    };
    for _ in 0..5 {
        send(&mut g, &s, "account-1", &k1, "runbook").unwrap();
    }
    assert_eq!(send(&mut g, &s, "account-1", &k1, "runbook"), Err(Rejection::RateLimited));
    assert!(send(&mut g, &s, "account-1", &k1, "session-status").is_ok(), "other commands keep their own caps");
    for _ in 0..5 {
        send(&mut g, &s, "account-2", &k2, "runbook").unwrap();
    }
    assert_eq!(send(&mut g, &s, "account-2", &k2, "runbook"), Err(Rejection::RateLimited), "the PC's 10");
    // The owner raises them in Settings (on the PC): more go through.
    let raised = RemoteSettings { runbooks_per_hour: 7, runbooks_per_hour_pc: 12, ..s.clone() };
    assert!(send(&mut g, &raised, "account-1", &k1, "runbook").is_ok());
    assert!(send(&mut g, &raised, "account-2", &k2, "runbook").is_ok());
    assert_eq!(send(&mut g, &raised, "account-2", &k2, "runbook"), Err(Rejection::RateLimited), "the PC's 12");
    // Out of range is kept in range.
    let wild = RemoteSettings { runbooks_per_hour: 999, runbooks_per_hour_pc: 999, ..Default::default() };
    assert_eq!(wild.runbook_limits(), (20, 50));
    // The runbook's name is in the activity list (D4), not in the log line.
    let last = g.activity.iter().rev().find(|a| a.ok && a.kind == "runbook").unwrap();
    assert_eq!(last.runbook, "glim-check");
    assert!(g.activity.iter().filter(|a| a.kind != "runbook").all(|a| a.runbook.is_empty()));
}

#[test]
fn no_command_can_carry_remote_limits_or_levels() {
    let kp = signer(7);
    let s = settings_for(&kp);
    let mut g = Gate::default();
    for (n, args) in [json!({ "name": "glim-check", "level": "cloud" }), json!({ "name": "glim-check", "runbooksPerHour": 20 }), json!({ "runbooksPerHour": 20 })].into_iter().enumerate() {
        let r = g.accept(&s, &envelope(&kp, &task(&id(800 + n), "runbook", args)), NOW);
        assert_eq!(r.err(), Some(Rejection::BadArgs));
    }
    for (n, cmd) in ["remote-limits", "set-limits", "runbook-level"].into_iter().enumerate() {
        let r = g.accept(&s, &envelope(&kp, &task(&id(820 + n), cmd, json!({}))), NOW);
        assert_eq!(r.err(), Some(Rejection::UnknownCommand));
    }
}
