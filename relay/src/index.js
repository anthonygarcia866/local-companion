// The courier relay Worker (docs/remote-courier.md). One Durable Object holds
// the mailbox; core.js has the logic. Nothing is printed (a test checks) and Workers
// Logs/observability are off (wrangler.toml): nothing about a request is
// logged by this code.
//
// Routes (each needs its own bearer token; the Worker holds only hashes; or,
// for a token-less agent client, a request signed with its key: GlimSig,
// agentauth.js, owner 2026-11-21; never the PC's route):
//   GET  /v1/pc               PC token: WebSocket (hibernatable). Pending
//                             tasks arrive at once, new ones as they're
//                             queued, each stamped with the cloud client
//                             whose token queued it.
//   POST /v1/tasks            a cloud client's token: queue one signed task.
//   GET  /v1/results/<id>     that client's token: the result of one of ITS
//                             tasks (?wait=1-25 seconds to wait for it;
//                             ?with=task: {task, result}, the task envelope
//                             it answers too). Kept until acknowledged.
//   DELETE /v1/results/<id>   that client's token: acknowledge (delete) it.
//   GET  /v1/results          that client's token: its pending tasks and
//                             kept results, ids, states and times only.
//   GET  /v1/status           a cloud client's token: {pc: connected?,
//                             last_seen: Unix s of the PC's last ping,
//                             frame, connect or close (null = never),
//                             pending: n of its own tasks}.
//                             "connected" = a PC socket that pinged (or
//                             sent a frame) in the last 75 s; a silent one
//                             is closed then (core.js PC_SILENT_MS).
//
// Phone approvals (owner 2026-11-21, approvals.js; NO token, CORS only for
// the approval page's origin): the 32-hex slot id the PC made is the only
// key, and what it unlocks is sealed (requests) or single-use and verified
// by the PC (answers).
//   GET  /v1/approve/<id>     a sealed approval request.
//   POST /v1/approve/<id>     its one answer (stop, or approve + assertion).
//   POST /v1/pair/<id>        a pairing slot's one answer (sealed to the PC).

import { DurableObject } from "cloudflare:workers";
import { Relay, caller, CLIENT_NAME, LEGACY_CLIENT, MAX_TASK_BYTES, MAX_WAIT_S } from "./core.js";
import { Approvals, MAX_ANSWER_BYTES, phoneRateLimit, readCapped } from "./approvals.js";
import { NONCE_KEEP_S, SCHEME, preCheck, signedCaller } from "./agentauth.js";

/** The approval page (GitHub Pages, approve/ in the repo): the only origin
 * a browser may call the phone routes from. */
export const PAGE_ORIGIN = "https://anthonygarcia866.github.io";
/** /v1/approve/<id> and /v1/pair/<id>. */
const PHONE_ROUTE = /^\/v1\/(approve|pair)\/([0-9a-f]{32})$/;
/** Set by the Worker for a phone route; whatever a caller sent is dropped. */
const PHONE_HEADER = "x-relay-phone";

const cors = (headers = {}) => ({ ...headers, "access-control-allow-origin": PAGE_ORIGIN, "access-control-allow-methods": "GET, POST", "access-control-allow-headers": "content-type", "access-control-max-age": "600", vary: "origin" });
const phoneJson = (status, body) => new Response(JSON.stringify(body), { status, headers: cors({ "content-type": "application/json", "cache-control": "no-store" }) });

/** Set by the Worker (never by the caller) for the Durable Object. */
const CLIENT_HEADER = "x-relay-client";
/** A signed agent request's nonce, set by the Worker (never the caller): the
 * Durable Object accepts each (client, nonce) once. */
const NONCE_HEADER = "x-relay-nonce";

const json = (status, body) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", "cache-control": "no-store" } });

export default {
  async fetch(request, env) {
    const url0 = new URL(request.url);
    const phone = PHONE_ROUTE.exec(url0.pathname);
    if (phone) {
      // No token here, by design. Only the page's origin gets CORS, and
      // only GET (approve) / POST reach the mailbox.
      if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: cors() });
      const okMethod = request.method === "POST" || (request.method === "GET" && phone[1] === "approve");
      if (!okMethod) return phoneJson(405, { error: "method" });
      // Rate limits before the body is read or the mailbox is woken.
      const limited = await phoneRateLimit(request, env);
      if (limited) return phoneJson(limited.status, { error: limited.error });
      // The body is read here, capped, before the mailbox sees it (a
      // chunked or oversized POST never reaches the Durable Object).
      let body = null;
      if (request.method === "POST") {
        const r = await readCapped(request, MAX_ANSWER_BYTES);
        if (!r.ok) return phoneJson(r.status, { error: r.error });
        body = r.text;
      }
      const headers = new Headers(request.headers);
      for (const h of [CLIENT_HEADER, NONCE_HEADER, "authorization", "content-length", "transfer-encoding"]) headers.delete(h);
      headers.set(PHONE_HEADER, "1");
      const stub = env.RELAY.get(env.RELAY.idFromName("mailbox"));
      return stub.fetch(new Request(request.url, { method: request.method, headers, body }));
    }
    const url = new URL(request.url);
    const pcRoute = url.pathname === "/v1/pc";
    const authorization = request.headers.get("authorization") || "";
    // Token-less agent clients (owner 2026-11-21, agentauth.js): the request
    // is signed with the client's key. The body is read here, capped, so its
    // hash can be checked, and forwarded exactly as read.
    if (authorization.startsWith(`${SCHEME} `)) {
      if (pcRoute) return json(403, { error: "wrong-token" });
      if (!preCheck(authorization, env, Math.floor(Date.now() / 1000))) return json(401, { error: "unauthorized" });
      let body = "";
      if (request.method === "POST") {
        const r = await readCapped(request, MAX_TASK_BYTES);
        if (!r.ok) return json(r.status, { error: r.error });
        body = r.text;
      } else if (request.method !== "GET" && request.method !== "DELETE") {
        return json(405, { error: "method" });
      }
      const signed = await signedCaller(authorization, { method: request.method, host: url.host, path: url.pathname + url.search, body }, env, Math.floor(Date.now() / 1000));
      if (!signed) return json(401, { error: "unauthorized" });
      const headers = new Headers(request.headers);
      for (const h of [CLIENT_HEADER, NONCE_HEADER, PHONE_HEADER, "authorization", "content-length", "transfer-encoding"]) headers.delete(h);
      headers.set(CLIENT_HEADER, signed.client);
      headers.set(NONCE_HEADER, signed.nonce);
      const stub = env.RELAY.get(env.RELAY.idFromName("mailbox"));
      return stub.fetch(new Request(request.url, { method: request.method, headers, body: request.method === "POST" ? body : null }));
    }
    const who = await caller(authorization, env);
    if (!who) return json(401, { error: "unauthorized" });
    if (pcRoute !== (who.role === "pc")) return json(403, { error: "wrong-token" });
    const stub = env.RELAY.get(env.RELAY.idFromName("mailbox"));
    // Whatever the caller sent under these names is replaced or dropped.
    const forwarded = new Request(request);
    forwarded.headers.delete(CLIENT_HEADER);
    forwarded.headers.delete(NONCE_HEADER);
    forwarded.headers.delete(PHONE_HEADER);
    if (who.role === "cloud") forwarded.headers.set(CLIENT_HEADER, who.client);
    return stub.fetch(forwarded);
  },
};

class SqlStore {
  constructor(sql) {
    this.sql = sql;
    sql.exec("CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, exp INTEGER NOT NULL, body TEXT NOT NULL)");
    sql.exec("CREATE TABLE IF NOT EXISTS results (id TEXT PRIMARY KEY, exp INTEGER NOT NULL, body TEXT NOT NULL)");
    // last_seen (2026-11-19): one row, the PC's last sign of life (ms).
    sql.exec("CREATE TABLE IF NOT EXISTS seen (k INTEGER PRIMARY KEY CHECK (k = 1), ms INTEGER NOT NULL)");
    sql.exec("CREATE TABLE IF NOT EXISTS nonces (k TEXT PRIMARY KEY, exp INTEGER NOT NULL)");
    // Cloud clients (2026-11-15): rows from before belong to account-1.
    for (const table of ["tasks", "results"]) {
      const cols = sql.exec(`PRAGMA table_info(${table})`).toArray().map((c) => c.name);
      if (!cols.includes("client")) sql.exec(`ALTER TABLE ${table} ADD COLUMN client TEXT NOT NULL DEFAULT '${LEGACY_CLIENT}'`);
    }
    // Store-and-forward (2026-11-17): a result keeps the task envelope it
    // answers and when it arrived.
    const rcols = sql.exec("PRAGMA table_info(results)").toArray().map((c) => c.name);
    if (!rcols.includes("task")) sql.exec("ALTER TABLE results ADD COLUMN task TEXT NOT NULL DEFAULT ''");
    if (!rcols.includes("ready_at")) sql.exec("ALTER TABLE results ADD COLUMN ready_at INTEGER NOT NULL DEFAULT 0");
  }
  one(q, ...a) {
    const rows = this.sql.exec(q, ...a).toArray();
    return rows.length ? rows[0] : null;
  }
  hasTask(id) { return this.one("SELECT 1 AS x FROM tasks WHERE id = ?", id) !== null; }
  hasResult(id) { return this.one("SELECT 1 AS x FROM results WHERE id = ?", id) !== null; }
  countTasks(client) {
    return client === undefined ? this.one("SELECT COUNT(*) AS n FROM tasks").n : this.one("SELECT COUNT(*) AS n FROM tasks WHERE client = ?", client).n;
  }
  countResults(client) { return this.one("SELECT COUNT(*) AS n FROM results WHERE client = ?", client).n; }
  putTask(id, exp, body, client) { this.sql.exec("INSERT INTO tasks (id, exp, body, client) VALUES (?, ?, ?, ?)", id, exp, body, client); }
  task(id) { return this.one("SELECT exp, client FROM tasks WHERE id = ?", id); }
  taskBody(id) { const r = this.one("SELECT body FROM tasks WHERE id = ?", id); return r ? r.body : null; }
  deleteTask(id) { this.sql.exec("DELETE FROM tasks WHERE id = ?", id); }
  tasks() { return this.sql.exec("SELECT id, exp, body, client FROM tasks ORDER BY rowid").toArray(); }
  putResult(id, exp, body, client, task = "", readyAt = 0) {
    this.sql.exec("INSERT OR REPLACE INTO results (id, exp, body, client, task, ready_at) VALUES (?, ?, ?, ?, ?, ?)", id, exp, body, client, task, readyAt);
  }
  result(id, client) {
    const r = this.one("SELECT body, task FROM results WHERE id = ? AND client = ?", id, client);
    return r ? { body: r.body, task: r.task } : null;
  }
  deleteResult(id, client) {
    if (this.one("SELECT 1 AS x FROM results WHERE id = ? AND client = ?", id, client) === null) return false;
    this.sql.exec("DELETE FROM results WHERE id = ? AND client = ?", id, client);
    return true;
  }
  listFor(client) {
    const tasks = this.sql.exec("SELECT id, exp FROM tasks WHERE client = ? ORDER BY rowid", client).toArray().map((r) => ({ id: r.id, state: "pending", exp: r.exp, readyAt: null }));
    const results = this.sql.exec("SELECT id, exp, ready_at FROM results WHERE client = ? ORDER BY rowid", client).toArray().map((r) => ({ id: r.id, state: "ready", exp: r.exp, readyAt: r.ready_at }));
    return [...tasks, ...results];
  }
  deleteExpired(before) {
    this.sql.exec("DELETE FROM tasks WHERE exp < ?", before);
    this.sql.exec("DELETE FROM results WHERE exp < ?", before);
  }
  earliestExp() {
    const r = this.one("SELECT MIN(exp) AS e FROM (SELECT exp FROM tasks UNION ALL SELECT exp FROM results)");
    return r && r.e !== null ? r.e : null;
  }
  seenMs() { const r = this.one("SELECT ms FROM seen WHERE k = 1"); return r ? r.ms : null; }
  setSeenMs(ms) { this.sql.exec("INSERT OR REPLACE INTO seen (k, ms) VALUES (1, ?)", ms); }
  // Agent request nonces (2026-11-21): kept about two minutes.
  hasNonce(key) { return this.one("SELECT 1 AS x FROM nonces WHERE k = ?", key) !== null; }
  putNonce(key, exp) { this.sql.exec("INSERT INTO nonces (k, exp) VALUES (?, ?)", key, exp); }
  deleteNonces(now) { this.sql.exec("DELETE FROM nonces WHERE exp < ?", now); }
}

/** Approval and pairing slots (approvals.js's store). */
class SqlSlots {
  constructor(sql) {
    this.sql = sql;
    sql.exec("CREATE TABLE IF NOT EXISTS slots (id TEXT PRIMARY KEY, kind TEXT NOT NULL, exp INTEGER NOT NULL, body TEXT NOT NULL, answer TEXT)");
  }
  get(id) {
    const rows = this.sql.exec("SELECT id, kind, exp, body, answer FROM slots WHERE id = ?", id).toArray();
    return rows.length ? rows[0] : null;
  }
  put(r) { this.sql.exec("INSERT INTO slots (id, kind, exp, body, answer) VALUES (?, ?, ?, ?, NULL)", r.id, r.kind, r.exp, r.body); }
  setAnswer(id, answer) { this.sql.exec("UPDATE slots SET answer = ? WHERE id = ? AND answer IS NULL", answer, id); }
  remove(id) { this.sql.exec("DELETE FROM slots WHERE id = ?", id); }
  count() { return this.sql.exec("SELECT COUNT(*) AS n FROM slots").toArray()[0].n; }
  answered() { return this.sql.exec("SELECT id, kind, answer FROM slots WHERE answer IS NOT NULL ORDER BY rowid").toArray(); }
  expire(before) { this.sql.exec("DELETE FROM slots WHERE exp < ?", before); }
}

const now = () => Math.floor(Date.now() / 1000);

export class Mailbox extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Keep-alive pings are answered without waking the object (hibernatable
    // WebSockets' auto-response: no wake-up, no billed request); when the
    // object is awake anyway, the last auto-response's time says whether
    // the PC is still there.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('{"t":"ping"}', '{"t":"pong"}'));
    const lastSeenMs = (ws) => {
      let a = null;
      try { a = ws.deserializeAttachment(); } catch { /* none */ }
      const auto = ctx.getWebSocketAutoResponseTimestamp(ws);
      const t = Math.max(a && a.since ? a.since : 0, a && a.last ? a.last : 0, auto ? auto.getTime() : 0);
      return t > 0 ? t : null;
    };
    this.lastSeenMs = lastSeenMs;
    this.relay = new Relay(new SqlStore(ctx.storage.sql), () => ctx.getWebSockets("pc"), lastSeenMs);
    this.approvals = new Approvals(new SqlSlots(ctx.storage.sql), () => ctx.getWebSockets("pc"));
  }

  async schedule() {
    const at = this.relay.nextPurgeMs();
    if (at === null) return;
    const cur = await this.ctx.storage.getAlarm();
    if (cur === null || cur > at) await this.ctx.storage.setAlarm(at);
  }

  async alarm() {
    this.relay.purge(now());
    await this.schedule();
  }

  async fetch(request) {
    const url = new URL(request.url);
    const phone = request.headers.get(PHONE_HEADER) === "1" ? PHONE_ROUTE.exec(url.pathname) : null;
    if (phone) {
      const [, route, id] = phone;
      const kind = route === "approve" ? "approval" : "pair";
      if (request.method === "GET") {
        const r = this.approvals.read(kind, id, now());
        return phoneJson(r.status, r.body);
      }
      // Only the Worker reaches this (it read the body capped already and
      // forwarded it without a Content-Length): the stream cap again.
      const body = await readCapped(request, MAX_ANSWER_BYTES, true);
      if (!body.ok) return phoneJson(body.status, { error: body.error });
      const r = this.approvals.answer(kind, id, body.text, now());
      return phoneJson(r.status, r.body);
    }
    const named = request.headers.get(CLIENT_HEADER) || "";
    const client = CLIENT_NAME.test(named) ? named : null;
    if (url.pathname !== "/v1/pc" && client === null) return json(403, { error: "wrong-token" });
    // A signed agent request: each nonce once (the Worker checked the
    // signature and the time and set this header; a caller's own is dropped).
    const nonce = request.headers.get(NONCE_HEADER);
    if (nonce !== null) {
      if (client === null || url.pathname === "/v1/pc" || !/^[0-9a-f]{32}$/.test(nonce)) return json(401, { error: "unauthorized" });
      if (!this.relay.useNonce(client, nonce, now(), NONCE_KEEP_S)) return json(401, { error: "replay" });
    }
    if (url.pathname === "/v1/pc") {
      if (request.headers.get("upgrade") !== "websocket") return json(426, { error: "websocket-only" });
      // One PC: a new connection replaces the old one.
      for (const old of this.ctx.getWebSockets("pc")) {
        try { old.close(4000, "replaced"); } catch { /* already gone */ }
      }
      const [client, server] = Object.values(new WebSocketPair());
      this.ctx.acceptWebSocket(server, ["pc"]);
      server.serializeAttachment({ since: Date.now() });
      this.relay.noteSeen(Date.now());
      for (const f of this.relay.pendingFrames(now())) server.send(f);
      for (const f of this.approvals.pendingFrames(now())) server.send(f);
      return new Response(null, { status: 101, webSocket: client });
    }
    if (url.pathname === "/v1/tasks" && request.method === "POST") {
      const len = Number(request.headers.get("content-length") || "0");
      if (len > MAX_TASK_BYTES) return json(413, { error: "too-big" });
      const text = await request.text();
      const r = this.relay.queue(text, now(), client);
      await this.schedule();
      return json(r.status, r.body);
    }
    if (url.pathname === "/v1/results" && request.method === "GET") {
      const r = this.relay.list(now(), client);
      return json(r.status, r.body);
    }
    const m = /^\/v1\/results\/([^/]+)$/.exec(url.pathname);
    if (m && request.method === "DELETE") {
      const r = this.relay.ack(m[1], now(), client);
      return json(r.status, r.body);
    }
    if (m && request.method === "GET") {
      const wait = Math.min(MAX_WAIT_S, Math.max(0, Number(url.searchParams.get("wait") || "0") | 0));
      const withTask = url.searchParams.get("with") === "task";
      const until = Date.now() + wait * 1000;
      for (;;) {
        const r = this.relay.takeResult(m[1], now(), client, withTask);
        if (r.raw !== undefined) return new Response(r.raw, { status: 200, headers: { "content-type": "application/json", "cache-control": "no-store" } });
        if (r.status !== 202 || Date.now() >= until) return json(r.status, r.body);
        await new Promise((res) => setTimeout(res, 500));
      }
    }
    if (url.pathname === "/v1/status" && request.method === "GET") return json(200, this.relay.status(now(), client));
    return json(404, { error: "not-found" });
  }

  async webSocketMessage(ws, message) {
    try { ws.serializeAttachment({ ...(ws.deserializeAttachment() || {}), last: Date.now() }); } catch { /* closing */ }
    this.relay.noteSeen(Date.now());
    if (typeof message !== "string") return;
    let f = null;
    try { f = JSON.parse(message); } catch { /* not JSON: core ignores it too */ }
    if (f && typeof f.t === "string" && /^(approval|pair)(-void|-ack)?$/.test(f.t)) {
      this.approvals.fromPc(f, now());
      return;
    }
    this.relay.fromPc(message, now());
    await this.schedule();
  }

  async webSocketClose(ws, code) {
    // A clean close the PC sent (1000 / 1001 / none) is a sign of life now;
    // anything else (1006 found long after a dropped line, our own 4001
    // no-ping) only counts the socket's last ping (review 2026-11-19).
    try { this.relay.noteSeen([1000, 1001, 1005].includes(code) ? Date.now() : this.lastSeenMs(ws)); } catch { /* gone */ }
    try { ws.close(code === 1005 ? 1000 : code, "bye"); } catch { /* already closed */ }
  }

  async webSocketError(ws) {
    // An error: only the socket's last ping counts.
    try { this.relay.noteSeen(this.lastSeenMs(ws)); } catch { /* gone */ }
    try { ws.close(1011, "error"); } catch { /* already closed */ }
  }
}
