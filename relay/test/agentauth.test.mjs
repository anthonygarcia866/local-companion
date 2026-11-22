// Token-less agent clients (owner 2026-11-21): signed relay requests
// (src/agentauth.js) made by courier.mjs (signRequest), and the nonce book.

import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";

import { keyClients, signedCaller, parseHeader, preCheck, SKEW_S, NONCE_KEEP_S } from "../src/agentauth.js";
import { cloudClients, caller, Relay, MemoryStore } from "../src/core.js";
import { keygen, signRequest, fingerprint } from "../courier.mjs";

const NOW = 1_763_700_000;
const HOST = "task-relay.example.workers.dev";
const URL0 = `https://${HOST}`;
const hex = (s) => crypto.createHash("sha256").update(s).digest("hex");
const PC = hex("pc-token");

function setup() {
  const muse = keygen(), other = keygen();
  const env = { PC_TOKEN_SHA256: PC, CLOUD_CLIENTS: `account-1=${hex("t1")},muse=key:${muse.pub},bot=key:${other.pub}` };
  return { muse, other, env };
}
const req = (method, path, body = "") => ({ method, host: HOST, path, body });

test("agent entries: key clients parsed apart from token clients", () => {
  const { muse, other, env } = setup();
  assert.deepEqual([...keyClients(env)], [["muse", muse.pub], ["bot", other.pub]]);
  // Token parsing ignores key entries (and so did the relay before this).
  assert.deepEqual([...cloudClients(env).keys()], ["account-1"]);
  // A name used twice, a key two names share, the legacy name, bad entries: dropped.
  assert.deepEqual([...keyClients({ CLOUD_CLIENTS: `muse=key:${muse.pub},muse=${hex("x")}` }).keys()], []);
  assert.deepEqual([...keyClients({ CLOUD_CLIENTS: `a=key:${muse.pub},b=key:${muse.pub},c=key:${other.pub}` }).keys()], ["c"]);
  assert.deepEqual([...keyClients({ CLOUD_CLIENTS: `account-1=key:${muse.pub}` }).keys()], []);
  assert.deepEqual([...keyClients({ CLOUD_CLIENTS: `x=key:${muse.pub.toUpperCase()},y=key:abc,Z=key:${other.pub}` }).keys()], []);
  assert.deepEqual([...keyClients({}).keys()], []);
});

test("a signed request from the agent's key passes, as that client, with its nonce", async () => {
  const { muse, env } = setup();
  const body = '{"v":1}';
  const auth = signRequest(muse.seed, "muse", "POST", `${URL0}/v1/tasks`, body, NOW, "a".repeat(32));
  assert.deepEqual(await signedCaller(auth, req("POST", "/v1/tasks", body), env, NOW), { client: "muse", nonce: "a".repeat(32) });
  // Within the skew either way.
  assert.ok(await signedCaller(auth, req("POST", "/v1/tasks", body), env, NOW + SKEW_S));
  assert.ok(await signedCaller(auth, req("POST", "/v1/tasks", body), env, NOW - SKEW_S));
  // A GET with a query, signed with the query.
  const g = signRequest(muse.seed, "muse", "GET", `${URL0}/v1/results/abcdefabcdefabcdef?wait=25`, "", NOW);
  assert.ok(await signedCaller(g, req("GET", "/v1/results/abcdefabcdefabcdef?wait=25"), env, NOW));
});

test("anything changed after signing, another key, a stale time or no PC: refused", async () => {
  const { muse, other, env } = setup();
  const body = '{"v":1}';
  const auth = signRequest(muse.seed, "muse", "POST", `${URL0}/v1/tasks`, body, NOW);
  const no = async (a, r, e = env, t = NOW) => assert.equal(await signedCaller(a, r, e, t), null);
  await no(auth, req("POST", "/v1/tasks", '{"v":2}'), env); // body
  await no(auth, req("POST", "/v1/results", body), env); // path
  await no(auth, req("DELETE", "/v1/tasks", body), env); // method
  await no(auth, { ...req("POST", "/v1/tasks", body), host: "evil.example.workers.dev" }); // host
  await no(auth, req("POST", "/v1/tasks", body), env, NOW + SKEW_S + 1); // too late
  await no(auth, req("POST", "/v1/tasks", body), env, NOW - SKEW_S - 1); // from the future
  await no(auth, req("POST", "/v1/tasks", body), { ...env, PC_TOKEN_SHA256: "" }); // no PC set up
  // bot's key claiming to be muse, and muse's request relabelled as bot.
  await no(signRequest(other.seed, "muse", "POST", `${URL0}/v1/tasks`, body, NOW), req("POST", "/v1/tasks", body));
  await no(auth.replace("client=muse", "client=bot"), req("POST", "/v1/tasks", body));
  // A token client's name can't sign; an unknown name can't either.
  await no(signRequest(muse.seed, "account-1", "POST", `${URL0}/v1/tasks`, body, NOW), req("POST", "/v1/tasks", body));
  await no(signRequest(muse.seed, "nobody", "POST", `${URL0}/v1/tasks`, body, NOW), req("POST", "/v1/tasks", body));
  // Malformed headers.
  for (const h of ["", "Bearer x", auth + " ", auth.replace("GlimSig", "glimsig"), auth.replace(/sig=[0-9a-f]+/, "sig=00"), auth.replace(/ts=\d+/, "ts=-1")]) await no(h, req("POST", "/v1/tasks", body));
  assert.equal(parseHeader(auth).client, "muse");
});

test("a GlimSig header is never a bearer token, and a token never passes as a signature", async () => {
  const { muse, env } = setup();
  const auth = signRequest(muse.seed, "muse", "GET", `${URL0}/v1/status`, "", NOW);
  assert.equal(await caller(auth, env), null);
  assert.equal(await signedCaller(`Bearer ${"a".repeat(40)}`, req("GET", "/v1/status"), env, NOW), null);
});

test("the relay signature can't be a task signature (different first bytes)", () => {
  const { muse } = setup();
  const auth = signRequest(muse.seed, "muse", "GET", `${URL0}/v1/status`, "", NOW);
  assert.match(auth, /^GlimSig /);
  // What is signed starts with "glim-relay-v1\n": never JSON, which a task payload must be.
  assert.throws(() => JSON.parse("glim-relay-v1\nmuse"));
});

test("each nonce once per client within the window, then forgotten", () => {
  const r = new Relay(new MemoryStore(), () => []);
  const n = "b".repeat(32);
  assert.equal(r.useNonce("muse", n, NOW, NONCE_KEEP_S), true);
  assert.equal(r.useNonce("muse", n, NOW + 1, NONCE_KEEP_S), false, "replay");
  assert.equal(r.useNonce("bot", n, NOW + 1, NONCE_KEEP_S), true, "another client's own nonce");
  // Long after the window (the request's ts would fail the skew check anyway).
  assert.equal(r.useNonce("muse", n, NOW + NONCE_KEEP_S + 1, NONCE_KEEP_S), true);
  assert.ok(NONCE_KEEP_S > 2 * SKEW_S, "kept while any ts it carries could still pass");
});

test("keygen's fingerprint is Glim's (SHA-256, first 16 bytes, groups of 4)", () => {
  const pub = "00".repeat(32);
  const d = crypto.createHash("sha256").update(Buffer.alloc(32)).digest("hex").toUpperCase();
  assert.equal(fingerprint(pub), d.slice(0, 32).match(/..../g).join(" "));
});

test("courier.mjs as an agent client: every request signed, no token sent, the relay accepts it", async () => {
  const { muse, env } = setup();
  const { main } = await import("../courier.mjs");
  const seen = [];
  const fetchFn = async (url, init) => {
    seen.push({ url, init });
    const u = new URL(url);
    const ok = await signedCaller(init.headers.authorization, { method: init.method, host: u.host, path: u.pathname + u.search, body: init.body || "" }, env, Math.floor(Date.now() / 1000));
    return new Response(JSON.stringify({ pc: false, ok: Boolean(ok) }), { status: ok ? 200 : 401 });
  };
  const cenv = { RELAY_URL: URL0, RELAY_CLIENT: "muse", GLIM_TASK_SIGNING_KEY: muse.seed, RELAY_CLOUD_TOKEN: "x".repeat(40) };
  const log = console.log;
  console.log = () => {};
  try {
    assert.equal(await main(["status"], cenv, fetchFn), 0);
  } finally {
    console.log = log;
  }
  assert.match(seen[0].init.headers.authorization, /^GlimSig client=muse,/);
  assert.ok(!JSON.stringify(seen).includes("x".repeat(40)), "the token is never sent in agent mode");
  assert.ok(!JSON.stringify(seen).includes(muse.seed), "the private key never leaves");
});

test("keygen: owner-only file, prints only the public key and its fingerprint", async () => {
  const { main } = await import("../courier.mjs");
  const fs = await import("node:fs");
  const os = await import("node:os");
  const path = await import("node:path");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "glim-keygen-"));
  const file = path.join(dir, "k.txt");
  const out = [];
  const [log, err] = [console.log, console.error];
  console.log = (s) => out.push(String(s));
  console.error = () => {};
  try {
    assert.equal(await main(["keygen", file], { USERPROFILE: dir, HOME: dir }), 0);
  } finally {
    [console.log, console.error] = [log, err];
  }
  const seed = fs.readFileSync(file, "utf8").trim();
  assert.match(seed, /^[0-9a-f]{64}$/);
  assert.equal(out.length, 2);
  assert.match(out[0], /^public key: [0-9a-f]{64}$/);
  assert.match(out[1], /^fingerprint: ([0-9A-F]{4} ){7}[0-9A-F]{4}$/);
  assert.ok(!out.join("\n").includes(seed), "the private key is never printed");
  if (process.platform !== "win32") assert.equal(fs.statSync(file).mode & 0o777, 0o600);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("a relay with only agent clients still lets the PC in (review 2026-11-21)", async () => {
  const { muse } = setup();
  const env = { PC_TOKEN_SHA256: hex("pc-token"), CLOUD_CLIENTS: `muse=key:${muse.pub}` };
  assert.deepEqual(await caller("Bearer pc-token-pc-token-pc-token-pc-token", { ...env, PC_TOKEN_SHA256: hex("pc-token-pc-token-pc-token-pc-token") }), { role: "pc" });
  // Nobody at all: still nobody, the PC included.
  assert.equal(await caller("Bearer pc-token-pc-token-pc-token-pc-token", { PC_TOKEN_SHA256: hex("pc-token-pc-token-pc-token-pc-token"), CLOUD_CLIENTS: "none" }), null);
});

test("the cheap checks run before any body is read: header, PC, known agent, time", () => {
  const { muse, env } = setup();
  const ok = signRequest(muse.seed, "muse", "GET", `${URL0}/v1/status`, "", NOW);
  assert.equal(preCheck(ok, env, NOW), true);
  assert.equal(preCheck(ok, env, NOW + SKEW_S + 1), false);
  assert.equal(preCheck(ok.replace("client=muse", "client=nobody"), env, NOW), false);
  assert.equal(preCheck(ok, { ...env, PC_TOKEN_SHA256: "" }, NOW), false);
  assert.equal(preCheck("GlimSig nonsense", env, NOW), false);
  assert.ok(NONCE_KEEP_S >= 2 * SKEW_S + 30, "a margin for the two clocks");
});
