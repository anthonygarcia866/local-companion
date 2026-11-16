// The courier relay's logic, free of Cloudflare APIs so node can test it
// (relay/test/core.test.mjs). index.js wires it to a Durable Object.
//
// What it is: a mailbox between cloud Glim (signs tasks) and the Glim app on
// the owner's PC (checks them, acts, signs results). The relay can't sign
// either way, and checks neither signature: the PC verifies tasks, cloud Glim
// verifies results (docs/remote-courier.md). It keeps an envelope only until
// it is answered or expires (10 minutes at most), never logs, and stores no
// token: only the SHA-256 of each, as Worker secrets.

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
 * Who is asking: "pc", "cloud" or null. Each side has its own token; the
 * Worker holds only their hashes (PC_TOKEN_SHA256, CLOUD_TOKEN_SHA256).
 * Missing hashes refuse everyone.
 */
export async function caller(authorization, env) {
  const m = /^Bearer ([A-Za-z0-9_-]{32,128})$/.exec(authorization || "");
  if (!m) return null;
  const pc = (env.PC_TOKEN_SHA256 || "").trim().toLowerCase();
  const cloud = (env.CLOUD_TOKEN_SHA256 || "").trim().toLowerCase();
  if (pc.length !== 64 || cloud.length !== 64 || pc === cloud) return null;
  const h = await sha256Hex(m[1]);
  if (sameHex(h, pc)) return "pc";
  if (sameHex(h, cloud)) return "cloud";
  return null;
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

  /** cloud Glim queues a task: delivered at once if the PC is connected. */
  queue(text, now) {
    this.purge(now);
    const meta = taskMeta(text, now);
    if (meta.error) return { status: 400, body: { error: meta.error } };
    if (this.store.hasTask(meta.id) || this.store.hasResult(meta.id)) return { status: 409, body: { error: "duplicate-id" } };
    if (this.store.countTasks() >= MAX_PENDING) return { status: 429, body: { error: "too-many-pending" } };
    this.store.putTask(meta.id, meta.exp, text);
    const delivered = this.deliver(meta.id, text, now);
    return { status: 202, body: { id: meta.id, delivered } };
  }

  deliver(id, text, now) {
    let n = 0;
    for (const ws of this.livePc(now)) {
      try {
        ws.send(JSON.stringify({ t: "task", env: text }));
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
    return this.store.tasks().map((t) => JSON.stringify({ t: "task", env: t.body }));
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
    const exp = this.store.taskExp(f.id);
    if (exp === null) return;
    if (!envelopeMeta(f.env, now, MAX_RESULT_BYTES)) return;
    this.store.deleteTask(f.id);
    this.store.putResult(f.id, exp, f.env);
  }

  /** cloud Glim fetches a result: handed over once, then deleted. */
  takeResult(id, now) {
    this.purge(now);
    if (!ID.test(id || "")) return { status: 400, body: { error: "bad-id" } };
    const r = this.store.takeResult(id);
    if (r !== null) return { status: 200, raw: r };
    return { status: this.store.hasTask(id) ? 202 : 404, body: { pending: this.store.hasTask(id) } };
  }

  status(now) {
    this.purge(now);
    return { pc: this.livePc(now).length > 0, pending: this.store.countTasks() };
  }

  /** When the alarm should next clear expired rows (ms), or null. */
  nextPurgeMs() {
    const e = this.store.earliestExp();
    return e === null ? null : (e + SKEW_S + 1) * 1000;
  }
}

/** A Map store for tests (and the shape index.js's SQLite store follows). */
export class MemoryStore {
  constructor() {
    this.t = new Map();
    this.r = new Map();
  }
  hasTask(id) { return this.t.has(id); }
  hasResult(id) { return this.r.has(id); }
  countTasks() { return this.t.size; }
  putTask(id, exp, body) { this.t.set(id, { id, exp, body }); }
  taskExp(id) { return this.t.has(id) ? this.t.get(id).exp : null; }
  deleteTask(id) { this.t.delete(id); }
  tasks() { return [...this.t.values()]; }
  putResult(id, exp, body) { this.r.set(id, { exp, body }); }
  takeResult(id) {
    const r = this.r.get(id);
    this.r.delete(id);
    return r ? r.body : null;
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
