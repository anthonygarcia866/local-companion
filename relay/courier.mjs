#!/usr/bin/env node
// cloud Glim's side of the remote courier (docs/remote-courier.md). Node 18+,
// no dependencies.
//
//   node relay/courier.mjs keygen [<file>]      new task-signing key: the
//        private seed goes to <file> (default %LOCALAPPDATA%\Glim\
//        task-signing-key.txt; refused if it exists or sits in a synced
//        folder: OneDrive, Desktop, Documents), the PUBLIC key is printed
//        (paste it in Glim: Settings → Remote → Remote tasks). Delete the
//        file once its contents are in cloud Glim's environment.
//   node relay/courier.mjs send <cmd> ['<args json>'] [--wait 60]
//        sign a task, queue it, wait for the PC's signed result, verify it,
//        print it. Exit 0 ok, 1 refused/failed, 2 bad result signature,
//        3 no result in time, 4 setup/usage error.
//   node relay/courier.mjs status               {pc, pending}
//
// Environment (cloud Glim's environment settings, never a repo or chat):
//   RELAY_URL              https://task-relay.<subdomain>.workers.dev
//   RELAY_CLOUD_TOKEN      OPTIONAL. Recommended instead: a claude.ai/code
//                          network secret for the relay's exact host, path
//                          /v1/, so the agent proxy adds "Authorization:
//                          Bearer <token>" and the session never holds it.
//                          Unset = requests go out without that header.
//   GLIM_TASK_SIGNING_KEY  the 64-hex private seed from keygen
//   GLIM_PC_PUBLIC_KEY     the PC's result key (64 hex), from Glim's Settings

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
export const LIFETIME_S = 300;
export const SKEW_S = 60;
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

/** A signed task envelope, as the relay carries it. */
export function signTask(seedHex, cmd, args, now = Math.floor(Date.now() / 1000), id = newId()) {
  const payload = JSON.stringify({ id, iat: now, exp: now + LIFETIME_S, cmd, args });
  const sig = crypto.sign(null, Buffer.from(payload, "utf8"), privateKey(seedHex)).toString("hex");
  return { id, text: JSON.stringify({ v: 1, payload, sig }) };
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
export function verifyResult(pcPublicHex, text, id, taskText, now = Math.floor(Date.now() / 1000)) {
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
  if (r.v !== 1 || r.id !== id || r.task !== sha256Hex(taskText) || typeof r.at !== "number" || Math.abs(now - r.at) > LIFETIME_S + SKEW_S) throw new Error("bad-result");
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

class SetupError extends Error {}

function need(env, name) {
  const v = (env[name] || "").trim();
  if (!v) throw new SetupError(`missing ${name} in the environment`);
  return v;
}

/** One relay request. `env`/`fetchFn` are for tests. */
export async function call(route, init = {}, env = process.env, fetchFn = fetch) {
  const base = (env.RELAY_URL || "").trim().replace(/\/+$/, "");
  if (!RELAY_URL_SHAPE.test(base)) throw new SetupError("RELAY_URL must be https://<name>.<subdomain>.workers.dev");
  const headers = { ...authHeaders(env), ...(init.headers || {}) };
  const r = await fetchFn(base + route, { ...init, headers });
  if (r.status === 401 && !headers.authorization) {
    console.error("relay: 401 and no token was sent from here: check the network secret (the relay's exact host, path /v1/), or set RELAY_CLOUD_TOKEN");
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
      console.log(`public key (paste in Glim): ${k.pub}`);
      console.log(`private seed written to ${file}: put it in cloud Glim's environment as GLIM_TASK_SIGNING_KEY, then DELETE the file.`);
      return 0;
    }
    if (what === "status") {
      const r = await call("/v1/status", {}, env, fetchFn);
      console.log(r.status, await r.text());
      return r.ok ? 0 : 1;
    }
    if (what === "send") {
      const waitAt = rest.indexOf("--wait");
      const wait = waitAt >= 0 ? Number(rest[waitAt + 1]) : 60;
      const pos = rest.filter((_, i) => waitAt < 0 || (i !== waitAt && i !== waitAt + 1));
      const [cmd, argsJson] = pos;
      if (!cmd) return console.error("usage: courier.mjs send <cmd> ['<args json>'] [--wait s]"), 4;
      const args = argsJson ? JSON.parse(argsJson) : {};
      const pcKey = need(env, "GLIM_PC_PUBLIC_KEY").toLowerCase();
      const task = signTask(need(env, "GLIM_TASK_SIGNING_KEY").toLowerCase(), cmd, args);
      const q = await call("/v1/tasks", { method: "POST", headers: { "content-type": "application/json" }, body: task.text }, env, fetchFn);
      if (q.status !== 202) return console.error("relay refused:", q.status, await q.text()), 1;
      const until = Date.now() + wait * 1000;
      while (Date.now() < until) {
        const left = Math.max(1, Math.min(25, Math.ceil((until - Date.now()) / 1000)));
        const r = await call(`/v1/results/${task.id}?wait=${left}`, {}, env, fetchFn);
        if (r.status === 200) {
          let res;
          try {
            res = verifyResult(pcKey, await r.text(), task.id, task.text);
          } catch (e) {
            console.error(e.message);
            return 2;
          }
          console.log(JSON.stringify(res, null, 2));
          return res.ok ? 0 : 1;
        }
        if (r.status !== 202) return console.error("relay:", r.status, await r.text()), 1;
      }
      console.error(`no result within ${wait} s (task ${task.id}; is the PC on and connected?)`);
      return 3;
    }
  } catch (e) {
    if (e instanceof SetupError) return console.error(e.message), 4;
    throw e;
  }
  console.error("usage: courier.mjs keygen [<file>] | send <cmd> ['<args json>'] [--wait s] | status");
  return 4;
}

if (process.argv[1]?.endsWith("courier.mjs")) {
  // exitCode, not exit(): exiting with fetch's sockets still open trips a
  // libuv assertion on Windows.
  main(process.argv.slice(2)).then((code) => { process.exitCode = code; }, (e) => {
    console.error(e.message);
    process.exitCode = 4;
  });
}
