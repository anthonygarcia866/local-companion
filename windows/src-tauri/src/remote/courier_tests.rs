// The courier against a stand-in relay on 127.0.0.1 (net::relay::testing):
// tasks in, signed results out, both signature directions, expiry, and
// reconnecting after a dropped line, a silent relay or a sleep.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde_json::{json, Value};

use super::courier::{self, Courier, Link, Loop};
use super::exec::Exec;
use super::*;
use crate::net::relay::{testing, Timing};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn seed_pair(seed: &str) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&unhex(seed)).unwrap()
}

/// RFC 8032's published test seeds (never real keys), as relay/test uses.
const CLOUD_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PC_SEED: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
const VECTOR_TASK: &str = r#"{"v":1,"payload":"{\"id\":\"vector-task-0000001\",\"iat\":1763140000,\"exp\":1763140300,\"cmd\":\"session-status\",\"args\":{}}","sig":"3f3122e3715f013f73c6dd9498fbef70e0802e6cd6e2831c736ac8a70b4e343dd5f8fe0cf735f6513243e088f5910da37336ad2396465cc762e648760ceb0803"}"#;
const VECTOR_RESULT: &str = r#"{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000001\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"f8f6581ff78ddae585b53f6b1bbf709dd2475b4bf14b9013071daf85f86d1180b19b5c0cccbdf0adc256dc09d54e492ec8d9c871a40614620e3c8be7d1303a05"}"#;

#[test]
fn the_shared_vectors_a_node_signed_task_verifies_and_glims_result_matches_byte_for_byte() {
    let cloud = seed_pair(CLOUD_SEED);
    let s = RemoteSettings { on: true, consented: true, public_key: hex(cloud.public_key().as_ref()), ..Default::default() };
    let a = Gate::default().accept(&s, VECTOR_TASK.as_bytes(), 1_763_140_010).unwrap();
    assert_eq!((a.id.as_str(), a.command.kind()), ("vector-task-0000001", "session-status"));
    let pc = seed_pair(PC_SEED);
    let out = courier::sign_result(&pc, "vector-task-0000001", 1_763_140_003, &Ok(("session-status".into(), "local-companion: idle".into())));
    assert_eq!(out, VECTOR_RESULT);
    assert_eq!(hex(pc.public_key().as_ref()), "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
}

#[test]
fn setup_keeps_the_pc_token_and_key_in_the_vault_only_hashes_as_secrets_and_never_the_cloud_token() {
    let token = crate::ai::key::MemoryVault::default();
    let key = crate::ai::key::MemoryVault::default();
    let store = crate::secrets::MemoryStore::default();
    let done = courier::setup(&token, &key, &store).unwrap();
    let pc = crate::ai::key::Vault::read(&token).unwrap().unwrap();
    assert_eq!((pc.len(), done.cloud_token.len()), (64, 64));
    assert_ne!(pc, done.cloud_token);
    let signer = courier::load_signer(&key).unwrap();
    assert_eq!(hex(signer.public_key().as_ref()), done.pc_public_key);
    // Straight from the test store (only runbooks may reveal a value; a test scans).
    let hash = |n: &str| store.0.borrow().iter().find(|(k, _)| k.ends_with(n)).map(|(_, (v, _))| v.clone()).unwrap();
    assert_eq!(hash(courier::PC_TOKEN_HASH_SECRET), courier::sha256_hex(&pc));
    assert_eq!(hash(courier::CLOUD_TOKEN_HASH_SECRET), courier::sha256_hex(&done.cloud_token));
    let everything = format!("{:?}{:?}{:?}", crate::ai::key::Vault::read(&key), crate::secrets::list(&store), pc);
    assert!(!everything.contains(&done.cloud_token));
    // A second setup replaces all of it.
    let again = courier::setup(&token, &key, &store).unwrap();
    assert_ne!(again.cloud_token, done.cloud_token);
    assert_ne!(again.pc_public_key, done.pc_public_key);
}

struct Fake {
    delay: Duration,
    runs: Arc<Mutex<Vec<String>>>,
}

impl Exec for Fake {
    fn run(&self, c: &Command) -> Result<String, String> {
        std::thread::sleep(self.delay);
        self.runs.lock().unwrap().push(c.kind().to_string());
        Ok(format!("did {}", c.kind()))
    }
}

fn task(kp: &Ed25519KeyPair, id: &str, iat: i64, exp: i64) -> String {
    let p = json!({ "id": id, "iat": iat, "exp": exp, "cmd": "session-status", "args": {} }).to_string();
    let env = json!({ "v": 1, "payload": p, "sig": hex(kp.sign(p.as_bytes()).as_ref()) }).to_string();
    json!({ "t": "task", "env": env }).to_string()
}

fn fresh(kp: &Ed25519KeyPair, id: &str) -> String {
    let now = now_s();
    task(kp, id, now - 1, now + 299)
}

/// A result frame, opened and checked with the PC's public key.
fn open(frame: &str, pc_pub: &[u8]) -> Result<Value, &'static str> {
    let f: Value = serde_json::from_str(frame).map_err(|_| "bad-frame")?;
    let env: Value = serde_json::from_str(f["env"].as_str().ok_or("bad-frame")?).map_err(|_| "bad-frame")?;
    let payload = env["payload"].as_str().ok_or("bad-frame")?;
    let sig = unhex(env["sig"].as_str().ok_or("bad-frame")?);
    UnparsedPublicKey::new(&ED25519, pc_pub).verify(payload.as_bytes(), &sig).map_err(|_| "bad-result-signature")?;
    let p: Value = serde_json::from_str(payload).map_err(|_| "bad-frame")?;
    if p["id"] != f["id"] {
        return Err("id-mismatch");
    }
    Ok(p)
}

struct Rig {
    settings: Arc<Mutex<RemoteSettings>>,
    running: Arc<AtomicBool>,
    pc_pub: Vec<u8>,
    cloud: Ed25519KeyPair,
    runs: Arc<Mutex<Vec<String>>>,
}

const SHORT: Duration = Duration::from_secs(5);

fn rig_loop(url: &str, delay: Duration, timing: Timing) -> (Rig, Loop) {
    let cloud = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let pc = Arc::new(Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap());
    let settings = Arc::new(Mutex::new(RemoteSettings {
        on: true,
        consented: true,
        public_key: hex(cloud.public_key().as_ref()),
        relay: "https://stand-in.example.workers.dev".into(),
        ..Default::default()
    }));
    let running = Arc::new(AtomicBool::new(true));
    let gate = Arc::new(Mutex::new(Gate::default()));
    let runs = Arc::new(Mutex::new(Vec::new()));
    let url = crate::net::Url::parse(url).unwrap();
    let s2 = settings.clone();
    let r2 = running.clone();
    let signer = pc.clone();
    let l = Loop {
        settings: Arc::new(move || s2.lock().unwrap().clone()),
        link: Arc::new(move |_s: &RemoteSettings| Ok(Link { url: url.clone(), token: "pc-token-for-tests".into(), signer: signer.clone() })),
        accept: Arc::new(move |s: &RemoteSettings, raw: &[u8], now: i64| gate.lock().unwrap().accept(s, raw, now)),
        exec: Arc::new(Fake { delay, runs: runs.clone() }),
        timing,
        backoff_max: Duration::from_millis(50),
        idle: Duration::from_millis(20),
        running: Arc::new(move || r2.load(Ordering::SeqCst)),
    };
    (Rig { settings, running, pc_pub: pc.public_key().as_ref().to_vec(), cloud, runs }, l)
}

fn quick() -> Timing {
    Timing { tick: Duration::from_millis(20), ping_every: Duration::from_millis(200), pong_within: Duration::from_millis(300), sleep_jump: Duration::from_secs(15) }
}

/// Runs `body` with the courier going in the background.
fn with_courier<F, Fut>(delay: Duration, timing: Timing, body: F)
where
    F: FnOnce(testing::Server, Rig) -> Fut,
    Fut: std::future::Future<Output = Rig>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let server = testing::server().await;
        let (rig, l) = rig_loop(&server.url, delay, timing);
        let handle = tokio::task::spawn_local(courier::run(l));
        let rig = body(server, rig).await;
        rig.running.store(false, Ordering::SeqCst);
        let _ = tokio::time::timeout(SHORT, handle).await;
    });
}

async fn accept(server: &testing::Server) -> testing::Conn {
    tokio::time::timeout(SHORT, server.accept()).await.expect("the courier connected")
}

#[test]
fn a_signed_task_gets_a_signed_result_and_the_pc_token_is_the_bearer() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        assert_eq!(c.auth, "Bearer pc-token-for-tests");
        c.send(&fresh(&rig.cloud, "task-ok-000000001")).await;
        let frame = c.recv(SHORT).await.expect("a result");
        let r = open(&frame, &rig.pc_pub).unwrap();
        assert_eq!((r["id"].as_str(), r["ok"].as_bool(), r["kind"].as_str(), r["detail"].as_str()), (Some("task-ok-000000001"), Some(true), Some("session-status"), Some("did session-status")));
        // Tampered on the way, the result no longer verifies (cloud Glim refuses it).
        let bad = frame.replace("did session-status", "did something else");
        assert_eq!(open(&bad, &rig.pc_pub).err(), Some("bad-result-signature"));
        // Nor with another key.
        let other = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
        assert_eq!(open(&frame, other.public_key().as_ref()).err(), Some("bad-result-signature"));
        rig
    });
}

#[test]
fn bad_signatures_and_expired_tasks_are_refused_with_a_signed_answer_and_never_run() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        let stranger = Ed25519KeyPair::from_seed_unchecked(&[5; 32]).unwrap();
        c.send(&fresh(&stranger, "task-badsig-00001")).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["ok"].as_bool(), r["error"].as_str(), r["id"].as_str()), (Some(false), Some("bad-signature"), Some("task-badsig-00001")));
        let now = now_s();
        c.send(&task(&rig.cloud, "task-expired-0001", now - 900, now - 600)).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("expired")));
        assert!(rig.runs.lock().unwrap().is_empty());
        rig
    });
}

#[test]
fn after_the_line_drops_mid_task_the_courier_reconnects_and_delivers_the_result() {
    with_courier(Duration::from_millis(300), quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        let frame = fresh(&rig.cloud, "task-sleep-000001");
        c.send(&frame).await;
        // The PC "sleeps": the line dies with no goodbye while the task runs.
        c.drop_abruptly();
        let mut c2 = accept(&server).await;
        let first = c2.recv(SHORT).await.expect("the result, after reconnecting");
        let r = open(&first, &rig.pc_pub).unwrap();
        assert_eq!((r["id"].as_str(), r["ok"].as_bool()), (Some("task-sleep-000001"), Some(true)));
        // The relay hands the unanswered task over again: same answer, not a replay refusal, run once.
        c2.send(&frame).await;
        assert_eq!(c2.recv(SHORT).await.as_deref(), Some(first.as_str()));
        assert_eq!(rig.runs.lock().unwrap().len(), 1);
        rig
    });
}

#[test]
fn a_relay_that_stops_answering_pings_is_left_and_reconnected() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        c.answer_pings = false;
        // Read (and ignore) pings so the line looks alive but silent.
        let _ = c.recv(Duration::from_millis(800)).await;
        let mut c2 = accept(&server).await;
        c2.send(&fresh(&rig.cloud, "task-after-pong-1")).await;
        assert!(open(&c2.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap()["ok"].as_bool().unwrap());
        rig
    });
}

#[test]
fn the_kill_switch_drops_the_line_and_nothing_reconnects() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        rig.settings.lock().unwrap().killed = true;
        assert_eq!(c.recv(SHORT).await, None, "the courier closed the line");
        assert!(tokio::time::timeout(Duration::from_millis(700), server.accept()).await.is_err(), "no reconnect while stopped");
        // "Allow again": it comes back.
        rig.settings.lock().unwrap().killed = false;
        let _c2 = accept(&server).await;
        rig
    });
}

#[test]
fn off_or_not_consented_means_no_connection_at_all() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let _c = accept(&server).await;
        rig.settings.lock().unwrap().on = false;
        // Lines close on their own; nothing new is opened.
        assert!(tokio::time::timeout(Duration::from_millis(700), server.accept()).await.is_err());
        rig
    });
    let s = RemoteSettings { on: true, consented: false, public_key: "ab".repeat(32), relay: "https://a.b.workers.dev".into(), ..Default::default() };
    assert!(!courier::wanted(&s));
    assert!(courier::wanted(&RemoteSettings { consented: true, ..s.clone() }));
    assert!(!courier::wanted(&RemoteSettings { consented: true, relay: String::new(), ..s }));
}

#[test]
fn a_frame_that_is_not_a_task_is_ignored() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let c = Courier::new(
        Arc::new(Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap()),
        Arc::new(Fake { delay: Duration::ZERO, runs: Arc::default() }),
        Arc::new(|_: &RemoteSettings, _: &[u8], _: i64| Err(Rejection::Off)),
        tx,
        Arc::default(),
    );
    for f in ["", "nonsense", r#"{"t":"pong"}"#, r#"{"t":"task"}"#, r#"{"t":"task","env":"{}"}"#] {
        c.on_frame(&RemoteSettings::default(), f, 0);
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn transcript_tails_keep_only_messages_and_their_words() {
    let lines = [
        json!({ "type": "user", "message": { "content": "Please   fix the build" } }),
        json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "On it." }, { "type": "tool_use", "name": "Bash" }] } }),
        json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": "secret output" }] } }),
        json!({ "type": "summary", "summary": "x" }),
        json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "Done: CI is green." }] } }),
    ];
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert_eq!(super::exec::tail_of(text.as_bytes(), 20), "you: Please fix the build\nclaude: On it.\nclaude: Done: CI is green.");
    assert_eq!(super::exec::tail_of(text.as_bytes(), 1), "claude: Done: CI is green.");
    assert_eq!(super::exec::project_slug(std::path::Path::new(r"C:\Users\x\local-companion")), "C--Users-x-local-companion");
}
