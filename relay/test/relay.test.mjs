// node --test relay/test  (CI: Windows CI's "Relay tests").
import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Relay, MemoryStore, caller, cloudClients, sha256Hex, taskMeta, MAX_PENDING, PC_SILENT_MS, RESULT_KEEP_S, MAX_RESULTS_PER_CLIENT } from "../src/core.js";
import { keygen, signTask, verifyResult, privateKey, sha256Hex as hashOf, authHeaders, main, defaultKeyFile, keyFileProblem } from "../courier.mjs";
import os from "node:os";

const here = path.dirname(fileURLToPath(import.meta.url));
const NOW = 1_800_000_000;

function sockets() {
  const list = [];
  return { list, fn: () => list, add() { const s = { sent: [], send(t) { this.sent.push(t); } }; list.push(s); return s; } };
}

function signedResult(seed, id, at, taskText, extra = {}) {
  const payload = JSON.stringify({ v: 1, id, task: hashOf(taskText), at, ok: true, kind: "session-status", detail: "idle", error: "", ...extra });
  const sig = crypto.sign(null, Buffer.from(payload), privateKey(seed)).toString("hex");
  return JSON.stringify({ v: 1, payload, sig });
}

test("tokens: each side only with its own, nobody without both hashes", async () => {
  const pc = "p".repeat(43), cloud = "c".repeat(43);
  const env = { PC_TOKEN_SHA256: await sha256Hex(pc), CLOUD_TOKEN_SHA256: await sha256Hex(cloud) };
  assert.deepEqual(await caller(`Bearer ${pc}`, env), { role: "pc" });
  // The single cloud token from before clients is account-1.
  assert.deepEqual(await caller(`Bearer ${cloud}`, env), { role: "cloud", client: "account-1" });
  assert.equal(await caller(`Bearer ${"x".repeat(43)}`, env), null);
  assert.equal(await caller(pc, env), null);
  assert.equal(await caller(`Bearer ${pc}`, { PC_TOKEN_SHA256: env.PC_TOKEN_SHA256 }), null);
  assert.equal(await caller(`Bearer ${pc}`, { PC_TOKEN_SHA256: env.PC_TOKEN_SHA256, CLOUD_TOKEN_SHA256: env.PC_TOKEN_SHA256 }), null);
});

test("cloud clients: two named tokens, each revocable on its own; the old token stays account-1", async () => {
  const pc = "p".repeat(43), one = "1".repeat(43), two = "2".repeat(43);
  const [hp, h1, h2] = [await sha256Hex(pc), await sha256Hex(one), await sha256Hex(two)];
  // The list names the old single token "legacy": nothing to re-enter.
  const both = { PC_TOKEN_SHA256: hp, CLOUD_TOKEN_SHA256: h1, CLOUD_CLIENTS: `account-1=legacy,account-2=${h2}` };
  assert.deepEqual(await caller(`Bearer ${one}`, both), { role: "cloud", client: "account-1" });
  assert.deepEqual(await caller(`Bearer ${two}`, both), { role: "cloud", client: "account-2" });
  assert.deepEqual(await caller(`Bearer ${pc}`, both), { role: "pc" });
  // account-1 removed: its token stops, account-2's keeps working.
  const onlyTwo = { ...both, CLOUD_CLIENTS: `account-2=${h2}` };
  assert.equal(await caller(`Bearer ${one}`, onlyTwo), null);
  assert.deepEqual(await caller(`Bearer ${two}`, onlyTwo), { role: "cloud", client: "account-2" });
  // And the other way round.
  const onlyOne = { ...both, CLOUD_CLIENTS: "account-1=legacy" };
  assert.equal(await caller(`Bearer ${two}`, onlyOne), null);
  assert.deepEqual(await caller(`Bearer ${one}`, onlyOne), { role: "cloud", client: "account-1" });
  // "none": nobody, not even the old token.
  assert.equal(await caller(`Bearer ${one}`, { ...both, CLOUD_CLIENTS: "none" }), null);
  // Junk entries are skipped; a hash two names share, or the PC's, counts for neither.
  assert.deepEqual([...cloudClients({ ...both, CLOUD_CLIENTS: `Bad Name=${h2},x=nothex,account-2=${h2}=extra,ok=${h2}` }).keys()], ["ok"]);
  assert.deepEqual([...cloudClients({ ...both, CLOUD_CLIENTS: `a=${h2},b=${h2}` }).keys()], []);
  assert.deepEqual([...cloudClients({ ...both, CLOUD_CLIENTS: `a=${hp},b=${h2}` }).keys()], ["b"]);
});

test("two clients at once: each task is stamped with its client, each sees only its own results and count", () => {
  const k1 = keygen(), k2 = keygen(), pcKey = keygen();
  const s = sockets();
  const pc = s.add();
  const relay = new Relay(new MemoryStore(), s.fn);
  const a = signTask(k1.seed, "session-status", {}, NOW);
  const b = signTask(k2.seed, "session-status", {}, NOW);
  assert.equal(relay.queue(a.text, NOW, "account-1").body.delivered, true);
  assert.equal(relay.queue(b.text, NOW, "account-2").body.delivered, true);
  assert.deepEqual(pc.sent.map((f) => [JSON.parse(f).env, JSON.parse(f).client]), [[a.text, "account-1"], [b.text, "account-2"]]);
  assert.deepEqual(relay.pendingFrames(NOW).map((f) => JSON.parse(f).client), ["account-1", "account-2"]);
  assert.equal(relay.status(NOW, "account-1").pending, 1);
  assert.equal(relay.status(NOW, "account-2").pending, 1);
  // account-2 can't see or take account-1's task or result.
  assert.equal(relay.takeResult(a.id, NOW, "account-2").status, 404);
  relay.fromPc(JSON.stringify({ t: "result", id: a.id, env: signedResult(pcKey.seed, a.id, NOW, a.text) }), NOW);
  assert.equal(relay.takeResult(a.id, NOW, "account-2").status, 404);
  assert.equal(relay.takeResult(a.id, NOW, "account-1").status, 200);
  assert.equal(relay.status(NOW, "account-2").pending, 1);
  assert.equal(relay.takeResult(b.id, NOW, "account-2").status, 202);
});

test("a queued task goes to the connected PC at once, and to a later one on connect", () => {
  const k = keygen();
  const s = sockets();
  const relay = new Relay(new MemoryStore(), s.fn);
  const a = signTask(k.seed, "session-status", {}, NOW);
  assert.deepEqual(relay.queue(a.text, NOW), { status: 202, body: { id: a.id, delivered: false } });
  // The PC (re)connects, e.g. after sleep: the pending task is handed over.
  assert.deepEqual(relay.pendingFrames(NOW + 5).map((f) => JSON.parse(f).env), [a.text]);
  const pc = s.add();
  const b = signTask(k.seed, "session-status", {}, NOW);
  assert.equal(relay.queue(b.text, NOW).body.delivered, true);
  assert.equal(JSON.parse(pc.sent[0]).env, b.text);
});

test("expired, too long-lived, duplicate and malformed tasks are refused; at most 50 pending", () => {
  const k = keygen();
  const relay = new Relay(new MemoryStore(), () => []);
  const old = signTask(k.seed, "session-status", {}, NOW - 1000);
  assert.equal(relay.queue(old.text, NOW).body.error, "expired");
  const life = (s) => JSON.stringify({ v: 1, payload: JSON.stringify({ id: "x".repeat(20), iat: NOW, exp: NOW + s, cmd: "a", args: {} }), sig: "00" });
  assert.equal(taskMeta(life(72 * 3600), NOW).error, undefined, "72 h is allowed (owner 2026-11-17)");
  assert.equal(taskMeta(life(72 * 3600 + 1), NOW).error, "lifetime-too-long");
  assert.equal(relay.queue("not json", NOW).body.error, "bad-envelope");
  const t = signTask(k.seed, "session-status", {}, NOW);
  relay.queue(t.text, NOW);
  assert.equal(relay.queue(t.text, NOW).status, 409);
  for (let i = 1; i < MAX_PENDING; i++) relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW);
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW).status, 429);
});

test("tasks are forgotten after their expiry: a PC waking later gets nothing", () => {
  const k = keygen();
  const relay = new Relay(new MemoryStore(), () => []);
  relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW);
  assert.equal(relay.pendingFrames(NOW + 300 + 1).length, 0);
  assert.equal(relay.nextPurgeMs(), null);
});

test("a task expired at the relay is never handed over or taken, with no skew after its expiry", () => {
  const k = keygen();
  const relay = new Relay(new MemoryStore(), () => []);
  const t = signTask(k.seed, "session-status", {}, NOW);
  assert.equal(relay.queue(t.text, NOW + 300).status, 202, "still valid at its expiry second");
  assert.equal(relay.pendingFrames(NOW + 300).length, 1);
  assert.equal(relay.pendingFrames(NOW + 301).length, 0, "one second later it is gone");
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW + 301).body.error, "expired");
  // A result for a task the relay already forgot is dropped.
  relay.fromPc(JSON.stringify({ t: "result", id: t.id, env: signedResult(k.seed, t.id, NOW, t.text) }), NOW + 302);
  assert.equal(relay.takeResult(t.id, NOW + 302).status, 404);
});

test("results stay until their client acknowledges them (or 72 h), with the task they answer", () => {
  const a = keygen(), pcKey = keygen();
  const relay = new Relay(new MemoryStore(), () => []);
  const t = signTask(a.seed, "session-status", {}, NOW);
  relay.queue(t.text, NOW, "account-2");
  assert.deepEqual(relay.list(NOW, "account-2").body, { results: [{ id: t.id, state: "pending", exp: NOW + 300, readyAt: null }] });
  assert.deepEqual(relay.list(NOW, "account-1").body, { results: [] }, "another client sees nothing");
  relay.fromPc(JSON.stringify({ t: "result", id: t.id, env: signedResult(pcKey.seed, t.id, NOW + 2, t.text) }), NOW + 2);
  const listed = relay.list(NOW + 3, "account-2").body.results;
  assert.deepEqual(listed, [{ id: t.id, state: "ready", exp: NOW + 2 + RESULT_KEEP_S, readyAt: NOW + 2 }]);
  assert.deepEqual(Object.keys(listed[0]).sort(), ["exp", "id", "readyAt", "state"], "ids, states and times only");
  // Fetched twice: still there (a lost response loses nothing).
  assert.equal(relay.takeResult(t.id, NOW + 4, "account-2").status, 200);
  const both = JSON.parse(relay.takeResult(t.id, NOW + 4, "account-2", true).raw);
  assert.equal(both.task, t.text, "the exact task envelope it answers");
  assert.equal(verifyResult(pcKey.pub, both.result, t.id, both.task, NOW + 4).ok, true);
  // Only its own client may fetch or acknowledge it.
  assert.equal(relay.takeResult(t.id, NOW + 5, "account-1").status, 404);
  assert.equal(relay.ack(t.id, NOW + 5, "account-1").status, 404);
  assert.equal(relay.ack(t.id, NOW + 5, "account-2").status, 200);
  assert.equal(relay.takeResult(t.id, NOW + 6, "account-2").status, 404, "gone once acknowledged");
  assert.deepEqual(relay.list(NOW + 6, "account-2").body, { results: [] });
  assert.equal(relay.ack(t.id, NOW + 6, "account-2").status, 404);
  // Never acknowledged: purged 72 h after it arrived, long after the task's expiry.
  const u = signTask(a.seed, "session-status", {}, NOW);
  relay.queue(u.text, NOW, "account-2");
  relay.fromPc(JSON.stringify({ t: "result", id: u.id, env: signedResult(pcKey.seed, u.id, NOW + 10, u.text) }), NOW + 10);
  assert.equal(relay.takeResult(u.id, NOW + 10 + RESULT_KEEP_S, "account-2").status, 200);
  assert.equal(relay.takeResult(u.id, NOW + 11 + RESULT_KEEP_S, "account-2").status, 404);
  assert.equal(relay.nextPurgeMs(), null);
});

test("sealed (v2) envelopes: the relay reads only the id and times; results must name their task", () => {
  const relay = new Relay(new MemoryStore(), () => []);
  const id = "s".repeat(24);
  const sealed = (extra) => JSON.stringify({ v: 2, id, iat: NOW, exp: NOW + 24 * 3600, epk: "ab".repeat(32), n: "cd".repeat(12), ct: "ef".repeat(40), ...extra });
  assert.equal(relay.queue(sealed({ ct: "not hex" }), NOW).body.error, "bad-envelope");
  assert.equal(relay.queue(sealed({ epk: "ab" }), NOW).body.error, "bad-envelope");
  assert.equal(relay.queue(sealed({ exp: NOW + 73 * 3600 }), NOW).body.error, "lifetime-too-long");
  assert.equal(relay.queue(sealed(), NOW).status, 202);
  const result = (rid) => JSON.stringify({ v: 2, id: rid, epk: "12".repeat(32), n: "34".repeat(12), ct: "56".repeat(40) });
  relay.fromPc(JSON.stringify({ t: "result", id, env: result("o".repeat(24)) }), NOW + 1);
  assert.equal(relay.takeResult(id, NOW + 1).status, 202, "a result naming another id doesn't answer it");
  relay.fromPc(JSON.stringify({ t: "result", id, env: result(id) }), NOW + 1);
  assert.equal(relay.takeResult(id, NOW + 1).raw, result(id));
});

test("at most 200 unacknowledged results per client: its new tasks wait, others' don't", () => {
  const k = keygen();
  const store = new MemoryStore();
  for (let i = 0; i < MAX_RESULTS_PER_CLIENT; i++) store.putResult(`r${String(i).padStart(20, "0")}`, NOW + 1000, "{}", "account-2", "", NOW);
  const relay = new Relay(store, () => []);
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW, "account-2").body.error, "too-many-results");
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW, "account-1").status, 202);
});

test("a result answers its task once; unknown ids and junk are ignored", () => {
  const k = keygen(), pcKey = keygen();
  const relay = new Relay(new MemoryStore(), () => []);
  const t = signTask(k.seed, "session-status", {}, NOW);
  relay.queue(t.text, NOW);
  assert.equal(relay.takeResult(t.id, NOW).status, 202);
  relay.fromPc("garbage", NOW);
  relay.fromPc(JSON.stringify({ t: "result", id: "u".repeat(20), env: signedResult(pcKey.seed, "u".repeat(20), NOW, "x") }), NOW);
  assert.equal(relay.takeResult("u".repeat(20), NOW).status, 404);
  relay.fromPc(JSON.stringify({ t: "result", id: t.id, env: signedResult(pcKey.seed, t.id, NOW, t.text) }), NOW);
  assert.equal(relay.pendingFrames(NOW).length, 0, "answered = deleted");
  const got = relay.takeResult(t.id, NOW);
  assert.equal(got.status, 200);
  assert.equal(verifyResult(pcKey.pub, got.raw, t.id, t.text, NOW).detail, "idle");
  assert.equal(relay.takeResult(t.id, NOW).status, 200, "kept until acknowledged (owner 2026-11-17)");
  assert.equal(relay.ack(t.id, NOW).status, 200);
  assert.equal(relay.takeResult(t.id, NOW).status, 404, "acknowledged = gone");
});

test("cloud Glim refuses a result with a bad signature, the wrong key, another task's id or another envelope", () => {
  const pcKey = keygen(), other = keygen();
  const id = "a".repeat(24);
  const sent = "the envelope cloud Glim sent";
  const good = signedResult(pcKey.seed, id, NOW, sent);
  assert.equal(verifyResult(pcKey.pub, good, id, sent, NOW).ok, true);
  assert.throws(() => verifyResult(other.pub, good, id, sent, NOW), /bad-result-signature/);
  const env = JSON.parse(good);
  const tampered = JSON.stringify({ ...env, payload: env.payload.replace("idle", "done") });
  assert.throws(() => verifyResult(pcKey.pub, tampered, id, sent, NOW), /bad-result-signature/);
  assert.throws(() => verifyResult(pcKey.pub, good, "b".repeat(24), sent, NOW), /bad-result/);
  assert.throws(() => verifyResult(pcKey.pub, good, id, sent, NOW + 3600), /bad-result/);
  // A genuine PC refusal of a look-alike the relay made up with the same id doesn't answer ours.
  const lookalike = signedResult(pcKey.seed, id, NOW, "a forged envelope with our id");
  assert.throws(() => verifyResult(pcKey.pub, lookalike, id, sent, NOW), /bad-result/);
});

// RFC 8032's published test seeds (never real keys). The Rust tests
// (windows/src-tauri/src/remote/courier_tests.rs) check the same bytes: a task
// signed here verifies in Glim, a result Glim signs verifies here.
export const VECTOR = {
  cloudSeed: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
  pcPub: "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
  task: String.raw`{"v":1,"payload":"{\"id\":\"vector-task-0000001\",\"iat\":1763140000,\"exp\":1763140300,\"cmd\":\"session-status\",\"args\":{}}","sig":"3f3122e3715f013f73c6dd9498fbef70e0802e6cd6e2831c736ac8a70b4e343dd5f8fe0cf735f6513243e088f5910da37336ad2396465cc762e648760ceb0803"}`,
  result: String.raw`{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000001\",\"task\":\"f39855e5975f49aac158e7d2bb272fe82a8ce7d07cfc1d5bf9c3cd6239f9792e\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"74c8f2725a5e382a8409de4dd599b745ffe9ba8fc205cba7870e58411b4b9404e3e9ef73a1f37e5494b207d2fae9dc3644f7f3a317a971a0555c6403d5e25409"}`,
};

test("the shared vectors: this side signs what Glim checks and checks what Glim signs", () => {
  assert.equal(signTask(VECTOR.cloudSeed, "session-status", {}, 1763140000, "vector-task-0000001").text, VECTOR.task);
  const r = verifyResult(VECTOR.pcPub, VECTOR.result, "vector-task-0000001", VECTOR.task, 1763140010);
  assert.deepEqual([r.ok, r.kind, r.detail], [true, "session-status", "local-companion: idle"]);
});

test("the relay code never logs", () => {
  for (const f of ["../src/index.js", "../src/core.js"]) {
    const src = fs.readFileSync(path.join(here, f), "utf8");
    assert.doesNotMatch(src, /console\./, f);
  }
  const toml = fs.readFileSync(path.join(here, "../wrangler.toml"), "utf8");
  assert.match(toml, /\[observability\]\s*\nenabled = false/);
  assert.match(toml, /new_sqlite_classes/);
  assert.match(toml, /^name = "task-relay"$/m);
});

// cloud Glim's two ways to authenticate: a claude.ai/code network secret (the
// agent proxy adds the header; the session has no RELAY_CLOUD_TOKEN), or the
// env var as a fallback.
function stubRelay(pc) {
  const seen = [];
  let queued = null;
  const fetchFn = async (url, init) => {
    seen.push({ url, auth: (init.headers || {}).authorization });
    if (url.endsWith("/v1/tasks")) {
      queued = init.body;
      return new Response(JSON.stringify({ delivered: true }), { status: 202 });
    }
    if (url.includes("/v1/results/")) {
      const id = JSON.parse(JSON.parse(queued).payload).id;
      return new Response(signedResult(pc.seed, id, Math.floor(Date.now() / 1000), queued), { status: 200 });
    }
    return new Response(JSON.stringify({ pc: true, pending: 0 }), { status: 200 });
  };
  return { seen, fetchFn };
}

const BASE = { RELAY_URL: "https://task-relay.example-sub.workers.dev" };

test("no RELAY_CLOUD_TOKEN: requests go without an Authorization header, and send still works", async () => {
  assert.deepEqual(authHeaders({}), {});
  assert.deepEqual(authHeaders({ RELAY_CLOUD_TOKEN: "  " }), {});
  const signer = keygen();
  const pc = keygen();
  const { seen, fetchFn } = stubRelay(pc);
  const env = { ...BASE, GLIM_TASK_SIGNING_KEY: signer.seed, GLIM_PC_PUBLIC_KEY: pc.pub };
  assert.equal(await main(["send", "session-status", "--wait", "5"], env, fetchFn), 0);
  assert.equal(await main(["status"], env, fetchFn), 0);
  assert.ok(seen.length >= 3);
  for (const s of seen) assert.equal(s.auth, undefined, s.url);
});

test("RELAY_CLOUD_TOKEN set: every request carries it as a bearer token", async () => {
  const token = "c".repeat(64);
  assert.deepEqual(authHeaders({ RELAY_CLOUD_TOKEN: token }), { authorization: `Bearer ${token}` });
  const signer = keygen();
  const pc = keygen();
  const { seen, fetchFn } = stubRelay(pc);
  const env = { ...BASE, RELAY_CLOUD_TOKEN: token, GLIM_TASK_SIGNING_KEY: signer.seed, GLIM_PC_PUBLIC_KEY: pc.pub };
  assert.equal(await main(["send", "session-status", "--wait", "5"], env, fetchFn), 0);
  for (const s of seen) assert.equal(s.auth, `Bearer ${token}`, s.url);
});

test("setup errors exit 4 without a stack: bad RELAY_URL, missing keys", async () => {
  const never = async () => assert.fail("no request expected");
  assert.equal(await main(["status"], { RELAY_URL: "https://evil.example.com" }, never), 4);
  assert.equal(await main(["send", "session-status"], { ...BASE }, never), 4);
});

test("keygen: default file under LOCALAPPDATA/Glim, never a synced folder", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "courier-"));
  const env = { LOCALAPPDATA: tmp, USERPROFILE: String.raw`C:\Users\someone` };
  assert.equal(defaultKeyFile(env), path.join(tmp, "Glim", "task-signing-key.txt"));
  assert.equal(keyFileProblem(String.raw`C:\Users\someone\Desktop\k.txt`, env) !== null, true);
  assert.equal(keyFileProblem(String.raw`C:\Users\someone\Documents\x\k.txt`, env) !== null, true);
  assert.equal(keyFileProblem(String.raw`C:\Users\someone\OneDrive - Org\k.txt`, env) !== null, true);
  assert.equal(keyFileProblem(String.raw`D:\OneDrive\k.txt`, env) !== null, true);
  assert.equal(keyFileProblem(String.raw`C:\Users\someone\DesktopNotes\k.txt`, env), null);
  assert.equal(keyFileProblem(path.join(tmp, "Glim", "k.txt"), env), null);
  assert.equal(await main(["keygen"], env), 0);
  const seed = fs.readFileSync(defaultKeyFile(env), "utf8").trim();
  assert.match(seed, /^[0-9a-f]{64}$/);
  assert.equal(await main(["keygen"], env), 4, "refuses to overwrite");
  assert.equal(await main(["keygen", String.raw`C:\Users\someone\Desktop\k.txt`], env), 4);
  fs.rmSync(tmp, { recursive: true, force: true });
});

test("a PC socket silent for 75 s is closed, gets no task, and status says pc:false", () => {
  const k = keygen();
  const list = [];
  const mk = (lastSeen) => { const ws = { sent: [], closed: null, lastSeen, send(t) { this.sent.push(t); }, close(code, why) { this.closed = [code, why]; } }; list.push(ws); return ws; };
  const relay = new Relay(new MemoryStore(), () => list.filter((w) => !w.closed), (ws) => ws.lastSeen);
  const pc = mk(NOW * 1000);
  // Pinged 74 s ago: alive.
  assert.equal(relay.status(NOW + 74).pc, true);
  assert.equal(pc.closed, null);
  // 76 s with no ping (a half-open line): closed, not connected, and a new
  // task stays pending for the next connection instead of vanishing.
  const t = signTask(k.seed, "session-status", {}, NOW + 76);
  assert.deepEqual(relay.queue(t.text, NOW + 76).body, { id: t.id, delivered: false });
  assert.deepEqual(pc.closed, [4001, "no-ping"]);
  assert.equal(pc.sent.length, 0);
  assert.deepEqual(relay.status(NOW + 76), { pc: false, pending: 1 });
  // The PC reconnects: the pending task is handed over (once per connection).
  assert.deepEqual(relay.pendingFrames(NOW + 80).map((f) => JSON.parse(f).env), [t.text]);
  assert.equal(PC_SILENT_MS, 75_000);
  // Unknown last-seen (older wiring): treated as alive.
  const bare = new Relay(new MemoryStore(), () => [{ send() {} }]);
  assert.equal(bare.status(NOW).pc, true);
});

test("index.js: pings are auto-answered (no wake-up) and the socket's last ping decides liveness", () => {
  const src = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  assert.ok(src.includes("setWebSocketAutoResponse") && src.includes("getWebSocketAutoResponseTimestamp"));
  assert.ok(src.includes("serializeAttachment({ since: Date.now() })"));
});

test("index.js: the client name the Durable Object sees is the Worker's, never the caller's", () => {
  const src = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  assert.ok(src.includes("forwarded.headers.delete(CLIENT_HEADER)"));
  assert.ok(src.includes('if (who.role === "cloud") forwarded.headers.set(CLIENT_HEADER, who.client)'));
  assert.ok(src.includes("return stub.fetch(forwarded)"));
  assert.ok(src.includes("ADD COLUMN client"));
});

test("index.js: the SQLite store has every method the tested Map store has, and the new routes", () => {
  const src = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  const sql = src.slice(src.indexOf("class SqlStore"), src.indexOf("export class Mailbox"));
  for (const m of Object.getOwnPropertyNames(MemoryStore.prototype).filter((n) => n !== "constructor")) {
    assert.match(sql, new RegExp(`\\n  ${m}\\(`), `SqlStore.${m}`);
  }
  assert.ok(src.includes("ADD COLUMN task") && src.includes("ADD COLUMN ready_at"));
  assert.ok(src.includes('request.method === "DELETE"') && src.includes('url.pathname === "/v1/results" && request.method === "GET"'));
});
