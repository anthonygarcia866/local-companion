// The courier relay's logic, free of Cloudflare APIs so node can test it
// (relay/test/core.test.mjs). index.js wires it to a Durable Object.
//
// What it is: a mailbox between cloud Glim (signs tasks) and the Glim app on
// the owner's PC (checks them, acts, signs results). The relay can't sign
// either way, and checks neither signature: the PC verifies tasks, cloud Glim
// verifies results (docs/remote-courier.md). It keeps an envelope only until
// it is answered or expires (10 minutes at most), never logs, and stores no
// token: only the SHA-256 of each, as Worker secrets.
//
// Cloud clients (owner 2026-11-15): one PC token, and one cloud token per
// cloud client ("account-1", "account-2" ...: one per Claude account), each
// revocable on its own. The relay stamps the name of the client whose token
// queued a task on the frame it hands the PC; the PC checks the task was
// signed by THAT client's key. A client sees only its own results and
// pending count.

export const MAX_TASK_BYTES = 16 * 1024;
export const MAX_RESULT_BYTES = 32 * 1024;
export const MAX_LIFETIME_S = 600;
export const SKEW_S = 60;
export const MAX_PENDING = 50;
export const MAX_WAIT_S = 25;
/** A PC socket that hasn't pinged (or sent anything) for this long is dead:
 * a half-open line after sleep or a network change never sends a close
 * frame. Glim pings every 30 s (owner 2026-11-15). */
export const PC_SILENT_MS = 75_000;
const ID = /^[A-Za-z0-9_-]{16,64}$/;
const HEX64 = /^[0-9a-f]{64}$/;
/** A cloud client's name: lower-case letters, digits and dashes. */
export const CLIENT_NAME = /^[a-z0-9][a-z0-9-]{0,31}$/;
/** The name the single cloud token of before 2026-11-15 goes by. */
export const LEGACY_CLIENT = "account-1";
export const MAX_CLIENTS = 10;

const hex64 = (v) => {
  const h = (v || "").trim().toLowerCase();
  return HEX64.test(h) ? h : null;
};

/**
 * The cloud clients: name -> token hash. CLOUD_CLIENTS is the list
 * ("account-1=<sha256 hex>,account-2=<sha256 hex>"; "none" = nobody), synced
 * by the relay-tokens runbook from Glim's REMOTE_CLOUD_CLIENTS. An entry's
 * hash "legacy" means CLOUD_TOKEN_SHA256 (the single cloud token from before
 * clients, so that setup keeps working unchanged). Without CLOUD_CLIENTS at
 * all, CLOUD_TOKEN_SHA256 alone is "account-1". A hash two names share, or
 * the PC's own, counts for neither.
 */
export function cloudClients(env) {
  const legacy = hex64(env.CLOUD_TOKEN_SHA256);
  const list = new Map();
  if (env.CLOUD_CLIENTS === undefined || env.CLOUD_CLIENTS === null) {
    if (legacy) list.set(LEGACY_CLIENT, legacy);
  } else {
    for (const part of String(env.CLOUD_CLIENTS).split(",")) {
      const [name, value, extra] = part.trim().split("=");
      if (extra !== undefined || !CLIENT_NAME.test(name || "") || list.has(name) || list.size >= MAX_CLIENTS) continue;
      const h = value === "legacy" ? legacy : hex64(value);
      if (h) list.set(name, h);
    }
  }
  const pc = hex64(env.PC_TOKEN_SHA256);
  const counts = new Map();
  for (const h of list.values()) counts.set(h, (counts.get(h) || 0) + 1);
  for (const [name, h] of list) if (counts.get(h) > 1 || h === pc) list.delete(name);
  return list;
}

/** Lower-case hex SHA-256 of a string. */
export async function sha256Hex(text) {
  const d = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Equal strings, in time that doesn't depend on where they differ. */
export function sameHex(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length || a.length === 0) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/**
 * Who is asking: { role: "pc" }, { role: "cloud", client: "<name>" } or
 * null. The PC has one token, each cloud client its own; the Worker holds
 * only their hashes (PC_TOKEN_SHA256; CLOUD_CLIENTS / CLOUD_TOKEN_SHA256,
 * cloudClients). No PC hash, or no cloud client, refuses everyone.
 */
export async function caller(authorization, env) {
  const m = /^Bearer ([A-Za-z0-9_-]{32,128})$/.exec(authorization || "");
  if (!m) return null;
  const pc = hex64(env.PC_TOKEN_SHA256);
  const clients = cloudClients(env);
  if (!pc || clients.size === 0) return null;
  const h = await sha256Hex(m[1]);
  if (sameHex(h, pc)) return { role: "pc" };
  // Every entry is compared (no early exit on a match).
  let found = null;
  for (const [name, hash] of clients) if (sameHex(h, hash)) found = name;
  return found ? { role: "cloud", client: found } : null;
}

/**
 * The little the relay reads from a signed envelope: its id and expiry, so it
 * can deliver it once and forget it on time. Unverified (the PC verifies);
 * null when it isn't one.
 */
export function envelopeMeta(text, now, maxBytes) {
  if (typeof text !== "string" || new TextEncoder().encode(text).length > maxBytes) return null;
  let env, p;
  try {
    env = JSON.parse(text);
    if (!env || env.v !== 1 || typeof env.payload !== "string" || typeof env.sig !== "string") return null;
    p = JSON.parse(env.payload);
  } catch {
    return null;
  }
  if (!p || typeof p.id !== "string" || !ID.test(p.id)) return null;
  return { id: p.id, iat: p.iat, exp: p.exp };
}

/** A task cloud Glim may queue: well-formed, not expired, not too long-lived. */
export function taskMeta(text, now) {
  const m = envelopeMeta(text, now, MAX_TASK_BYTES);
  if (!m || !Number.isInteger(m.iat) || !Number.isInteger(m.exp)) return { error: "bad-envelope" };
  if (m.exp < m.iat || m.exp - m.iat > MAX_LIFETIME_S) return { error: "lifetime-too-long" };
  if (m.iat > now + SKEW_S) return { error: "not-yet-valid" };
  if (now > m.exp + SKEW_S) return { error: "expired" };
  return { id: m.id, exp: m.exp };
}

/**
 * The relay. `store` keeps tasks and results (Map-backed in tests, SQLite in
 * the Durable Object); `pcSockets()` lists the PC's open sockets, each with
 * `send(text)` and `close(code, reason)`; `lastSeenMs(ws)` says when a
 * socket last pinged, connected or sent a frame (ms; null = unknown).
 */
export class Relay {
  constructor(store, pcSockets, lastSeenMs = () => null) {
    this.store = store;
    this.pcSockets = pcSockets;
    this.lastSeenMs = lastSeenMs;
  }

  /** The PC's sockets that are alive; a silent one is closed (4001) here,
   * so it neither gets a task nor counts as connected. */
  livePc(now) {
    const live = [];
    for (const ws of this.pcSockets()) {
      const seen = this.lastSeenMs(ws);
      if (seen !== null && now * 1000 - seen > PC_SILENT_MS) {
        try { ws.close(4001, "no-ping"); } catch { /* already gone */ }
        continue;
      }
      live.push(ws);
    }
    return live;
  }

  /** Everything past its expiry (+ skew) goes. */
  purge(now) {
    this.store.deleteExpired(now - SKEW_S);
  }

  /** A cloud client queues a task: delivered at once if the PC is connected. */
  queue(text, now, client = LEGACY_CLIENT) {
    this.purge(now);
    const meta = taskMeta(text, now);
    if (meta.error) return { status: 400, body: { error: meta.error } };
    if (this.store.hasTask(meta.id) || this.store.hasResult(meta.id)) return { status: 409, body: { error: "duplicate-id" } };
    if (this.store.countTasks() >= MAX_PENDING) return { status: 429, body: { error: "too-many-pending" } };
    this.store.putTask(meta.id, meta.exp, text, client);
    const delivered = this.deliver(text, client, now);
    return { status: 202, body: { id: meta.id, delivered } };
  }

  deliver(text, client, now) {
    let n = 0;
    for (const ws of this.livePc(now)) {
      try {
        ws.send(taskFrame(text, client));
        n++;
      } catch {
        // A closing socket: the task waits for the next connection.
      }
    }
    return n > 0;
  }

  /** The PC connected: every task still pending, oldest first. */
  pendingFrames(now) {
    this.purge(now);
    return this.store.tasks().map((t) => taskFrame(t.body, t.client));
  }

  /**
   * A frame from the PC. A result answers (and removes) its task; it is kept
   * for cloud Glim until fetched or the task's expiry. Anything else is
   * ignored.
   */
  fromPc(frame, now) {
    let f;
    try {
      f = JSON.parse(frame);
    } catch {
      return;
    }
    if (!f || f.t !== "result" || typeof f.id !== "string" || !ID.test(f.id) || typeof f.env !== "string") return;
    const task = this.store.task(f.id);
    if (task === null) return;
    if (!envelopeMeta(f.env, now, MAX_RESULT_BYTES)) return;
    this.store.deleteTask(f.id);
    this.store.putResult(f.id, task.exp, f.env, task.client);
  }

  /** A cloud client fetches a result of its own: handed over once, then
   * deleted. Another client's task or result is "not found". */
  takeResult(id, now, client = LEGACY_CLIENT) {
    this.purge(now);
    if (!ID.test(id || "")) return { status: 400, body: { error: "bad-id" } };
    const r = this.store.takeResult(id, client);
    if (r !== null) return { status: 200, raw: r };
    const t = this.store.task(id);
    const mine = t !== null && t.client === client;
    return { status: mine ? 202 : 404, body: { pending: mine } };
  }

  /** The PC's connection, and how many of this client's tasks wait. */
  status(now, client = LEGACY_CLIENT) {
    this.purge(now);
    return { pc: this.livePc(now).length > 0, pending: this.store.countTasks(client) };
  }

  /** When the alarm should next clear expired rows (ms), or null. */
  nextPurgeMs() {
    const e = this.store.earliestExp();
    return e === null ? null : (e + SKEW_S + 1) * 1000;
  }
}

/** What the PC gets: the envelope and the client whose token queued it. */
export function taskFrame(text, client) {
  return JSON.stringify({ t: "task", env: text, client });
}

/** A Map store for tests (and the shape index.js's SQLite store follows). */
export class MemoryStore {
  constructor() {
    this.t = new Map();
    this.r = new Map();
  }
  hasTask(id) { return this.t.has(id); }
  hasResult(id) { return this.r.has(id); }
  /** All pending tasks, or one client's. */
  countTasks(client) { return client === undefined ? this.t.size : [...this.t.values()].filter((x) => x.client === client).length; }
  putTask(id, exp, body, client) { this.t.set(id, { id, exp, body, client }); }
  /** { exp, client } of a pending task, or null. */
  task(id) { const x = this.t.get(id); return x ? { exp: x.exp, client: x.client } : null; }
  deleteTask(id) { this.t.delete(id); }
  tasks() { return [...this.t.values()]; }
  putResult(id, exp, body, client) { this.r.set(id, { exp, body, client }); }
  /** Handed over only to the client that queued the task. */
  takeResult(id, client) {
    const r = this.r.get(id);
    if (!r || r.client !== client) return null;
    this.r.delete(id);
    return r.body;
  }
  deleteExpired(before) {
    for (const [k, v] of this.t) if (v.exp < before) this.t.delete(k);
    for (const [k, v] of this.r) if (v.exp < before) this.r.delete(k);
  }
  earliestExp() {
    const all = [...this.t.values(), ...this.r.values()].map((x) => x.exp);
    return all.length ? Math.min(...all) : null;
  }
}
