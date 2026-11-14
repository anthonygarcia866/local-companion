// node --test relay/test  (CI: Windows CI's "Relay tests").
import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Relay, MemoryStore, caller, sha256Hex, taskMeta, MAX_PENDING } from "../src/core.js";
import { keygen, signTask, verifyResult, privateKey } from "../courier.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const NOW = 1_800_000_000;

function sockets() {
  const list = [];
  return { list, fn: () => list, add() { const s = { sent: [], send(t) { this.sent.push(t); } }; list.push(s); return s; } };
}

function signedResult(seed, id, at, extra = {}) {
  const payload = JSON.stringify({ v: 1, id, at, ok: true, kind: "session-status", detail: "idle", error: "", ...extra });
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
  relay.fromPc(JSON.stringify({ t: "result", id: "u".repeat(20), env: signedResult(pcKey.seed, "u".repeat(20), NOW) }), NOW);
  assert.equal(relay.takeResult("u".repeat(20), NOW).status, 404);
  relay.fromPc(JSON.stringify({ t: "result", id: t.id, env: signedResult(pcKey.seed, t.id, NOW) }), NOW);
  assert.equal(relay.pendingFrames(NOW).length, 0, "answered = deleted");
  const got = relay.takeResult(t.id, NOW);
  assert.equal(got.status, 200);
  assert.equal(verifyResult(pcKey.pub, got.raw, t.id, NOW).detail, "idle");
  assert.equal(relay.takeResult(t.id, NOW).status, 404, "handed over once");
});

test("cloud Glim refuses a result with a bad signature, the wrong key or another task's id", () => {
  const pcKey = keygen(), other = keygen();
  const id = "a".repeat(24);
  const good = signedResult(pcKey.seed, id, NOW);
  assert.equal(verifyResult(pcKey.pub, good, id, NOW).ok, true);
  assert.throws(() => verifyResult(other.pub, good, id, NOW), /bad-result-signature/);
  const env = JSON.parse(good);
  const tampered = JSON.stringify({ ...env, payload: env.payload.replace("idle", "done") });
  assert.throws(() => verifyResult(pcKey.pub, tampered, id, NOW), /bad-result-signature/);
  assert.throws(() => verifyResult(pcKey.pub, good, "b".repeat(24), NOW), /bad-result/);
  assert.throws(() => verifyResult(pcKey.pub, good, id, NOW + 3600), /bad-result/);
});

// RFC 8032's published test seeds (never real keys). The Rust tests
// (windows/src-tauri/src/remote/courier_tests.rs) check the same bytes: a task
// signed here verifies in Glim, a result Glim signs verifies here.
export const VECTOR = {
  cloudSeed: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
  pcPub: "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
  task: String.raw`{"v":1,"payload":"{\"id\":\"vector-task-0000001\",\"iat\":1763140000,\"exp\":1763140300,\"cmd\":\"session-status\",\"args\":{}}","sig":"3f3122e3715f013f73c6dd9498fbef70e0802e6cd6e2831c736ac8a70b4e343dd5f8fe0cf735f6513243e088f5910da37336ad2396465cc762e648760ceb0803"}`,
  result: String.raw`{"v":1,"payload":"{\"v\":1,\"id\":\"vector-task-0000001\",\"at\":1763140003,\"ok\":true,\"kind\":\"session-status\",\"detail\":\"local-companion: idle\",\"error\":\"\"}","sig":"f8f6581ff78ddae585b53f6b1bbf709dd2475b4bf14b9013071daf85f86d1180b19b5c0cccbdf0adc256dc09d54e492ec8d9c871a40614620e3c8be7d1303a05"}`,
};

test("the shared vectors: this side signs what Glim checks and checks what Glim signs", () => {
  assert.equal(signTask(VECTOR.cloudSeed, "session-status", {}, 1763140000, "vector-task-0000001").text, VECTOR.task);
  const r = verifyResult(VECTOR.pcPub, VECTOR.result, "vector-task-0000001", 1763140010);
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
