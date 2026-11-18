// node --test relay/test  (CI: Windows CI's "Relay tests").
import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Relay, MemoryStore, caller, cloudClients, sha256Hex, taskMeta, MAX_PENDING, PC_SILENT_MS, RESULT_KEEP_S, MAX_RESULTS_PER_CLIENT, MAX_PENDING_PER_CLIENT } from "../src/core.js";
import { keygen, signTask, verifyResult, privateKey, sha256Hex as hashOf, authHeaders, main, defaultKeyFile, keyFileProblem } from "../courier.mjs";
import { sealTo, sealTask, openSealed, openResult, openOwnTask, verifyFull, resultKey, taskEphemeral, x25519PublicOf, buildTask, parseTtl, LABEL } from "../courier.mjs";
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
  // One client's share is 25 (one leaked token can't lock the others out)...
  for (let i = 1; i < MAX_PENDING_PER_CLIENT; i++) assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW).status, 202);
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW).status, 429);
  // ...another client still has room, up to 50 in all.
  for (let i = 0; i < MAX_PENDING - MAX_PENDING_PER_CLIENT; i++) assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW, "account-2").status, 202);
  assert.equal(relay.queue(signTask(k.seed, "session-status", {}, NOW).text, NOW, "account-3").status, 429);
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
/**
 * What the Glim app does with a task, in node: opens it if sealed (with the
 * PC's encryption key `enc`), signs a result with `pc`, and seals it to the
 * task's result key when it names one (unless `unsealed`).
 */
function pcAnswer(pc, enc, taskText, { unsealed = false, extra = {} } = {}) {
  const outer = JSON.parse(taskText);
  const inner = outer.v === 2 ? openSealed(enc, outer.epk, "task", outer.id, outer.n, outer.ct) : taskText;
  const p = JSON.parse(JSON.parse(inner).payload);
  const signed = signedResult(pc.seed, p.id, Math.floor(Date.now() / 1000), taskText, extra);
  if (!p.rk || unsealed) return signed;
  const s = sealTo(Buffer.from(p.rk, "hex"), crypto.randomBytes(32), crypto.randomBytes(12), "result", p.id, signed);
  return JSON.stringify({ v: 2, id: p.id, ...s });
}

/** A stand-in relay keeping one task and its result until acknowledged. */
function stubRelay(pc, opts = {}) {
  const seen = [];
  const store = { task: null, result: null };
  const fetchFn = async (url, init = {}) => {
    seen.push({ url, auth: (init.headers || {}).authorization, method: init.method || "GET" });
    if (url.endsWith("/v1/tasks")) {
      store.task = init.body;
      store.sent = init.body;
      store.result = opts.pending ? null : pcAnswer(pc, opts.enc, init.body, opts);
      return new Response(JSON.stringify({ delivered: true }), { status: 202 });
    }
    if (/\/v1\/results\/[^/?]+/.test(url)) {
      if (init.method === "DELETE") {
        const had = store.result !== null;
        store.result = null;
        store.task = had ? null : store.task;
        return new Response(JSON.stringify({ deleted: had }), { status: had ? 200 : 404 });
      }
      if (store.result === null) return new Response(JSON.stringify({ pending: store.task !== null }), { status: store.task !== null ? 202 : 404 });
      return new Response(url.includes("with=task") ? JSON.stringify({ task: store.task, result: store.result }) : store.result, { status: 200 });
    }
    if (url.endsWith("/v1/results")) return new Response(JSON.stringify({ results: [] }), { status: 200 });
    return new Response(JSON.stringify({ pc: true, pending: 0 }), { status: 200 });
  };
  return { seen, fetchFn, store };
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

// ── Store-and-forward (owner 2026-11-17): sealing, async send, result, results ──

// RFC 7748's published X25519 test keys (never real ones): the PC's
// encryption key ("Bob") and the result seal's ephemeral key ("Alice"). The
// Rust tests (courier_tests.rs, SEALED_*) check the same bytes.
const SEAL = {
  pcEncSecret: "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
  pcEncPub: "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
  pcSeed: "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
  id: "vector-task-0000002",
  rk: "eb2187a344e922fe15692b2e64550738db75862a77eee972920926c21e6e672c",
  task: String.raw`{"v":2,"id":"vector-task-0000002","iat":1763140000,"exp":1763226400,"epk":"75eaa546af013b14eace88be3b1fe5478f2f9a072e675516fd76fe6face3530f","n":"000102030405060708090a0b","ct":"b8ec532257db42bb3bd002f8b83b5e6dc4671c92936df8cb1f8884577b71402b097a9461cd7c08a02c89df156d4e774458d51b1988b8e0adfdeb656a793483237d94861e8f3b30a1e19125af008f10dda88e1b20d741793f89e1e49724da5ab1fb5ccbcedf30dbdf22180f6dab96d566ee7b57f03a7520299e8a8244050a138c0637687bd3692ea6039911d6cf7f1641b6fdd6fa123b7c14f1e1b807790e0cdfe7c50fad3913dd0146b2e4102bbefe1ffd6b9679d0f61387727b7cfc76f4bed6c1a2d7c2e463d46174b72e0fe934c3b8600e20ecb5b3c33f12bb2cb2066ab224e22f909015608d80bbd890a1d55a5e017c025e1a19c640b4aa7a37e667464d10ad0cb34378ea2dbbabffefe860a3f29feedae4a5c5d02c26bf2dd9bee39b110877cf7ffb092da75b53bad828b190104ff26aabcf0e1ba36dae8c8bf2f7d7be116976b516c3fa5aa1ce54abb657657475b89307588e68d5442ef6667ae37930c34d3250e0688c"}`,
  innerResult: String.raw`{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000002\",\"task\":\"7a28565ea45aa4568f31a15310d56e02c18afb1227ddf8098634cf3a053b6d4a\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"5c5656aeedc84c9c20cab9a72ba70ed6311a359d0de7bbdc6fe1887ba8da8a5f7873d2e6f001b5030e26c53dff3c8ade27affec0570a800d49942f641f1ce102"}`,
  result: String.raw`{"v":2,"id":"vector-task-0000002","epk":"8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a","n":"a0a1a2a3a4a5a6a7a8a9aaab","ct":"fca0f22cb2c91cbd459d82a6e49defa9053992c58a55fdee2e3ec880f92f29f2561391e2d28dd8ea2d8e4fbcf0f5c880c9c924ab7344635d43826ca5cdd14e073875e85face52a6f014562d741fd9196419c62e1b714a25949e02fd40e5787ecdb3717a4290f5a4c92026ff2f4189cea5baa3d2c01c3839a0d771a5fb8337eabe1c0a45c92dcd56f653449c4fb208b33099a783cc7e3a85a4c2e1d8f5a62bf421617c568d22ceb5b6170f2e6ed5c14b56bee7166d61418cc95c56c65b388dec1f72ab2160d0be0b500e1818b9e90c650b38386228f2242307b8b44fd9a3bf69f4714ad4675d39b8045841c7781e0b3ff2c45d1b6c68520d1863cfb5900e7d872959d5453f98eaf129b6abf29872749f057d996f16c40fd55c26fb11a73d347ee049a8f4320141312bf2986dbb30e8dcc8671952d7ce53f97a20e77fee6cbe994a782007ee1af63f053bbbcc63a92313a6d02c2ab3d43abe77dcfc580eb508fe7dc47f5e8dadc586942f373ac213a40fb2ef9ad24a99b1ae5e612a39593a60dec3d7d538e343fc8dfd216df4504f7221b58"}`,
};

test("the seal vectors: this side seals what Glim opens and opens what Glim seals", () => {
  const seed = VECTOR.cloudSeed;
  assert.equal(x25519PublicOf(Buffer.from(SEAL.pcEncSecret, "hex")).toString("hex"), SEAL.pcEncPub);
  assert.equal(x25519PublicOf(resultKey(seed, SEAL.id)).toString("hex"), SEAL.rk);
  const inner = signTask(seed, "session-status", {}, 1763140000, SEAL.id, { ttl: 86400, rk: SEAL.rk }).text;
  assert.equal(sealTask(seed, SEAL.pcEncPub, inner, SEAL.id, 1763140000, 1763226400, Buffer.from("000102030405060708090a0b", "hex")), SEAL.task);
  assert.equal(openOwnTask(seed, SEAL.pcEncPub, SEAL.task, SEAL.id), inner, "the sender can open its own task again later");
  assert.deepEqual(openResult(seed, SEAL.result, SEAL.id), { inner: SEAL.innerResult, sealed: true });
  // The PC's result key is RFC 8032's second test key (VECTOR.pcPub).
  const env = { GLIM_TASK_SIGNING_KEY: seed, GLIM_PC_PUBLIC_KEY: VECTOR.pcPub, GLIM_PC_ENC_KEY: SEAL.pcEncPub };
  const realNow = Date.now;
  Date.now = () => 1763140010 * 1000;
  try {
    const r = verifyFull(env, SEAL.id, SEAL.task, SEAL.result);
    assert.deepEqual([r.ok, r.detail], [true, "local-companion: idle"]);
  } finally {
    Date.now = realNow;
  }
});

test("per-task keys: domain-separated labels, never the signing seed itself", () => {
  const k = keygen();
  const t = taskEphemeral(k.seed, "a".repeat(20)), r = resultKey(k.seed, "a".repeat(20));
  assert.notDeepEqual(t, r, "task and result keys differ");
  assert.notEqual(t.toString("hex"), k.seed);
  assert.notEqual(r.toString("hex"), k.seed);
  assert.notDeepEqual(resultKey(k.seed, "b".repeat(20)), r, "one per task id");
  assert.equal(LABEL, "glim-courier-v2");
  const src = fs.readFileSync(path.join(here, "../courier.mjs"), "utf8");
  assert.ok(src.includes("task-ephemeral|") && src.includes("result-key|") && src.includes("${LABEL} ${what}|"));
});

function sealedEnv() {
  const signer = keygen(), pc = keygen();
  const enc = crypto.randomBytes(32);
  const env = { ...BASE, GLIM_TASK_SIGNING_KEY: signer.seed, GLIM_PC_PUBLIC_KEY: pc.pub, GLIM_PC_ENC_KEY: x25519PublicOf(enc).toString("hex") };
  return { signer, pc, enc, env };
}

test("verifyFull refuses a tampered seal, an unsealed success, a task the relay made up, another id", () => {
  const { pc, enc, env } = sealedEnv();
  const t = buildTask(env, "session-status", {}, 3600);
  assert.equal(JSON.parse(t.text).v, 2, "sealed to the PC");
  assert.ok(!t.text.includes("session-status"), "the relay can't read the command");
  const good = pcAnswer(pc, enc, t.text);
  assert.equal(verifyFull(env, t.id, t.text, good).ok, true);
  const g = JSON.parse(good);
  const flipped = JSON.stringify({ ...g, ct: g.ct.slice(0, -1) + (g.ct.endsWith("0") ? "1" : "0") });
  assert.throws(() => verifyFull(env, t.id, t.text, flipped), /bad-seal/);
  // The relay can't downgrade: an unsealed success for a task that named a result key.
  assert.throws(() => verifyFull(env, t.id, t.text, pcAnswer(pc, enc, t.text, { unsealed: true })), /bad-result/);
  // A refusal (the PC never seals to a key from a task it couldn't verify) may come unsealed.
  const refusal = pcAnswer(pc, enc, t.text, { unsealed: true, extra: { ok: false, kind: "", detail: "", error: "bad-seal" } });
  assert.equal(verifyFull(env, t.id, t.text, refusal).error, "bad-seal");
  // A task the relay made up (signed by someone else) with our id: refused before the result counts.
  const other = sealedEnv();
  const forged = buildTask({ ...env, GLIM_TASK_SIGNING_KEY: other.signer.seed }, "session-status", {}, 3600, undefined, t.id);
  assert.throws(() => verifyFull(env, t.id, forged.text, pcAnswer(pc, enc, forged.text)), /bad-task/);
  // Another task's result.
  const u = buildTask(env, "session-status", {}, 3600);
  assert.throws(() => verifyFull(env, t.id, t.text, pcAnswer(pc, enc, u.text)), /bad-result|bad-seal/);
});

test("send --async, then result <id> from a later session: verified, acknowledged, then gone", async () => {
  const { pc, enc, env } = sealedEnv();
  const relay = stubRelay(pc, { enc });
  const out = [];
  const log = console.log;
  console.log = (s) => out.push(String(s));
  try {
    assert.equal(await main(["send", "--async", "session-status"], env, relay.fetchFn), 0);
    const { id, exp } = JSON.parse(out.pop());
    const sent = JSON.parse(relay.store.task);
    assert.equal(sent.id, id);
    assert.equal(exp - sent.iat, 24 * 3600, "24 h by default");
    assert.equal(relay.seen.some((s) => s.method === "DELETE"), false, "nothing acknowledged yet");
    // A new session: nothing but the environment and the id.
    assert.equal(await main(["result", id, "--keep"], env, relay.fetchFn), 0);
    assert.equal(relay.seen.some((s) => s.method === "DELETE"), false, "--keep keeps it");
    assert.equal(await main(["result", id], env, relay.fetchFn), 0);
    assert.equal(JSON.parse(out.pop()).ok, true);
    assert.equal(relay.seen.filter((s) => s.method === "DELETE").length, 1, "acknowledged once verified");
    assert.equal(await main(["result", id], env, relay.fetchFn), 5, "gone once acknowledged");
    assert.equal(await main(["results"], env, relay.fetchFn), 0);
  } finally {
    console.log = log;
  }
});

test("result: still pending is exit 3; a result that fails its checks is exit 2 and never acknowledged", async () => {
  const { pc, enc, env } = sealedEnv();
  const pending = stubRelay(pc, { enc, pending: true });
  const out = [];
  const log = console.log;
  console.log = (s) => out.push(String(s));
  try {
    assert.equal(await main(["send", "--async", "session-status"], env, pending.fetchFn), 0);
    const { id } = JSON.parse(out.pop());
    assert.equal(await main(["result", id], env, pending.fetchFn), 3);
    const bad = stubRelay(keygen(), { enc });
    assert.equal(await main(["send", "--async", "session-status"], env, bad.fetchFn), 0);
    const { id: id2 } = JSON.parse(out.pop());
    assert.equal(await main(["result", id2], env, bad.fetchFn), 2, "signed by another PC key");
    assert.equal(bad.seen.some((s) => s.method === "DELETE"), false);
  } finally {
    console.log = log;
  }
});

test("--ttl: 30 s to 72 h, start-session and send-to-session 24 h at most", async () => {
  assert.deepEqual(["90", "30s", "15m", "24h", "3d", "x", "-1"].map(parseTtl), [90, 30, 900, 86400, 259200, null, null]);
  const { pc, enc, env } = sealedEnv();
  const relay = stubRelay(pc, { enc });
  const never = async () => assert.fail("no request expected");
  assert.equal(await main(["send", "--async", "session-status", "--ttl", "73h"], env, never), 4);
  assert.equal(await main(["send", "--async", "session-status", "--ttl", "10s"], env, never), 4);
  assert.equal(await main(["send", "--async", "send-to-session", "{}", "--ttl", "25h"], env, never), 4);
  assert.equal(await main(["send", "--async", "start-session", "{}", "--ttl", "25h"], env, never), 4);
  const log = console.log;
  console.log = () => {};
  try {
    assert.equal(await main(["send", "--async", "session-status", "--ttl", "72h"], env, relay.fetchFn), 0);
    const t = JSON.parse(relay.store.task);
    assert.equal(t.exp - t.iat, 72 * 3600);
  } finally {
    console.log = log;
  }
  assert.equal(await main(["send", "--bogus", "session-status"], env, never), 4);
});

test("sync send seals the task, opens and verifies the result, then acknowledges it", async () => {
  const { pc, enc, env } = sealedEnv();
  const relay = stubRelay(pc, { enc });
  const log = console.log;
  console.log = () => {};
  try {
    assert.equal(await main(["send", "session-status", "--wait", "5"], env, relay.fetchFn), 0);
  } finally {
    console.log = log;
  }
  assert.equal(JSON.parse(relay.store.sent).v, 2, "sealed to the PC");
  assert.equal(relay.seen.filter((s) => s.method === "DELETE").length, 1, "acknowledged");
  assert.equal(relay.store.result, null, "and gone");
});
