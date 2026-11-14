// node --test relay/test  (CI: Windows CI's "Relay tests").
import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Relay, MemoryStore, caller, sha256Hex, taskMeta, MAX_PENDING } from "../src/core.js";
import { keygen, signTask, verifyResult, privateKey, sha256Hex as hashOf } from "../courier.mjs";

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
  assert.equal(await caller(`Bearer ${pc}`, env), "pc");
  assert.equal(await caller(`Bearer ${cloud}`, env), "cloud");
  assert.equal(await caller(`Bearer ${"x".repeat(43)}`, env), null);
  assert.equal(await caller(pc, env), null);
  assert.equal(await caller(`Bearer ${pc}`, { PC_TOKEN_SHA256: env.PC_TOKEN_SHA256 }), null);
  assert.equal(await caller(`Bearer ${pc}`, { PC_TOKEN_SHA256: env.PC_TOKEN_SHA256, CLOUD_TOKEN_SHA256: env.PC_TOKEN_SHA256 }), null);
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
  const long = JSON.stringify({ v: 1, payload: JSON.stringify({ id: "x".repeat(20), iat: NOW, exp: NOW + 3600, cmd: "a", args: {} }), sig: "00" });
  assert.equal(taskMeta(long, NOW).error, "lifetime-too-long");
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
  assert.equal(relay.pendingFrames(NOW + 300 + 61).length, 0);
  assert.equal(relay.nextPurgeMs(), null);
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
  assert.equal(relay.takeResult(t.id, NOW).status, 404, "handed over once");
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
