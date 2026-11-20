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
use super::seal;
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
const VECTOR_RESULT: &str = r#"{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000001\",\"task\":\"f39855e5975f49aac158e7d2bb272fe82a8ce7d07cfc1d5bf9c3cd6239f9792e\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"74c8f2725a5e382a8409de4dd599b745ffe9ba8fc205cba7870e58411b4b9404e3e9ef73a1f37e5494b207d2fae9dc3644f7f3a317a971a0555c6403d5e25409"}"#;

#[test]
fn the_shared_vectors_a_node_signed_task_verifies_and_glims_result_matches_byte_for_byte() {
    let cloud = seed_pair(CLOUD_SEED);
    let s = RemoteSettings { on: true, consented: true, public_key: hex(cloud.public_key().as_ref()), ..Default::default() };
    let a = Gate::default().accept(&s, VECTOR_TASK.as_bytes(), 1_763_140_010).unwrap();
    assert_eq!((a.id.as_str(), a.command.kind()), ("vector-task-0000001", "session-status"));
    let pc = seed_pair(PC_SEED);
    let out = courier::sign_result(&pc, "vector-task-0000001", &courier::sha256_hex(VECTOR_TASK), 1_763_140_003, &Ok(("session-status".into(), "local-companion: idle".into())));
    assert_eq!(out, VECTOR_RESULT);
    assert_eq!(hex(pc.public_key().as_ref()), "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
}

#[test]
fn setup_keeps_the_pc_token_and_key_in_the_vault_and_only_the_hash_as_a_secret() {
    let token = crate::ai::key::MemoryVault::default();
    let key = crate::ai::key::MemoryVault::default();
    let store = crate::secrets::MemoryStore::default();
    let enc = crate::ai::key::MemoryVault::default();
    let done = courier::setup(&token, &key, &enc, &store).unwrap();
    let pc = crate::ai::key::Vault::read(&token).unwrap().unwrap();
    assert_eq!(pc.len(), 64);
    let signer = courier::load_signer(&key).unwrap();
    assert_eq!(hex(signer.public_key().as_ref()), done.pc_public_key);
    // The encryption key: private half in its own vault entry, public in Settings.
    assert_eq!(seal::public_hex(&courier::load_enc(&enc).unwrap()), done.pc_enc_key);
    assert_ne!(done.pc_enc_key, done.pc_public_key);
    // Straight from the test store (only runbooks may reveal a value; a test scans).
    let hash = |n: &str| store.0.borrow().iter().find(|(k, _)| k.ends_with(n)).map(|(_, (v, _))| v.clone());
    assert_eq!(hash(courier::PC_TOKEN_HASH_SECRET), Some(courier::sha256_hex(&pc)));
    // Cloud tokens are each client's own: setup leaves them alone.
    assert_eq!(hash(courier::CLOUD_TOKEN_HASH_SECRET), None);
    // A second setup replaces the PC's token and key.
    let again = courier::setup(&token, &key, &enc, &store).unwrap();
    assert_ne!(again.pc_public_key, done.pc_public_key);
    assert_ne!(again.pc_enc_key, done.pc_enc_key);
    assert_ne!(crate::ai::key::Vault::read(&token).unwrap().unwrap(), pc);
}

#[test]
fn a_new_cloud_client_token_is_random_and_only_its_hash_reaches_the_relay_list() {
    let (t1, h1) = courier::new_cloud_token().unwrap();
    let (t2, _) = courier::new_cloud_token().unwrap();
    assert_eq!((t1.len(), h1.len()), (64, 64));
    assert_ne!(t1, t2);
    assert_eq!(h1, courier::sha256_hex(&t1));
    let mut s = RemoteSettings { public_key: "ab".repeat(32), ..Default::default() };
    s.add_client(super::CloudClient { name: "account-2".into(), public_key: "cd".repeat(32), token_sha256: h1.clone(), added: 5 }).unwrap();
    let store = crate::secrets::MemoryStore::default();
    courier::save_client_list(&s, &store).unwrap();
    let list = store.0.borrow().iter().find(|(k, _)| k.ends_with(courier::CLOUD_CLIENTS_SECRET)).map(|(_, (v, _))| v.clone()).unwrap();
    assert_eq!(list, format!("account-1=legacy,account-2={h1}"));
    assert!(!list.contains(&t1));
}

struct Fake {
    delay: Duration,
    runs: Arc<Mutex<Vec<String>>>,
}

impl Exec for Fake {
    fn run(&self, c: &Command, _client: &str) -> Result<super::exec::Ran, String> {
        std::thread::sleep(self.delay);
        self.runs.lock().unwrap().push(c.kind().to_string());
        Ok(format!("did {}", c.kind()).into())
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
        accept: Arc::new(move |s: &RemoteSettings, client: &str, raw: &[u8], now: i64| gate.lock().unwrap().accept_from(s, client, raw, now)),
        exec: Arc::new(Fake { delay, runs: runs.clone() }),
        timing,
        backoff_max: Duration::from_millis(50),
        idle: Duration::from_millis(20),
        running: Arc::new(move || r2.load(Ordering::SeqCst)),
        ledger: Arc::default(),
    };
    (Rig { settings, running, pc_pub: pc.public_key().as_ref().to_vec(), cloud, runs }, l)
}

fn quick() -> Timing {
    Timing { tick: Duration::from_millis(20), ping_every: Duration::from_millis(200), silent_for: Duration::from_millis(500), sleep_jump: Duration::from_secs(15) }
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
        let sent = fresh(&rig.cloud, "task-ok-000000001");
        let frame = c.recv(SHORT).await.expect("a result");
        let r = open(&frame, &rig.pc_pub).unwrap();
        // Bound to the exact envelope it answers.
        let env: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(r["task"].as_str(), Some(courier::sha256_hex(env["env"].as_str().unwrap()).as_str()));
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

/// A task frame the relay stamped with `client`.
fn from_client(kp: &Ed25519KeyPair, id: &str, client: &str) -> String {
    let mut f: Value = serde_json::from_str(&fresh(kp, id)).unwrap();
    f["client"] = json!(client);
    f.to_string()
}

#[test]
fn two_cloud_clients_work_at_the_same_time_each_with_its_own_key_and_removing_one_leaves_the_other() {
    with_courier(Duration::from_millis(200), quick(), |server, rig| async move {
        let second = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
        rig.settings.lock().unwrap().add_client(super::CloudClient { name: "account-2".into(), public_key: hex(second.public_key().as_ref()), token_sha256: "ab".repeat(32), added: 1 }).unwrap();
        let mut c = accept(&server).await;
        // Both at once: each runs (the exec is slow, so they overlap).
        c.send(&from_client(&rig.cloud, "two-clients-a-0001", "account-1")).await;
        c.send(&from_client(&second, "two-clients-b-0001", "account-2")).await;
        let mut got = Vec::new();
        for _ in 0..2 {
            let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
            got.push((r["id"].as_str().unwrap().to_string(), r["ok"].as_bool().unwrap()));
        }
        got.sort();
        assert_eq!(got, vec![("two-clients-a-0001".to_string(), true), ("two-clients-b-0001".to_string(), true)]);
        // Signed with the other client's key: refused, never run.
        c.send(&from_client(&rig.cloud, "two-clients-x-0001", "account-2")).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("wrong-client")));
        // account-1 removed: its tasks are refused, account-2's still run.
        assert!(rig.settings.lock().unwrap().remove_client("account-1"));
        c.send(&from_client(&rig.cloud, "two-clients-a-0002", "account-1")).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("unknown-client")));
        c.send(&from_client(&second, "two-clients-b-0002", "account-2")).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!(r["ok"].as_bool(), Some(true));
        assert_eq!(rig.runs.lock().unwrap().len(), 3);
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

/// Owner 2026-11-15 (relay said pc:true, a task got no answer): a half-open
/// line (open, but nothing read or answered, no close frame) is found by the
/// silence check, the courier reconnects, the relay hands the still-pending
/// task over again, and it runs exactly once (the first result again).
#[test]
fn a_dead_line_with_no_close_frame_is_left_and_the_pending_task_runs_once() {
    with_courier(Duration::from_millis(50), quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        let frame = fresh(&rig.cloud, "task-halfopen-0001");
        c.send(&frame).await;
        // The line dies silently with the task delivered: its result goes
        // into the dead socket, and nobody answers the PC's pings.
        let dead = c.go_dead();
        let began = std::time::Instant::now();
        let mut c2 = accept(&server).await;
        assert!(began.elapsed() < Duration::from_secs(3), "found within the silence window (+ a tick), {:?}", began.elapsed());
        drop(dead);
        // The relay redelivers what is still pending.
        c2.send(&frame).await;
        let mut got = None;
        while let Some(f) = c2.recv(SHORT).await {
            let r = open(&f, &rig.pc_pub).unwrap();
            if r["id"] == "task-halfopen-0001" {
                got = Some(r);
                break;
            }
        }
        let r = got.expect("the pending task's result after reconnecting");
        assert_eq!(r["ok"].as_bool(), Some(true));
        assert_eq!(rig.runs.lock().unwrap().len(), 1, "run exactly once");
        rig
    });
}

/// The same task id signed again (other bytes) is a replay: refused, never
/// run a second time.
#[test]
fn a_task_id_seen_once_is_refused_as_a_replay_and_not_run_again() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        let now = now_s();
        c.send(&task(&rig.cloud, "task-dupe-0000001", now - 1, now + 299)).await;
        assert_eq!(open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap()["ok"].as_bool(), Some(true));
        c.send(&task(&rig.cloud, "task-dupe-0000001", now, now + 290)).await;
        let r = open(&c.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("replay")));
        assert_eq!(rig.runs.lock().unwrap().len(), 1);
        rig
    });
}

/// Windows said it resumed (remote/wake.rs): the line is dropped and opened
/// again at once, even a young one (no backoff).
#[test]
fn a_resume_kick_reconnects_at_once() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let _c = accept(&server).await;
        crate::net::relay::kick(crate::net::relay::Kick::Resumed);
        let began = std::time::Instant::now();
        let mut c2 = tokio::time::timeout(Duration::from_millis(1500), server.accept()).await.expect("reconnected at once");
        assert!(began.elapsed() < Duration::from_millis(1500));
        c2.send(&fresh(&rig.cloud, "task-after-wake-1")).await;
        assert!(open(&c2.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap()["ok"].as_bool().unwrap());
        rig
    });
}

/// Owner 2026-11-15: down 10 minutes → one Down ping; back → one Back;
/// quiet hours hold both; the owner's own stop is no outage.
#[test]
fn a_relay_outage_pings_once_down_and_once_back() {
    use courier::{Outage, Ping};
    let t0 = std::time::Instant::now();
    let after = Duration::from_secs(600);
    let at = |s: u64| t0 + Duration::from_secs(s);
    let mut o = Outage::default();
    assert_eq!(o.tick(true, true, at(0), after, true), None);
    assert_eq!(o.tick(true, false, at(10), after, true), None);
    assert_eq!(o.tick(true, false, at(609), after, true), None, "9 min 59 s");
    assert_eq!(o.tick(true, false, at(611), after, true), Some(Ping::Down));
    assert_eq!(o.tick(true, false, at(1200), after, true), None, "once per outage");
    assert_eq!(o.tick(true, true, at(1300), after, true), Some(Ping::Back));
    assert_eq!(o.tick(true, true, at(1400), after, true), None);
    // Quiet hours: the Down waits for them to end; so does its Back.
    let mut o = Outage::default();
    o.tick(true, false, at(0), after, false);
    assert_eq!(o.tick(true, false, at(700), after, false), None);
    assert_eq!(o.tick(true, false, at(800), after, true), Some(Ping::Down));
    assert_eq!(o.tick(true, true, at(900), after, false), None);
    assert_eq!(o.tick(true, true, at(1000), after, true), Some(Ping::Back));
    // Switched off or stopped by the owner: nothing, and the count restarts.
    let mut o = Outage::default();
    o.tick(true, false, at(0), after, true);
    assert_eq!(o.tick(false, false, at(700), after, true), None);
    assert_eq!(o.tick(true, false, at(710), after, true), None, "a new outage starts at 710");
    assert_eq!(o.tick(true, false, at(1311), after, true), Some(Ping::Down));
    // A Down is never followed by a Back the owner's stop cut short.
    assert_eq!(o.tick(false, false, at(1400), after, true), None);
    assert_eq!(o.tick(true, true, at(1500), after, true), None);
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

/// Owner 2026-11-14: a task sent while stopped waits at the relay; after
/// "Allow again" the relay hands it over on reconnect. It is answered (signed)
/// as killed and never runs; a task sent after "Allow again" runs.
#[test]
fn a_task_queued_while_stopped_is_answered_killed_after_allow_again_and_never_runs() {
    with_courier(Duration::ZERO, quick(), |server, rig| async move {
        let mut c = accept(&server).await;
        rig.settings.lock().unwrap().killed = true;
        assert_eq!(c.recv(SHORT).await, None, "the courier closed the line");
        // Sent from the cloud while stopped (it waits at the relay).
        let sent_at = now_s();
        let queued = task(&rig.cloud, "task-queued-00001", sent_at, sent_at + 300);
        // "Allow again", as lib.rs's set_remote_killed does it.
        {
            let mut s = rig.settings.lock().unwrap();
            s.killed = false;
            s.allowed_since = sent_at + 1;
        }
        let mut c2 = accept(&server).await;
        c2.send(&queued).await;
        let r = open(&c2.recv(SHORT).await.expect("an answer"), &rig.pc_pub).unwrap();
        assert_eq!((r["id"].as_str(), r["ok"].as_bool(), r["error"].as_str()), (Some("task-queued-00001"), Some(false), Some("killed")));
        // Handed over again: the same answer, still not run.
        c2.send(&queued).await;
        let again = open(&c2.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!(again["error"].as_str(), Some("killed"));
        assert!(rig.runs.lock().unwrap().is_empty(), "never run");
        // A task sent after Allow again runs.
        let now = now_s().max(sent_at + 1);
        c2.send(&task(&rig.cloud, "task-after-allow1", now, now + 299)).await;
        let r = open(&c2.recv(SHORT).await.unwrap(), &rig.pc_pub).unwrap();
        assert_eq!((r["id"].as_str(), r["ok"].as_bool()), (Some("task-after-allow1"), Some(true)));
        assert_eq!(rig.runs.lock().unwrap().len(), 1);
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

// ── Store-and-forward (owner 2026-11-17): sealed tasks and results ──

/// RFC 7748's published X25519 test keys (never real ones): the PC's
/// encryption key ("Bob") and the result seal's ephemeral key ("Alice").
/// relay/test/relay.test.mjs checks the same bytes from the other side.
const PC_ENC_SECRET: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
const RESULT_EPHEMERAL: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
const SEALED_RK: &str = "eb2187a344e922fe15692b2e64550738db75862a77eee972920926c21e6e672c";
const SEALED_TASK: &str = r#"{"v":2,"id":"vector-task-0000002","iat":1763140000,"exp":1763226400,"epk":"75eaa546af013b14eace88be3b1fe5478f2f9a072e675516fd76fe6face3530f","n":"000102030405060708090a0b","ct":"b8ec532257db42bb3bd002f8b83b5e6dc4671c92936df8cb1f8884577b71402b097a9461cd7c08a02c89df156d4e774458d51b1988b8e0adfdeb656a793483237d94861e8f3b30a1e19125af008f10dda88e1b20d741793f89e1e49724da5ab1fb5ccbcedf30dbdf22180f6dab96d566ee7b57f03a7520299e8a8244050a138c0637687bd3692ea6039911d6cf7f1641b6fdd6fa123b7c14f1e1b807790e0cdfe7c50fad3913dd0146b2e4102bbefe1ffd6b9679d0f61387727b7cfc76f4bed6c1a2d7c2e463d46174b72e0fe934c3b8600e20ecb5b3c33f12bb2cb2066ab224e22f909015608d80bbd890a1d55a5e017c025e1a19c640b4aa7a37e667464d10ad0cb34378ea2dbbabffefe860a3f29feedae4a5c5d02c26bf2dd9bee39b110877cf7ffb092da75b53bad828b190104ff26aabcf0e1ba36dae8c8bf2f7d7be116976b516c3fa5aa1ce54abb657657475b89307588e68d5442ef6667ae37930c34d3250e0688c"}"#;
const SEALED_TASK_INNER: &str = r#"{"v":1,"payload":"{\"id\":\"vector-task-0000002\",\"iat\":1763140000,\"exp\":1763226400,\"cmd\":\"session-status\",\"args\":{},\"rk\":\"eb2187a344e922fe15692b2e64550738db75862a77eee972920926c21e6e672c\"}","sig":"228126820638786f72d342d76c58bad75b08de897dd59c81d2f68b71b908d961b7db211b3299487f790277e93c4646c5bd89bbbe7f6f6fe736b6201d6f7f3b02"}"#;
const SEALED_RESULT_INNER: &str = r#"{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000002\",\"task\":\"7a28565ea45aa4568f31a15310d56e02c18afb1227ddf8098634cf3a053b6d4a\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"5c5656aeedc84c9c20cab9a72ba70ed6311a359d0de7bbdc6fe1887ba8da8a5f7873d2e6f001b5030e26c53dff3c8ade27affec0570a800d49942f641f1ce102"}"#;
const SEALED_RESULT: &str = r#"{"v":2,"id":"vector-task-0000002","epk":"8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a","n":"a0a1a2a3a4a5a6a7a8a9aaab","ct":"fca0f22cb2c91cbd459d82a6e49defa9053992c58a55fdee2e3ec880f92f29f2561391e2d28dd8ea2d8e4fbcf0f5c880c9c924ab7344635d43826ca5cdd14e073875e85face52a6f014562d741fd9196419c62e1b714a25949e02fd40e5787ecdb3717a4290f5a4c92026ff2f4189cea5baa3d2c01c3839a0d771a5fb8337eabe1c0a45c92dcd56f653449c4fb208b33099a783cc7e3a85a4c2e1d8f5a62bf421617c568d22ceb5b6170f2e6ed5c14b56bee7166d61418cc95c56c65b388dec1f72ab2160d0be0b500e1818b9e90c650b38386228f2242307b8b44fd9a3bf69f4714ad4675d39b8045841c7781e0b3ff2c45d1b6c68520d1863cfb5900e7d872959d5453f98eaf129b6abf29872749f057d996f16c40fd55c26fb11a73d347ee049a8f4320141312bf2986dbb30e8dcc8671952d7ce53f97a20e77fee6cbe994a782007ee1af63f053bbbcc63a92313a6d02c2ab3d43abe77dcfc580eb508fe7dc47f5e8dadc586942f373ac213a40fb2ef9ad24a99b1ae5e612a39593a60dec3d7d538e343fc8dfd216df4504f7221b58"}"#;
const SEALED_ID: &str = "vector-task-0000002";

fn arr<const N: usize>(h: &str) -> [u8; N] {
    unhex(h).try_into().unwrap()
}

fn cloud_settings() -> RemoteSettings {
    RemoteSettings { on: true, consented: true, public_key: hex(seed_pair(CLOUD_SEED).public_key().as_ref()), ..Default::default() }
}

#[test]
fn the_seal_vectors_a_node_sealed_task_opens_and_glims_sealed_result_matches_byte_for_byte() {
    let pc_enc = seal::secret_from_hex(PC_ENC_SECRET).unwrap();
    assert_eq!(seal::public_hex(&pc_enc), "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f", "RFC 7748's Bob");
    let (inner, outer) = seal::open_task(Some(&pc_enc), SEALED_TASK).unwrap();
    assert_eq!(inner, SEALED_TASK_INNER);
    assert_eq!(outer, seal::Outer { id: SEALED_ID.into(), iat: 1_763_140_000, exp: 1_763_226_400 });
    let a = Gate::default().accept_sealed(&cloud_settings(), "account-1", SEALED_TASK.as_bytes(), 1_763_140_010, Some(&pc_enc)).unwrap();
    assert_eq!((a.id.as_str(), a.command.kind(), a.rk), (SEALED_ID, "session-status", Some(arr::<32>(SEALED_RK))));
    // Signed over the hash of the SEALED envelope (what cloud Glim sent), then
    // sealed to the task's result key: byte for byte what relay/test opens.
    let pc = seed_pair(PC_SEED);
    let signed = courier::sign_result(&pc, SEALED_ID, &courier::sha256_hex(SEALED_TASK), 1_763_140_003, &Ok(("session-status".into(), "local-companion: idle".into())));
    assert_eq!(signed, SEALED_RESULT_INNER);
    let s = seal::seal_with(arr::<32>(RESULT_EPHEMERAL), arr::<12>("a0a1a2a3a4a5a6a7a8a9aaab"), &arr::<32>(SEALED_RK), "result", SEALED_ID, signed.as_bytes()).unwrap();
    assert_eq!(seal::result_envelope(SEALED_ID, &s), SEALED_RESULT);
}

#[test]
fn a_sealed_task_altered_misaddressed_or_relabelled_is_refused_as_bad_seal_and_noted() {
    let pc_enc = seal::secret_from_hex(PC_ENC_SECRET).unwrap();
    let s = cloud_settings();
    let t = 1_763_140_010;
    let mut g = Gate::default();
    let other = seal::secret_from_hex(&"11".repeat(32)).unwrap();
    assert_eq!(g.accept_sealed(&s, "account-1", SEALED_TASK.as_bytes(), t, Some(&other)), Err(Rejection::BadSeal), "another PC's key");
    assert_eq!(g.accept_sealed(&s, "account-1", SEALED_TASK.as_bytes(), t, None), Err(Rejection::BadSeal), "no key on this PC");
    let edit = |f: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_str(SEALED_TASK).unwrap();
        f(&mut v);
        v.to_string()
    };
    let flipped = edit(&|v| {
        let ct = v["ct"].as_str().unwrap().to_string();
        let last = if ct.ends_with('0') { '1' } else { '0' };
        v["ct"] = json!(format!("{}{last}", &ct[..ct.len() - 1]));
    });
    // The relay stretching the outside's expiry to keep it longer, or
    // renaming it: the outside no longer matches the signed task (or the
    // AEAD's label), refused.
    let stretched = edit(&|v| v["exp"] = json!(1_763_226_400 + 3600));
    let renamed = edit(&|v| v["id"] = json!("vector-task-0000003"));
    for bad in [&flipped, &stretched, &renamed] {
        assert_eq!(g.accept_sealed(&s, "account-1", bad.as_bytes(), t, Some(&pc_enc)), Err(Rejection::BadSeal));
    }
    assert_eq!(g.activity.iter().filter(|a| a.reason == "bad-seal").count(), 5, "every one noted");
    assert!(g.activity.iter().all(|a| a.id.is_empty()), "no id from an unopened seal");
    // The kill switch comes first, sealed or not.
    let killed = RemoteSettings { killed: true, ..s.clone() };
    assert_eq!(g.accept_sealed(&killed, "account-1", flipped.as_bytes(), t, Some(&pc_enc)), Err(Rejection::Killed));
    assert_eq!(g.accept_sealed(&killed, "account-1", SEALED_TASK.as_bytes(), t, Some(&pc_enc)), Err(Rejection::Killed));
    // Anyone can seal to the PC's public key, but not sign as the client.
    let forger = Ed25519KeyPair::from_seed_unchecked(&[5; 32]).unwrap();
    let frame: Value = serde_json::from_str(&task(&forger, "forged-task-0000001", t - 5, t + 300)).unwrap();
    let inner = frame["env"].as_str().unwrap().to_string();
    let sealed = seal::seal(&arr::<32>(&seal::public_hex(&pc_enc)), "task", "forged-task-0000001", inner.as_bytes()).unwrap();
    let outer = json!({ "v": 2, "id": "forged-task-0000001", "iat": t - 5, "exp": t + 300, "epk": seal::to_hex(&sealed.epk), "n": seal::to_hex(&sealed.nonce), "ct": seal::to_hex(&sealed.ct) }).to_string();
    assert_eq!(g.accept_sealed(&s, "account-1", outer.as_bytes(), t, Some(&pc_enc)), Err(Rejection::BadSignature));
    // The genuine one still goes, once.
    assert!(g.accept_sealed(&s, "account-1", SEALED_TASK.as_bytes(), t, Some(&pc_enc)).is_ok());
    assert_eq!(g.accept_sealed(&s, "account-1", SEALED_TASK.as_bytes(), t + 1, Some(&pc_enc)), Err(Rejection::Replay));
}

#[test]
fn results_are_sealed_to_the_tasks_result_key_and_only_it_opens_them() {
    let pc = seed_pair(PC_SEED);
    let id = "result-test-000001";
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let rk = arr::<32>(&seal::public_hex(&rk_secret));
    let ok: Result<(String, String), String> = Ok(("session-status".into(), "local-companion: idle".into()));
    let out = courier::answer(&pc, id, "hash", 5, &ok, Some(&rk));
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["v"], 2);
    assert!(!out.contains("idle"), "the relay can't read it");
    let (epk, n, ct) = (arr::<32>(v["epk"].as_str().unwrap()), arr::<12>(v["n"].as_str().unwrap()), unhex(v["ct"].as_str().unwrap()));
    let inner = seal::open(&rk_secret, &epk, &n, "result", id, &ct).unwrap();
    assert_eq!(String::from_utf8(inner).unwrap(), courier::sign_result(&pc, id, "hash", 5, &ok), "the signed result, unchanged");
    assert!(seal::open(&rk_secret, &epk, &n, "result", "another-task-00001", &ct).is_none(), "bound to its id");
    assert!(seal::open(&rk_secret, &epk, &n, "task", id, &ct).is_none(), "task and result keys are apart");
    assert!(seal::open(&seal::secret_from_hex(&"43".repeat(32)).unwrap(), &epk, &n, "result", id, &ct).is_none(), "only the result key opens it");
    assert_ne!(out, courier::answer(&pc, id, "hash", 5, &ok, Some(&rk)), "a fresh ephemeral key and nonce each time");
    // No result key (an older courier): signed and unsealed, as before.
    assert_eq!(courier::answer(&pc, id, "hash", 5, &ok, None), courier::sign_result(&pc, id, "hash", 5, &ok));
    // A key that can't be sealed to (low order): a signed refusal, never the result in clear.
    let bad = courier::answer(&pc, id, "hash", 5, &ok, Some(&[0u8; 32]));
    assert!(bad.contains("bad-result-key") && !bad.contains("idle"), "{bad}");
    // Too big for the relay once sealed: its newest part that fits, still sealed.
    let big: Result<(String, String), String> = Ok(("transcript-tail".into(), format!("{}END", "€".repeat(12_000))));
    let out = courier::answer(&pc, id, "hash", 5, &big, Some(&rk));
    assert!(out.len() <= seal::MAX_SEALED_RESULT_BYTES);
    let v: Value = serde_json::from_str(&out).unwrap();
    let inner = seal::open(&rk_secret, &arr::<32>(v["epk"].as_str().unwrap()), &arr::<12>(v["n"].as_str().unwrap()), "result", id, &unhex(v["ct"].as_str().unwrap())).unwrap();
    let p = payload_of(&String::from_utf8(inner).unwrap());
    assert_eq!(p["ok"], true, "{p}");
    let d = p["detail"].as_str().unwrap();
    assert!(d.starts_with('…') && d.ends_with("END") && d.chars().count() > 5_000, "trimmed from the front, newest kept: {} chars", d.chars().count());
}

fn payload_of(signed: &str) -> Value {
    let v: Value = serde_json::from_str(signed).unwrap();
    serde_json::from_str(v["payload"].as_str().unwrap()).unwrap()
}

fn open_result(rk_secret: &seal::Secret, id: &str, out: &str) -> Value {
    let v: Value = serde_json::from_str(out).unwrap();
    let inner = seal::open(rk_secret, &arr::<32>(v["epk"].as_str().unwrap()), &arr::<12>(v["n"].as_str().unwrap()), "result", id, &unhex(v["ct"].as_str().unwrap())).unwrap();
    payload_of(&String::from_utf8(inner).unwrap())
}

#[test]
fn a_quote_heavy_transcript_tail_fits_under_the_cap_instead_of_failing() {
    let pc = seed_pair(PC_SEED);
    let id = "result-test-000002";
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let rk = arr::<32>(&seal::public_hex(&rk_secret));
    // 50 messages of 230 characters, nearly all quotes and backslashes: well
    // under 12,000 characters, but each grows to 4-8 bytes once escaped twice
    // and sealed (hex).
    // Unsealed, quotes grow only 4x and every detail is redacted (control
    // characters gone), so 12,000 characters always fit: no trimming.
    let quotes = format!("you: {}", "\"\\".repeat(5_990));
    let p = payload_of(&courier::answer(&pc, id, "hash", 5, &Ok(("transcript-tail".into(), quotes.clone())), None));
    assert_eq!((p["detail"].as_str(), p["capped"].as_bool()), (Some(quotes.as_str()), Some(false)));
    {
        let (rk, filler) = (Some(&rk), "\"\\");
        let lines: Vec<String> = (0..50).map(|i| format!("you: msg{i:02} {}", filler.repeat(110))).collect();
        let detail = lines.join("\n");
        assert!(detail.chars().count() <= super::exec::MAX_DETAIL_CHARS);
        let tail: Result<(String, String), String> = Ok(("transcript-tail".into(), detail.clone()));
        let out = courier::answer(&pc, id, "hash", 5, &tail, rk);
        assert!(out.len() <= courier::MAX_RESULT_BYTES, "{} bytes", out.len());
        let p = if rk.is_some() { open_result(&rk_secret, id, &out) } else { payload_of(&out) };
        assert_eq!(p["ok"], true, "{p}");
        assert_eq!(p["error"], "");
        let d = p["detail"].as_str().unwrap();
        assert!(d.starts_with("…you: msg"), "starts at a whole message");
        assert!(d.ends_with(lines.last().unwrap().as_str()), "the newest message kept whole");
        assert!(d.len() < detail.len());
        // Only whole messages: every kept line is one of the originals.
        assert!(d.trim_start_matches('…').lines().all(|l| lines.iter().any(|o| o == l)));
    }
    // Small enough already: unchanged.
    let small: Result<(String, String), String> = Ok(("transcript-tail".into(), "you: \"hi\"\nclaude: \"hello\"".into()));
    assert_eq!(payload_of(&courier::answer(&pc, id, "hash", 5, &small, None))["detail"], "you: \"hi\"\nclaude: \"hello\"");
}

#[test]
fn fit_detail_keeps_the_longest_tail_that_fits_or_gives_up() {
    // A made-up size: 100 bytes a character.
    let make = |d: &str| "x".repeat(d.chars().count() * 100);
    let detail = format!("old line\n{}", "n".repeat(2_000));
    let got = courier::fit_detail(&detail, make).unwrap();
    assert!(got.len() <= courier::MAX_RESULT_BYTES && got.len() > courier::MAX_RESULT_BYTES - 200);
    // Nothing fits, not even the marker: None (the caller answers result-too-big).
    assert!(courier::fit_detail(&detail, |_| "x".repeat(courier::MAX_RESULT_BYTES + 1)).is_none());
    // tail_chars: whole lines when a break is in the part kept.
    let c: Vec<char> = "one\ntwo\nthree".chars().collect();
    assert_eq!(courier::tail_chars(&c, 7), "…three", "mid line: from the next whole line");
    assert_eq!(courier::tail_chars(&c, 9), "…two\nthree", "already at a line's start: kept");
    assert_eq!(courier::tail_chars(&c, 5), "…three");
    assert_eq!(courier::tail_chars(&c, 3), "…ree");
    assert_eq!(courier::tail_chars(&c, 13), "one\ntwo\nthree");
}

#[test]
fn a_refusal_that_cannot_fit_is_still_answered() {
    let pc = seed_pair(PC_SEED);
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let rk = arr::<32>(&seal::public_hex(&rk_secret));
    let out = courier::answer(&pc, "result-test-000003", "hash", 5, &Err("busy".into()), Some(&rk));
    assert_eq!(open_result(&rk_secret, "result-test-000003", &out)["error"], "busy");
}

#[test]
fn a_sealed_task_through_the_courier_gets_a_sealed_signed_result() {
    let pc_enc = seal::secret_from_hex(PC_ENC_SECRET).unwrap();
    let pc_pub = arr::<32>(&seal::public_hex(&pc_enc));
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let cloud = seed_pair(CLOUD_SEED);
    let now = super::now_s();
    let id = "courier-sealed-0001";
    let p = json!({ "id": id, "iat": now, "exp": now + 3600, "cmd": "session-status", "args": {}, "rk": seal::public_hex(&rk_secret) }).to_string();
    let inner = json!({ "v": 1, "payload": p, "sig": hex(cloud.sign(p.as_bytes()).as_ref()) }).to_string();
    let s = seal::seal(&pc_pub, "task", id, inner.as_bytes()).unwrap();
    let outer = json!({ "v": 2, "id": id, "iat": now, "exp": now + 3600, "epk": seal::to_hex(&s.epk), "n": seal::to_hex(&s.nonce), "ct": seal::to_hex(&s.ct) }).to_string();
    let gate = Arc::new(Mutex::new(Gate::default()));
    let (g2, enc2) = (gate.clone(), pc_enc.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let c = Courier::new(
        Arc::new(seed_pair(PC_SEED)),
        Arc::new(Fake { delay: Duration::ZERO, runs: Arc::default() }),
        Arc::new(move |s: &RemoteSettings, client: &str, raw: &[u8], now: i64| g2.lock().unwrap().accept_sealed(s, client, raw, now, Some(&enc2))),
        tx,
        Arc::default(),
    );
    c.on_frame(&cloud_settings(), &json!({ "t": "task", "env": outer, "client": "account-1" }).to_string(), now);
    let frame: Value = serde_json::from_str(&rx.blocking_recv().unwrap()).unwrap();
    assert_eq!(frame["id"], id);
    let env: Value = serde_json::from_str(frame["env"].as_str().unwrap()).unwrap();
    assert_eq!(env["v"], 2, "sealed");
    let opened = seal::open(&rk_secret, &arr::<32>(env["epk"].as_str().unwrap()), &arr::<12>(env["n"].as_str().unwrap()), "result", id, &unhex(env["ct"].as_str().unwrap())).unwrap();
    let signed: Value = serde_json::from_slice(&opened).unwrap();
    let payload = signed["payload"].as_str().unwrap();
    assert!(UnparsedPublicKey::new(&ED25519, seed_pair(PC_SEED).public_key().as_ref()).verify(payload.as_bytes(), &unhex(signed["sig"].as_str().unwrap())).is_ok());
    let r: Value = serde_json::from_str(payload).unwrap();
    assert_eq!((r["ok"].as_bool(), r["detail"].as_str(), r["task"].as_str()), (Some(true), Some("did session-status"), Some(courier::sha256_hex(&outer).as_str())));
    // The result format (owner 2026-11-19): id, client, status, created /
    // started / finished, capped, and no session for session-status.
    assert_eq!((r["id"].as_str(), r["client"].as_str(), r["status"].as_str()), (Some(id), Some("account-1"), Some("done")));
    assert_eq!(r["created"].as_i64(), Some(now), "the task's own iat");
    let (started, finished) = (r["started"].as_i64().unwrap(), r["finished"].as_i64().unwrap());
    assert!(started >= now && finished >= started && r["at"].as_i64() == Some(finished), "{r}");
    assert_eq!((r["session"].is_null(), r["capped"].as_bool()), (true, Some(false)));
    // A seal that doesn't open: answered at once, signed, unsealed, with a fixed code.
    c.on_frame(&cloud_settings(), &json!({ "t": "task", "env": outer.replace("\"n\":\"", "\"n\":\"ff"), "client": "account-1" }).to_string(), now);
    let refused: Value = serde_json::from_str(&rx.blocking_recv().unwrap()).unwrap();
    let env: Value = serde_json::from_str(refused["env"].as_str().unwrap()).unwrap();
    assert_eq!(env["v"], 1);
    assert!(env["payload"].as_str().unwrap().contains("\"error\":\"bad-envelope\"") || env["payload"].as_str().unwrap().contains("\"error\":\"bad-seal\""));
    // A refusal: it never ran, nothing of the task's is trusted but the client the relay named.
    let r: Value = serde_json::from_str(env["payload"].as_str().unwrap()).unwrap();
    assert_eq!((r["status"].as_str(), r["client"].as_str(), r["ok"].as_bool()), (Some("refused"), Some("account-1"), Some(false)));
    assert!(r["created"].is_null() && r["started"].is_null() && r["session"].is_null() && r["finished"].as_i64() == Some(now), "{r}");
}

#[test]
fn every_result_carries_status_times_session_and_a_redacted_capped_detail() {
    let pc = seed_pair(PC_SEED);
    let id = "format-test-000001";
    let m = courier::Meta { client: "account-2".into(), created: Some(100), started: Some(110), session: Some("3f2c1a9e-0b4d-4c8e-9a7b-1d2e3f405162".into()), capped: false };
    let ok: Result<(String, String), String> = Ok(("start-session".into(), "started, session 3f2c1a9e-0b4d-4c8e-9a7b-1d2e3f405162".into()));
    let p = payload_of(&courier::answer_meta(&pc, id, "hash", 120, &ok, None, &m));
    assert_eq!(p["status"], "done");
    assert_eq!((p["created"].as_i64(), p["started"].as_i64(), p["finished"].as_i64(), p["at"].as_i64()), (Some(100), Some(110), Some(120), Some(120)));
    assert_eq!(p["session"], "3f2c1a9e-0b4d-4c8e-9a7b-1d2e3f405162");
    assert_eq!(p["client"], "account-2");
    assert!(p["detail"].as_str().unwrap().contains("3f2c1a9e-0b4d-4c8e-9a7b-1d2e3f405162"), "session ids are never masked");
    // It ran and failed: failed; never ran: refused.
    let p = payload_of(&courier::answer_meta(&pc, id, "hash", 120, &Err("busy".into()), None, &m));
    assert_eq!((p["status"].as_str(), p["error"].as_str(), p["detail"].as_str()), (Some("failed"), Some("busy"), Some("")));
    let p = payload_of(&courier::sign_result(&pc, id, "hash", 120, &Err("killed".into())));
    assert_eq!(p["status"], "refused");
    // Capped: over 12,000 characters, or trimmed to fit the relay.
    let long: Result<(String, String), String> = Ok(("transcript-tail".into(), "x".repeat(12_500)));
    let p = payload_of(&courier::answer_meta(&pc, id, "hash", 120, &long, None, &m));
    assert_eq!((p["capped"].as_bool(), p["detail"].as_str().unwrap().chars().count()), (Some(true), 12_001));
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let rk = arr::<32>(&seal::public_hex(&rk_secret));
    let quotes: Result<(String, String), String> = Ok(("transcript-tail".into(), "\"".repeat(13_000)));
    let p = open_result(&rk_secret, id, &courier::answer_meta(&pc, id, "hash", 120, &quotes, Some(&rk), &m));
    assert_eq!((p["capped"].as_bool(), p["ok"].as_bool(), p["session"].as_str()), (Some(true), Some(true), m.session.as_deref()));
    // Redacted on the way out, whatever the command.
    let leak: Result<(String, String), String> = Ok(("runbook".into(), format!("deploy: key sk-ant-api03-{} ok", "a1".repeat(20))));
    let d = payload_of(&courier::answer_meta(&pc, id, "hash", 120, &leak, None, &m))["detail"].as_str().unwrap().to_string();
    assert_eq!(d, "deploy: key [secret] ok");
}

#[test]
fn result_details_are_redacted_but_ids_and_hashes_kept() {
    use super::exec::redact_detail as r;
    let gh = format!("ghp_{}", "Ab1".repeat(10));
    assert_eq!(r(&format!("token {gh} here")), "token [secret] here");
    assert_eq!(r("you: password: hunter2 please"), "you: password: [secret] please");
    assert_eq!(r("DB_PASSWORD=hunter2"), "DB_PASSWORD=[secret]");
    assert_eq!(r("api_key = abc123"), "api_key = [secret]");
    assert_eq!(r("Authorization: Bearer abcdefghijklmnop1234"), "Authorization: Bearer [secret]");
    assert_eq!(r(&format!("stripe sk_live_{} and whsec_{}", "x9".repeat(12), "y8".repeat(12))), "stripe [secret] and [secret]");
    assert_eq!(r("you: here -----BEGIN PRIVATE KEY----- MIIEvQ abc -----END PRIVATE KEY----- thanks"), "you: here [secret] thanks");
    // A BEGIN with no END (prose, or cut short): that line only, never the messages after it.
    assert_eq!(r("you: grep for -----BEGIN in the file\nclaude: found it"), "you: grep for [secret]\nclaude: found it");
    // JSON, hyphenated, compound and flag forms; URLs; tabs.
    assert_eq!(r("{\"password\":\"hunter2\",\"user\":\"sam\"}"), "password\":[secret]");
    assert_eq!(r("\"token\": \"abc\", \"user\": \"sam\""), "\"token\": [secret] \"user\": \"sam\"");
    assert_eq!(r("x-api-key: abc123"), "x-api-key: [secret]");
    assert_eq!(r("AWS_SECRET_ACCESS_KEY=abc123"), "AWS_SECRET_ACCESS_KEY=[secret]");
    assert_eq!(r("run --password hunter2 now"), "run --password [secret] now");
    assert_eq!(r("open https://x.example/cb?state=1&token=abc123&b=2"), "open https://x.example/cb?state=1&token=[secret]&b=2");
    assert_eq!(r("db postgres://sam:pw123@db.example:5432/app"), "db postgres://sam:[secret]@db.example:5432/app");
    assert_eq!(r(&format!("npm_{} and hf_{}", "a1".repeat(10), "b2".repeat(10))), "[secret] and [secret]");
    assert_eq!(r("token\t=\tabc"), "token\t=\t[secret]");
    // Security review 2026-11-19: every PEM block on a line, not just the first.
    let two = "a -----BEGIN KEY----- AAA -----END KEY----- b -----BEGIN PRIVATE KEY----- BBB -----END PRIVATE KEY----- c";
    assert_eq!(r(two), "a [secret] b [secret] c");
    assert_eq!(r("x -----BEGIN KEY----- AAA -----END KEY----- y -----BEGIN KEY----- CCC"), "x [secret] y [secret]");
    // A header with data on its line, the body on the next lines: all of it,
    // up to END; the message after it is kept.
    let body = "-----BEGIN PRIVATE KEY----- MIIEvQIBADAN\nBgkqhkiG9w0BAQEFAASC\nBKcwggSjAgEAAoIBAQC7\n-----END PRIVATE KEY-----\nclaude: done";
    assert_eq!(r(body), "[secret]\nclaude: done");
    let enc = "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,ABC\n\nMIIEpAIBAAKCAQEA\n-----END RSA PRIVATE KEY----- after";
    assert_eq!(r(enc), "[secret]\n after");
    // A URL is masked AND still checked as a word.
    assert_eq!(r("password=hunter2&token=abc"), "password=[secret]");
    assert_eq!(r("line1\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nline5"), "line1\n[secret]\nline5");
    assert_eq!(r("bell\u{7} and tab\tkept"), "bell and tab\tkept");
    // Kept: prose, session ids, commit hashes, the courier's own fixed lines.
    for fine in [
        "the token works now and the secret is patience",
        "local-companion: idle, last activity 3 min ago (session 3f2c1a9e-0b4d-4c8e-9a7b-1d2e3f405162) [Claude]",
        "merged 5288756a0b2b7f4c1d9e8f7a6b5c4d3e2f1a0b9c",
        "keep-alive: restarted 1 session(s) [Claude2]",
    ] {
        assert_eq!(r(fine), fine);
    }
}

#[test]
fn start_session_names_the_session_it_starts() {
    for _ in 0..20 {
        let id = super::exec::new_session_id().unwrap();
        assert!(super::command::is_session_id(&id), "{id}");
        assert_eq!(id.as_bytes()[14], b'4', "v4");
    }
    assert_ne!(super::exec::new_session_id().unwrap(), super::exec::new_session_id().unwrap());
    // The real command: `--session-id <that id>`, and the result names it.
    let src = include_str!("exec.rs");
    assert!(src.contains(r#""-p", "-w", "--session-id", id.as_str()"#));
    assert!(src.contains("session: Some(id)") && src.contains("session: Some(session.clone())"));
}

#[test]
fn a_task_that_ran_gets_its_first_answer_again_after_a_restart_never_replay() {
    let dir = std::env::temp_dir().join(format!("glim-ledger-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("remote-answers.json");
    let _ = std::fs::remove_file(&path);
    let pc_enc = seal::secret_from_hex(PC_ENC_SECRET).unwrap();
    let pc_pub = arr::<32>(&seal::public_hex(&pc_enc));
    let rk_secret = seal::secret_from_hex(&"42".repeat(32)).unwrap();
    let cloud = seed_pair(CLOUD_SEED);
    let now = super::now_s();
    let id = "ledger-task-000001";
    let p = json!({ "id": id, "iat": now, "exp": now + 3600, "cmd": "session-status", "args": {}, "rk": seal::public_hex(&rk_secret) }).to_string();
    let inner = json!({ "v": 1, "payload": p, "sig": hex(cloud.sign(p.as_bytes()).as_ref()) }).to_string();
    let s = seal::seal(&pc_pub, "task", id, inner.as_bytes()).unwrap();
    let outer = json!({ "v": 2, "id": id, "iat": now, "exp": now + 3600, "epk": seal::to_hex(&s.epk), "n": seal::to_hex(&s.nonce), "ct": seal::to_hex(&s.ct) }).to_string();
    let frame = json!({ "t": "task", "env": outer, "client": "account-1" }).to_string();
    // One gate for both "runs" of the app (its seen ids are on disk too).
    let gate = Arc::new(Mutex::new(Gate::default()));
    let runs: Arc<Mutex<Vec<String>>> = Arc::default();
    let make = |tx| {
        let (g, enc) = (gate.clone(), pc_enc.clone());
        Courier::new(
            Arc::new(seed_pair(PC_SEED)),
            Arc::new(Fake { delay: Duration::from_millis(300), runs: runs.clone() }),
            Arc::new(move |s: &RemoteSettings, client: &str, raw: &[u8], now: i64| g.lock().unwrap().accept_sealed(s, client, raw, now, Some(&enc))),
            tx,
            Arc::default(),
        )
        .with_ledger(Arc::new(Mutex::new(courier::Ledger::load(path.clone()))))
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let first = make(tx);
    first.on_frame(&cloud_settings(), &frame, now);
    // Handed over again while it still runs (a reconnect): nothing yet.
    first.on_frame(&cloud_settings(), &frame, now);
    let answer = rx.blocking_recv().unwrap();
    assert!(rx.try_recv().is_err(), "one answer, not a replay refusal");
    // The app restarts (no memory left) and the relay hands it over again.
    let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
    make(tx2).on_frame(&cloud_settings(), &frame, now + 5);
    assert_eq!(rx2.blocking_recv().unwrap(), answer, "the first answer again");
    assert_eq!(runs.lock().unwrap().len(), 1, "it ran once");
    // On disk: the sealed result only, never the result text.
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(disk.contains(id) && !disk.contains("did session-status") && !disk.contains("session-status"), "sealed: {disk}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unsealed_success_is_remembered_only_as_already_ran() {
    let mut l = courier::Ledger::default();
    l.start("k");
    assert_eq!(l.lookup("k", 10), Some(courier::Seen::Running));
    l.finish("k", 100, None);
    assert_eq!(l.lookup("k", 50), Some(courier::Seen::Answered(None)));
    assert_eq!(l.lookup("k", 101), None, "forgotten after the task's expiry");
}

/// The sealing crate and ring's AEAD / HKDF stay in seal.rs.
#[test]
fn only_seal_rs_uses_the_sealing_crypto() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let words = [concat!("x25519", "_dalek"), concat!("ring::", "aead"), concat!("ring::", "hkdf")];
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("remote/seal.rs") && !path.ends_with(r"remote\seal.rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for w in words {
                    assert!(!text.contains(w), "{} uses {w}", path.display());
                }
            }
        }
    }
}

#[test]
fn a_pc_set_up_before_sealing_gets_its_encryption_key_once() {
    let v = crate::ai::key::MemoryVault::default();
    let first = courier::ensure_enc_key(&v).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(courier::ensure_enc_key(&v).unwrap(), first, "kept, not remade");
    // An entry that doesn't parse is an error, and it is left as it was.
    let bad = crate::ai::key::MemoryVault::default();
    crate::ai::key::Vault::write(&bad, "not a key").unwrap();
    assert!(courier::ensure_enc_key(&bad).is_err());
    assert_eq!(crate::ai::key::Vault::read(&bad).unwrap().as_deref(), Some("not a key"));
}

#[test]
fn a_frame_that_is_not_a_task_is_ignored() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let c = Courier::new(
        Arc::new(Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap()),
        Arc::new(Fake { delay: Duration::ZERO, runs: Arc::default() }),
        Arc::new(|_: &RemoteSettings, _: &str, _: &[u8], _: i64| Err(Rejection::Off)),
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
    // 50 long messages (2026-11-19): up to 12,000 characters of WHOLE
    // messages, newest last; it used to be cut to 4,000 mid message by the
    // runbook redactor's own cap. Control characters still go.
    let long: String = (0..50).map(|i| json!({ "type": "user", "message": { "content": format!("m{i:02}\u{7} {}", "word ".repeat(80)) } }).to_string() + "\n").collect();
    let t = super::exec::tail_of(long.as_bytes(), 50);
    let n = t.chars().count();
    assert!(n > 11_000 && n <= super::exec::MAX_DETAIL_CHARS, "{n}");
    assert!(t.starts_with("…you: m") && t.contains("you: m49 ") && !t.contains('\u{7}'), "older ones left out, marked");
    assert!(t.lines().all(|l| l.trim_start_matches('…').starts_with("you: m") && l.ends_with('…')), "whole (400-character) messages only");
    assert_eq!(super::exec::project_slug(std::path::Path::new(r"C:\Users\x\local-companion")), "C--Users-x-local-companion");
    // The folder as runbooks::allowed_folder hands it over: canonicalized,
    // with Windows' verbatim prefix (2026-11-14: session-status said "no
    // Claude Code session yet" because of it). Same name as above.
    assert_eq!(super::exec::project_slug(std::path::Path::new(r"\\?\C:\Users\x\local-companion")), "C--Users-x-local-companion");
    assert_eq!(super::exec::project_slug(std::path::Path::new(r"\\?\UNC\server\share\repo")), "--server-share-repo");
    // Dots and spaces are dashes too, like Claude Code's own folder names.
    assert_eq!(super::exec::project_slug(std::path::Path::new(r"C:\Users\x\.claude2 notes")), "C--Users-x--claude2-notes");
}

/// session-status's lookup from a real folder, canonicalized the way
/// runbooks::allowed_folder hands it over (`\\?\C:\…` on Windows), against
/// project folders named the way Claude Code names them (from the plain
/// path): found in either config folder, the newest one wins, and a folder
/// name in another case still counts. Empty transcripts: no content.
#[test]
fn session_status_finds_the_transcript_folder_of_a_canonicalized_repo() {
    let root = std::env::temp_dir().join(format!("glim-slug-test-{}-{}", std::process::id(), now_s()));
    let repo = root.join("local-companion");
    std::fs::create_dir_all(&repo).unwrap();
    let canon = std::fs::canonicalize(&repo).unwrap();
    let plain = canon.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let name: String = plain.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let (one, two) = (root.join(".claude"), root.join(".claude2"));
    std::fs::create_dir_all(one.join("projects").join(&name)).unwrap();
    std::fs::create_dir_all(two.join("projects").join(name.to_ascii_lowercase())).unwrap();
    std::fs::write(one.join("projects").join(&name).join("older.jsonl"), b"").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(two.join("projects").join(name.to_ascii_lowercase()).join("newer.jsonl"), b"").unwrap();
    let found = super::exec::latest_transcript(&[one.clone(), two.clone()], &canon).map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string());
    assert_eq!(found.as_deref(), Some("newer.jsonl"), "slug {} from {}", super::exec::project_slug(&canon), canon.display());
    assert_eq!(super::exec::latest_transcript(&[one], &canon).map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).as_deref(), Some("older.jsonl"));
    let _ = std::fs::remove_dir_all(&root);
}

/// By hand only, against a real relay: the courier as the PC (fake commands,
/// the [9; 32] test key as the PC key) for RELAY_LIVE_SECS seconds, while
/// `node relay/courier.mjs send …` plays cloud Glim. Needs RELAY_LIVE_URL,
/// RELAY_LIVE_PC_TOKEN and RELAY_LIVE_SIGNER_PUB.
#[test]
#[ignore = "live relay; run by hand"]
fn live_relay_round_trip() {
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
    let url = crate::net::relay::endpoint(&var("RELAY_LIVE_URL")).unwrap();
    let token = var("RELAY_LIVE_PC_TOKEN");
    let secs: u64 = std::env::var("RELAY_LIVE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let pc = Arc::new(Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap());
    println!("PC result key: {}", hex(pc.public_key().as_ref()));
    let settings = RemoteSettings { on: true, consented: true, public_key: var("RELAY_LIVE_SIGNER_PUB"), relay: "https://live.example.workers.dev".into(), ..Default::default() };
    let gate = Arc::new(Mutex::new(Gate::default()));
    let until = std::time::Instant::now() + Duration::from_secs(secs);
    let l = Loop {
        settings: Arc::new(move || settings.clone()),
        link: Arc::new(move |_s: &RemoteSettings| Ok(Link { url: url.clone(), token: token.clone(), signer: pc.clone() })),
        accept: Arc::new(move |s: &RemoteSettings, client: &str, raw: &[u8], now: i64| gate.lock().unwrap().accept_from(s, client, raw, now)),
        exec: Arc::new(Fake { delay: Duration::ZERO, runs: Arc::default() }),
        timing: Timing::default(),
        backoff_max: Duration::from_secs(5),
        idle: Duration::from_millis(200),
        running: Arc::new(move || std::time::Instant::now() < until),
        ledger: Arc::default(),
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(courier::run(l));
    println!("final state: {}", courier::state());
}
