#!/usr/bin/env node
// cloud Glim's side of the remote courier (docs/remote-courier.md). Node 18+,
// no dependencies.
//
//   node relay/courier.mjs keygen [<file>]      new task-signing key: the
//        private seed goes to <file> (default %LOCALAPPDATA%\Glim\
//        task-signing-key.txt; refused if it exists or sits in a synced
//        folder: OneDrive, Desktop, Documents), owner-only (0600). Only the
//        PUBLIC key and its fingerprint are printed (paste the key in
//        Glim: Settings → Remote → Remote tasks, and compare fingerprints).
//        Delete the file once its contents are in the agent's secret store.
//   node relay/courier.mjs send <cmd> ['<args json>'] [--wait 60] [--ttl 24h]
//        sign a task, seal it to the PC, queue it, wait for the PC's result,
//        open and verify it, print it, acknowledge it (the relay deletes
//        it). Exit 0 ok, 1 refused/failed, 2 bad result (signature, seal,
//        task), 3 no result in time (the id is printed: fetch it later with
//        `result`), 4 setup/usage error.
//   node relay/courier.mjs send --async <cmd> ['<args json>'] [--ttl 24h]
//        queue only; prints {"id","exp"}. Exit 0 queued, 1 refused, 4 setup.
//   node relay/courier.mjs result <id> [--wait 25] [--keep]
//        fetch, open and verify one of this client's results (from any
//        session: nothing is kept between sessions), print it, then
//        acknowledge it (--keep: don't). Exit 0/1/2 as send, 3 still
//        pending, 5 unknown or gone (expired or acknowledged).
//   node relay/courier.mjs results              this client's pending and
//        ready ids, states and times
//   node relay/courier.mjs status               {pc, pending}
//
// Store-and-forward (owner 2026-11-17, docs/courier-store-and-forward.md):
// a task lives 24 h by default (--ttl 30s-72h; start-session and
// send-to-session 24 h at most). The task is SEALED to the PC's encryption
// key (GLIM_PC_ENC_KEY) and names a per-task result key; the PC seals its
// signed result to that key, so the relay can read neither. Both per-task
// keys are derived from GLIM_TASK_SIGNING_KEY and the task id (HKDF-SHA256,
// distinct labels), never the signing key itself.
//
// Environment (cloud Glim's environment settings, never a repo or chat):
//   RELAY_URL              https://task-relay.<subdomain>.workers.dev
//   RELAY_CLIENT           token-less agent client (owner 2026-11-21): its
//                          name as Glim shows it ("muse"). Set = every relay
//                          request is signed with GLIM_TASK_SIGNING_KEY
//                          (GlimSig, relay/src/agentauth.js) and no token is
//                          sent; RELAY_CLOUD_TOKEN is ignored.
//   RELAY_CLOUD_TOKEN      OPTIONAL. Recommended instead: a claude.ai/code
//                          network secret for the relay's exact host, path
//                          /v1/, so the agent proxy adds "Authorization:
//                          Bearer <token>" and the session never holds it.
//                          Unset = requests go out without that header.
//   GLIM_TASK_SIGNING_KEY  the 64-hex private seed from keygen
//   GLIM_PC_PUBLIC_KEY     the PC's result key (64 hex), from Glim's Settings
//   GLIM_PC_ENC_KEY        the PC's encryption key (64 hex, public), from
//                          Glim's Settings → Remote. Unset: tasks go
//                          UNSEALED (the relay could read them), with a
//                          warning; results are sealed either way.
//   NODE_USE_ENV_PROXY=1   REQUIRED in a claude.ai/code cloud session: Node's
//                          built-in fetch ignores the agent proxy's
//                          HTTPS_PROXY without it, so requests skip the
//                          proxy and the network secret is never added
//                          (a 401, or no route to the relay at all).

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

class SetupError extends Error {}

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const X25519_PKCS8 = Buffer.from("302e020100300506032b656e04220420", "hex");
const X25519_SPKI = Buffer.from("302a300506032b656e032100", "hex");
/** The v1 vectors' lifetime (signTask's default; the CLI uses DEFAULT_TTL_S). */
export const LIFETIME_S = 300;
export const DEFAULT_TTL_S = 24 * 3600;
export const MAX_TTL_S = 72 * 3600;
/** start-session and send-to-session act into live work: 24 h at most. */
export const MAX_LIVE_TTL_S = 24 * 3600;
export const LIVE_COMMANDS = ["start-session", "send-to-session"];
export const SKEW_S = 60;
/** HKDF salt and the prefix of every label (domain separation). */
export const LABEL = "glim-courier-v2";
const HEX64 = /^[0-9a-f]{64}$/;
export const RELAY_URL_SHAPE = /^https:\/\/[a-z0-9-]+(\.[a-z0-9-]+)*\.workers\.dev$/;

export function privateKey(seedHex) {
  if (!HEX64.test(seedHex || "")) throw new Error("signing key must be 64 hex characters");
  return crypto.createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seedHex, "hex")]), format: "der", type: "pkcs8" });
}

export function publicKey(hex) {
  if (!HEX64.test(hex || "")) throw new Error("public key must be 64 hex characters");
  return crypto.createPublicKey({ key: Buffer.concat([SPKI_PREFIX, Buffer.from(hex, "hex")]), format: "der", type: "spki" });
}

export function publicHexOf(priv) {
  return crypto.createPublicKey(priv).export({ format: "der", type: "spki" }).subarray(SPKI_PREFIX.length).toString("hex");
}

export function keygen() {
  const { privateKey: priv } = crypto.generateKeyPairSync("ed25519");
  const seed = priv.export({ format: "der", type: "pkcs8" }).subarray(PKCS8_PREFIX.length).toString("hex");
  return { seed, pub: publicHexOf(priv) };
}

export function newId() {
  return "t" + crypto.randomBytes(15).toString("hex");
}

/** A signed task envelope (v1), as the relay carries it. `ttl` seconds;
 * `rk`: the result key's public half (hex) to name in the payload. */
export function signTask(seedHex, cmd, args, now = Math.floor(Date.now() / 1000), id = newId(), { ttl = LIFETIME_S, rk } = {}) {
  const body = { id, iat: now, exp: now + ttl, cmd, args };
  if (rk) body.rk = rk;
  const payload = JSON.stringify(body);
  const sig = crypto.sign(null, Buffer.from(payload, "utf8"), privateKey(seedHex)).toString("hex");
  return { id, iat: now, exp: now + ttl, text: JSON.stringify({ v: 1, payload, sig }) };
}

// ---- Sealing (owner 2026-11-17): X25519 + HKDF-SHA256 + ChaCha20-Poly1305.
// The same bytes as windows/src-tauri/src/remote/seal.rs (shared vectors).

/** HKDF-SHA256, 32 bytes. */
export function hkdf32(ikm, salt, info) {
  return Buffer.from(crypto.hkdfSync("sha256", ikm, Buffer.from(salt), Buffer.from(info, "utf8"), 32));
}

function x25519Private(raw) {
  return crypto.createPrivateKey({ key: Buffer.concat([X25519_PKCS8, raw]), format: "der", type: "pkcs8" });
}

function x25519Public(raw) {
  return crypto.createPublicKey({ key: Buffer.concat([X25519_SPKI, raw]), format: "der", type: "spki" });
}

/** The public half (32 bytes) of a raw X25519 private key. */
export function x25519PublicOf(raw) {
  return crypto.createPublicKey(x25519Private(raw)).export({ format: "der", type: "spki" }).subarray(X25519_SPKI.length);
}

function dh(rawPrivate, rawPublic) {
  let s;
  try {
    s = crypto.diffieHellman({ privateKey: x25519Private(rawPrivate), publicKey: x25519Public(rawPublic) });
  } catch {
    throw new Error("bad-seal");
  }
  if (s.every((b) => b === 0)) throw new Error("bad-seal");
  return s;
}

/** The per-task keys, derived from the signing seed and the task id with
 * their own labels (never the seed itself): the ephemeral key a task is
 * sealed to the PC with (so the sender can open its own task again later),
 * and the key the PC seals the result to. */
export function taskEphemeral(seedHex, id) {
  return hkdf32(Buffer.from(seedHex, "hex"), LABEL, `${LABEL} task-ephemeral|${id}`);
}

export function resultKey(seedHex, id) {
  return hkdf32(Buffer.from(seedHex, "hex"), LABEL, `${LABEL} result-key|${id}`);
}

const sealInfo = (what, id) => `${LABEL} ${what}|${id}`;

function aeadKey(shared, epk, recipient, what, id) {
  return hkdf32(shared, Buffer.concat([epk, recipient]), sealInfo(what, id));
}

/** Seals `plain` (a string) to `recipient` (32 bytes) with the ephemeral
 * private key `eph` and a 12-byte `nonce`: {epk, n, ct} in hex. */
export function sealTo(recipient, eph, nonce, what, id, plain) {
  const epk = x25519PublicOf(eph);
  const key = aeadKey(dh(eph, recipient), epk, recipient, what, id);
  const c = crypto.createCipheriv("chacha20-poly1305", key, nonce, { authTagLength: 16 });
  const pt = Buffer.from(plain, "utf8");
  c.setAAD(Buffer.from(sealInfo(what, id), "utf8"), { plaintextLength: pt.length });
  const ct = Buffer.concat([c.update(pt), c.final(), c.getAuthTag()]);
  return { epk: epk.toString("hex"), n: nonce.toString("hex"), ct: ct.toString("hex") };
}

/** Opens a seal with the shared secret already worked out (either side). */
function openWith(shared, epk, recipient, what, id, nonceHex, ctHex) {
  const ct = Buffer.from(ctHex, "hex");
  if (ct.length < 16 || !/^[0-9a-f]{24}$/.test(nonceHex || "")) throw new Error("bad-seal");
  const d = crypto.createDecipheriv("chacha20-poly1305", aeadKey(shared, epk, recipient, what, id), Buffer.from(nonceHex, "hex"), { authTagLength: 16 });
  d.setAAD(Buffer.from(sealInfo(what, id), "utf8"), { plaintextLength: ct.length - 16 });
  d.setAuthTag(ct.subarray(ct.length - 16));
  try {
    return Buffer.concat([d.update(ct.subarray(0, ct.length - 16)), d.final()]).toString("utf8");
  } catch {
    throw new Error("bad-seal");
  }
}

/** Opens a seal as its recipient (raw private key `me`); the PC's side,
 * for tests and tools. Throws "bad-seal". */
export function openSealed(me, epkHex, what, id, nonceHex, ctHex) {
  const epk = Buffer.from(epkHex || "", "hex");
  if (epk.length !== 32) throw new Error("bad-seal");
  return openWith(dh(me, epk), epk, x25519PublicOf(me), what, id, nonceHex, ctHex);
}

/** A task sealed to the PC: `{v:2, id, iat, exp, epk, n, ct}`, the signed
 * v1 envelope inside. The id and times outside are what the relay reads;
 * the PC checks they match the signed ones. */
export function sealTask(seedHex, pcEncHex, inner, id, iat, exp, nonce = crypto.randomBytes(12)) {
  const s = sealTo(Buffer.from(pcEncHex, "hex"), taskEphemeral(seedHex, id), nonce, "task", id, inner);
  return JSON.stringify({ v: 2, id, iat, exp, epk: s.epk, n: s.n, ct: s.ct });
}

/** The signed v1 envelope inside a task we sealed (we can: its ephemeral
 * key is derived from our seed). A v1 task is returned as is. */
export function openOwnTask(seedHex, pcEncHex, text, id) {
  let env;
  try {
    env = JSON.parse(text);
  } catch {
    throw new Error("bad-task");
  }
  if (env && env.v === 1) return text;
  if (!env || env.v !== 2 || env.id !== id || !HEX64.test(env.epk || "")) throw new Error("bad-task");
  if (!HEX64.test(pcEncHex || "")) throw new SetupError("missing GLIM_PC_ENC_KEY in the environment (needed to check a sealed task)");
  const eph = taskEphemeral(seedHex, id);
  const pc = Buffer.from(pcEncHex, "hex");
  if (x25519PublicOf(eph).toString("hex") !== env.epk) throw new Error("bad-task");
  try {
    return openWith(dh(eph, pc), Buffer.from(env.epk, "hex"), pc, "task", id, env.n, env.ct);
  } catch {
    throw new Error("bad-task");
  }
}

/** Our own signature on a task envelope (v1), for `id`: its payload. A task
 * the relay made up can't pass this (it can't sign). */
export function verifyOwnTask(seedHex, inner, id) {
  let env, p;
  try {
    env = JSON.parse(inner);
    if (!env || env.v !== 1 || typeof env.payload !== "string" || !/^[0-9a-f]{128}$/.test(env.sig || "")) throw new Error();
    if (!crypto.verify(null, Buffer.from(env.payload, "utf8"), publicKey(publicHexOf(privateKey(seedHex))), Buffer.from(env.sig, "hex"))) throw new Error();
    p = JSON.parse(env.payload);
  } catch {
    throw new Error("bad-task");
  }
  if (!p || p.id !== id || !Number.isInteger(p.iat) || !Number.isInteger(p.exp)) throw new Error("bad-task");
  return p;
}

/** The signed result inside a sealed one ({inner, sealed: true}), or a v1
 * result as is ({inner, sealed: false}). */
export function openResult(seedHex, text, id) {
  let env;
  try {
    env = JSON.parse(text);
  } catch {
    throw new Error("bad-result");
  }
  if (env && env.v === 1) return { inner: text, sealed: false };
  if (!env || env.v !== 2 || env.id !== id || !HEX64.test(env.epk || "")) throw new Error("bad-result");
  const rk = resultKey(seedHex, id);
  const epk = Buffer.from(env.epk, "hex");
  return { inner: openWith(dh(rk, epk), epk, x25519PublicOf(rk), "result", id, env.n, env.ct), sealed: true };
}

/**
 * Everything cloud Glim checks before it trusts a result, from the task
 * envelope it sent (or the relay kept) and the result: its own signature on
 * the task (opening it first if sealed), the seal on the result, then the
 * PC's signature, id, task hash and time (verifyResult). A task that named
 * a result key must get a sealed answer, unless it is a refusal (the PC
 * never seals to a key from a task it couldn't verify).
 */
export function verifyFull(env, id, taskText, resultText) {
  const seed = need(env, "GLIM_TASK_SIGNING_KEY").toLowerCase();
  const pcKey = need(env, "GLIM_PC_PUBLIC_KEY").toLowerCase();
  const pcEnc = (env.GLIM_PC_ENC_KEY || "").trim().toLowerCase();
  const p = verifyOwnTask(seed, openOwnTask(seed, pcEnc, taskText, id), id);
  const { inner, sealed } = openResult(seed, resultText, id);
  const r = verifyResult(pcKey, inner, id, taskText, Math.floor(Date.now() / 1000), { iat: p.iat, exp: p.exp });
  if (!sealed && p.rk && r.ok) throw new Error("bad-result");
  // The result format (2026-11-19): a result that names when its task was
  // made must name OUR task's signed iat, and its status must agree with ok
  // (an older Glim names neither).
  if (r.created !== undefined && r.created !== null && r.created !== p.iat) throw new Error("bad-result");
  if (r.status !== undefined && (r.status === "done") !== (r.ok === true)) throw new Error("bad-result");
  return r;
}

/** "90", "30s", "15m", "24h", "3d" → seconds; null when not one. */
export function parseTtl(s) {
  const m = /^(\d+)([smhd]?)$/.exec(String(s || "").trim());
  if (!m) return null;
  return Number(m[1]) * { "": 1, s: 1, m: 60, h: 3600, d: 86400 }[m[2]];
}

/** A task as the CLI sends it: signed, naming its result key, sealed to the
 * PC when GLIM_PC_ENC_KEY is set. */
export function buildTask(env, cmd, args, ttl, now = Math.floor(Date.now() / 1000), id = newId()) {
  const seed = need(env, "GLIM_TASK_SIGNING_KEY").toLowerCase();
  const rk = x25519PublicOf(resultKey(seed, id)).toString("hex");
  const t = signTask(seed, cmd, args, now, id, { ttl, rk });
  const pcEnc = (env.GLIM_PC_ENC_KEY || "").trim().toLowerCase();
  if (!pcEnc) {
    console.error("warning: GLIM_PC_ENC_KEY is not set, so this task goes UNSEALED (the relay could read it). Add it from Glim's Settings → Remote.");
    return t;
  }
  if (!HEX64.test(pcEnc)) throw new SetupError("GLIM_PC_ENC_KEY must be 64 hex characters");
  return { ...t, text: sealTask(seed, pcEnc, t.text, id, t.iat, t.exp) };
}

export function sha256Hex(text) {
  return crypto.createHash("sha256").update(text, "utf8").digest("hex");
}

/**
 * The PC's result for the task we sent (`id`, envelope `taskText`), verified
 * with the PC's public key: the signature, the id, the SHA-256 of the exact
 * envelope it answers (so a refusal of a look-alike the relay made up doesn't
 * count) and its time. Throws "bad-result-signature" or "bad-result".
 */
export function verifyResult(pcPublicHex, text, id, taskText, now = Math.floor(Date.now() / 1000), times = null) {
  let env;
  try {
    env = JSON.parse(text);
  } catch {
    throw new Error("bad-result");
  }
  if (!env || env.v !== 1 || typeof env.payload !== "string" || typeof env.sig !== "string" || !/^[0-9a-f]{128}$/.test(env.sig)) throw new Error("bad-result");
  const ok = crypto.verify(null, Buffer.from(env.payload, "utf8"), publicKey(pcPublicHex), Buffer.from(env.sig, "hex"));
  if (!ok) throw new Error("bad-result-signature");
  const r = JSON.parse(env.payload);
  if (r.v !== 1 || r.id !== id || r.task !== sha256Hex(taskText) || typeof r.at !== "number") throw new Error("bad-result");
  // Made while the task was alive (store-and-forward: it may be fetched
  // long after), and not in the future; without the task's times, within
  // the v1 lifetime of now.
  const fresh = times ? r.at >= times.iat - SKEW_S && r.at <= times.exp + SKEW_S && r.at <= now + SKEW_S : Math.abs(now - r.at) <= LIFETIME_S + SKEW_S;
  if (!fresh) throw new Error("bad-result");
  return r;
}

/**
 * The Authorization header for the relay: from RELAY_CLOUD_TOKEN when set,
 * else none (a claude.ai/code network secret adds it on the way out).
 */
export function authHeaders(env = process.env) {
  const t = (env.RELAY_CLOUD_TOKEN || "").trim();
  return t ? { authorization: `Bearer ${t}` } : {};
}

/** What Glim shows for a public key: SHA-256's first 16 bytes, "3F2A 91C0 …". */
export function fingerprint(pubHex) {
  const d = crypto.createHash("sha256").update(Buffer.from(pubHex, "hex")).digest();
  return [...Array(8).keys()].map((i) => d.subarray(2 * i, 2 * i + 2).toString("hex").toUpperCase()).join(" ");
}

/** The text a token-less agent client signs (relay/src/agentauth.js signedString). */
export function relaySignedString({ client, method, host, path: p, body, ts, nonce }) {
  const bodyHash = crypto.createHash("sha256").update(body || "", "utf8").digest("hex");
  return ["glim-relay-v1", client, method, host, p, bodyHash, String(ts), nonce].join("\n");
}

/** "GlimSig client=…,ts=…,nonce=…,sig=…" for one request (agent clients). */
export function signRequest(seedHex, client, method, url, body, now = Math.floor(Date.now() / 1000), nonce = crypto.randomBytes(16).toString("hex")) {
  if (!/^[a-z0-9][a-z0-9-]{0,31}$/.test(client || "")) throw new SetupError("RELAY_CLIENT must be the client's name as Glim shows it (a-z, 0-9, -)");
  const u = new URL(url);
  const text = relaySignedString({ client, method, host: u.host, path: u.pathname + u.search, body, ts: now, nonce });
  const sig = crypto.sign(null, Buffer.from(text, "utf8"), privateKey(seedHex)).toString("hex");
  return `GlimSig client=${client},ts=${now},nonce=${nonce},sig=${sig}`;
}

/** keygen's default file: %LOCALAPPDATA%\Glim\task-signing-key.txt (not synced). */
export function defaultKeyFile(env = process.env) {
  return env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, "Glim", "task-signing-key.txt") : null;
}

/** Why keygen must not write the seed to `file` (a synced folder), or null. */
export function keyFileProblem(file, env = process.env) {
  const slash = (p) => p.split("\\").join("/").toLowerCase().replace(/\/+$/, "");
  const norm = slash(path.resolve(file));
  if (norm.split("/").some((part) => part.startsWith("onedrive"))) return "that folder is synced by OneDrive";
  const home = slash(env.USERPROFILE || env.HOME || "");
  for (const d of ["desktop", "documents"]) {
    if (home && (norm === `${home}/${d}` || norm.startsWith(`${home}/${d}/`))) return `${d} may be synced by OneDrive`;
  }
  return null;
}


function need(env, name) {
  const v = (env[name] || "").trim();
  if (!v) throw new SetupError(`missing ${name} in the environment`);
  return v;
}

/** One relay request. `env`/`fetchFn` are for tests. */
export async function call(route, init = {}, env = process.env, fetchFn = fetch) {
  const base = (env.RELAY_URL || "").trim().replace(/\/+$/, "");
  if (!RELAY_URL_SHAPE.test(base)) throw new SetupError("RELAY_URL must be https://<name>.<subdomain>.workers.dev");
  const agent = (env.RELAY_CLIENT || "").trim();
  const method = (init.method || "GET").toUpperCase();
  const auth = agent ? { authorization: signRequest(need(env, "GLIM_TASK_SIGNING_KEY"), agent, method, base + route, typeof init.body === "string" ? init.body : "") } : authHeaders(env);
  const headers = { ...(init.headers || {}), ...auth };
  const r = await fetchFn(base + route, { ...init, method, headers });
  if (r.status === 401 && agent) {
    console.error(`relay: 401 for agent client ${agent}: check that Glim lists it as an agent client with this key (compare fingerprints), that the relay-tokens runbook ran since it was added, and this machine's clock`);
  } else if (r.status === 401 && !headers.authorization) {
    console.error("relay: 401 and no token was sent from here: check the network secret (the relay's exact host, path /v1/) and that NODE_USE_ENV_PROXY=1 is set, or set RELAY_CLOUD_TOKEN");
  }
  return r;
}

/** The CLI; returns its exit code. `env`/`fetchFn` are for tests. */
export async function main(argv, env = process.env, fetchFn = fetch) {
  const [what, ...rest] = argv;
  try {
    if (what === "keygen") {
      const file = rest[0] || defaultKeyFile(env);
      if (!file) return console.error("usage: courier.mjs keygen <file> (no LOCALAPPDATA for the default)"), 4;
      const problem = keyFileProblem(file, env);
      if (problem) return console.error(`refusing: ${problem}; use the default, %LOCALAPPDATA%\\Glim\\task-signing-key.txt`), 4;
      if (fs.existsSync(file)) return console.error(`refusing: ${file} exists`), 4;
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const k = keygen();
      fs.writeFileSync(file, k.seed + "\n", { mode: 0o600, flag: "wx" });
      // Owner-only even under an unusual umask (a no-op on Windows, where
      // %LOCALAPPDATA% is the user's own).
      fs.chmodSync(file, 0o600);
      console.log(`public key: ${k.pub}`);
      console.log(`fingerprint: ${fingerprint(k.pub)}`);
      console.error(`The private key is in ${file} (owner-only). Put its contents in the agent's secret store as GLIM_TASK_SIGNING_KEY, never in a chat, then delete the file.`);
      return 0;
    }
    if (what === "status") {
      const r = await call("/v1/status", {}, env, fetchFn);
      console.log(r.status, await r.text());
      return r.ok ? 0 : 1;
    }
    if (what === "results") {
      const r = await call("/v1/results", {}, env, fetchFn);
      console.log(r.status, await r.text());
      return r.ok ? 0 : 1;
    }
    if (what === "send" || what === "result") {
      const flags = parseFlags(rest, ["--wait", "--ttl"], ["--async", "--keep"]);
      if (flags.error) return console.error(flags.error), 4;
      need(env, "GLIM_TASK_SIGNING_KEY");
      need(env, "GLIM_PC_PUBLIC_KEY");
      if (what === "result") {
        const [id] = flags.pos;
        if (!/^[A-Za-z0-9_-]{16,64}$/.test(id || "")) return console.error("usage: courier.mjs result <task id> [--wait s] [--keep]"), 4;
        return await fetchResult(env, fetchFn, id, null, flags.num("--wait", 0), flags.has("--keep"));
      }
      const [cmd, argsJson] = flags.pos;
      if (!cmd) return console.error("usage: courier.mjs send [--async] <cmd> ['<args json>'] [--wait s] [--ttl 24h]"), 4;
      const args = argsJson ? JSON.parse(argsJson) : {};
      const ttl = flags.get("--ttl") === undefined ? DEFAULT_TTL_S : parseTtl(flags.get("--ttl"));
      const cap = LIVE_COMMANDS.includes(cmd) ? MAX_LIVE_TTL_S : MAX_TTL_S;
      if (ttl === null || ttl < 30 || ttl > cap) return console.error(`--ttl must be 30s to ${cap / 3600}h for ${cmd}`), 4;
      const task = buildTask(env, cmd, args, ttl);
      const q = await call("/v1/tasks", { method: "POST", headers: { "content-type": "application/json" }, body: task.text }, env, fetchFn);
      if (q.status !== 202) return console.error("relay refused:", q.status, await q.text()), 1;
      if (flags.has("--async")) {
        console.log(JSON.stringify({ id: task.id, exp: task.exp }));
        return 0;
      }
      const code = await fetchResult(env, fetchFn, task.id, task.text, flags.num("--wait", 60), false);
      if (code === 3) console.error(`no result within the wait (task ${task.id}, valid until ${new Date(task.exp * 1000).toISOString()}); later: node relay/courier.mjs result ${task.id}`);
      return code;
    }
  } catch (e) {
    if (e instanceof SetupError) return console.error(e.message), 4;
    throw e;
  }
  console.error("usage: courier.mjs keygen [<file>] | send [--async] <cmd> ['<args json>'] [--wait s] [--ttl 24h] | result <id> [--wait s] [--keep] | results | status");
  return 4;
}

/** Flags with a value (`withValue`) and without (`bare`); the rest are
 * positional. */
export function parseFlags(argv, withValue, bare) {
  const vals = new Map();
  const on = new Set();
  const pos = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (withValue.includes(a)) {
      if (i + 1 >= argv.length) return { error: `${a} needs a value` };
      vals.set(a, argv[++i]);
    } else if (bare.includes(a)) on.add(a);
    else if (a.startsWith("--")) return { error: `unknown flag ${a}` };
    else pos.push(a);
  }
  return {
    pos,
    get: (k) => vals.get(k),
    has: (k) => on.has(k),
    num: (k, d) => (vals.has(k) ? Math.max(0, Number(vals.get(k)) || 0) : d),
  };
}

/**
 * Waits up to `wait` s for the result of task `id`, checks everything
 * (verifyFull; `taskText`: the envelope we just sent, else the one the
 * relay kept for us), prints it and acknowledges it unless `keep`. Exit
 * codes as `send`'s; 5 = unknown or gone.
 */
async function fetchResult(env, fetchFn, id, taskText, wait, keep) {
  const until = Date.now() + wait * 1000;
  for (;;) {
    const left = Math.max(0, Math.min(25, Math.ceil((until - Date.now()) / 1000)));
    const r = await call(`/v1/results/${id}?wait=${left}&with=task`, {}, env, fetchFn);
    if (r.status === 200) {
      let res;
      try {
        const both = JSON.parse(await r.text());
        res = verifyFull(env, id, taskText || both.task, both.result);
      } catch (e) {
        if (e instanceof SetupError) throw e;
        console.error(e.message || "bad-result");
        return 2;
      }
      console.log(JSON.stringify(res, null, 2));
      if (!keep) {
        const a = await call(`/v1/results/${id}`, { method: "DELETE" }, env, fetchFn);
        if (a.status !== 200) console.error(`relay: acknowledging ${id} answered ${a.status}`);
      }
      return res.ok ? 0 : 1;
    }
    if (r.status === 404) return console.error(`no task ${id} at the relay (unknown, expired or already acknowledged)`), 5;
    if (r.status !== 202) return console.error("relay:", r.status, await r.text()), 1;
    if (Date.now() >= until) return 3;
  }
}

if (process.argv[1]?.endsWith("courier.mjs")) {
  // exitCode, not exit(): exiting with fetch's sockets still open trips a
  // libuv assertion on Windows.
  main(process.argv.slice(2)).then((code) => { process.exitCode = code; }, (e) => {
    console.error(e.message);
    process.exitCode = 4;
  });
}
