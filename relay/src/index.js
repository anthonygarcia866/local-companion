// The courier relay Worker (docs/remote-courier.md). One Durable Object holds
// the mailbox; core.js has the logic. Nothing is printed (a test checks) and Workers
// Logs/observability are off (wrangler.toml): nothing about a request is
// logged by this code.
//
// Routes (each needs its own bearer token; the Worker holds only hashes):
//   GET  /v1/pc               PC token: WebSocket (hibernatable). Pending
//                             tasks arrive at once, new ones as they're
//                             queued, each stamped with the cloud client
//                             whose token queued it.
//   POST /v1/tasks            a cloud client's token: queue one signed task.
//   GET  /v1/results/<id>     that client's token: the signed result of one
//                             of ITS tasks, handed over once (?wait=1-25
//                             seconds to wait for it).
//   GET  /v1/status           a cloud client's token: {pc: connected?,
//                             pending: n of its own tasks}.
//                             "connected" = a PC socket that pinged (or
//                             sent a frame) in the last 75 s; a silent one
//                             is closed then (core.js PC_SILENT_MS).

import { DurableObject } from "cloudflare:workers";
import { Relay, caller, CLIENT_NAME, LEGACY_CLIENT, MAX_TASK_BYTES, MAX_WAIT_S } from "./core.js";

/** Set by the Worker (never by the caller) for the Durable Object. */
const CLIENT_HEADER = "x-relay-client";

const json = (status, body) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", "cache-control": "no-store" } });

export default {
  async fetch(request, env) {
    const who = await caller(request.headers.get("authorization"), env);
    if (!who) return json(401, { error: "unauthorized" });
    const url = new URL(request.url);
    const pcRoute = url.pathname === "/v1/pc";
    if (pcRoute !== (who.role === "pc")) return json(403, { error: "wrong-token" });
    const stub = env.RELAY.get(env.RELAY.idFromName("mailbox"));
    // Whatever the caller sent under this name is replaced.
    const forwarded = new Request(request);
    forwarded.headers.delete(CLIENT_HEADER);
    if (who.role === "cloud") forwarded.headers.set(CLIENT_HEADER, who.client);
    return stub.fetch(forwarded);
  },
};

class SqlStore {
  constructor(sql) {
    this.sql = sql;
    sql.exec("CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, exp INTEGER NOT NULL, body TEXT NOT NULL)");
    sql.exec("CREATE TABLE IF NOT EXISTS results (id TEXT PRIMARY KEY, exp INTEGER NOT NULL, body TEXT NOT NULL)");
    // Cloud clients (2026-11-15): rows from before belong to account-1.
    for (const table of ["tasks", "results"]) {
      const cols = sql.exec(`PRAGMA table_info(${table})`).toArray().map((c) => c.name);
      if (!cols.includes("client")) sql.exec(`ALTER TABLE ${table} ADD COLUMN client TEXT NOT NULL DEFAULT '${LEGACY_CLIENT}'`);
    }
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
  putTask(id, exp, body, client) { this.sql.exec("INSERT INTO tasks (id, exp, body, client) VALUES (?, ?, ?, ?)", id, exp, body, client); }
  task(id) { return this.one("SELECT exp, client FROM tasks WHERE id = ?", id); }
  deleteTask(id) { this.sql.exec("DELETE FROM tasks WHERE id = ?", id); }
  tasks() { return this.sql.exec("SELECT id, exp, body, client FROM tasks ORDER BY rowid").toArray(); }
  putResult(id, exp, body, client) { this.sql.exec("INSERT OR REPLACE INTO results (id, exp, body, client) VALUES (?, ?, ?, ?)", id, exp, body, client); }
  takeResult(id, client) {
    const r = this.one("SELECT body FROM results WHERE id = ? AND client = ?", id, client);
    if (r) this.sql.exec("DELETE FROM results WHERE id = ?", id);
    return r ? r.body : null;
  }
  deleteExpired(before) {
    this.sql.exec("DELETE FROM tasks WHERE exp < ?", before);
    this.sql.exec("DELETE FROM results WHERE exp < ?", before);
  }
  earliestExp() {
    const r = this.one("SELECT MIN(exp) AS e FROM (SELECT exp FROM tasks UNION ALL SELECT exp FROM results)");
    return r && r.e !== null ? r.e : null;
  }
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
    this.relay = new Relay(new SqlStore(ctx.storage.sql), () => ctx.getWebSockets("pc"), lastSeenMs);
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
    const named = request.headers.get(CLIENT_HEADER) || "";
    const client = CLIENT_NAME.test(named) ? named : null;
    if (url.pathname !== "/v1/pc" && client === null) return json(403, { error: "wrong-token" });
    if (url.pathname === "/v1/pc") {
      if (request.headers.get("upgrade") !== "websocket") return json(426, { error: "websocket-only" });
      // One PC: a new connection replaces the old one.
      for (const old of this.ctx.getWebSockets("pc")) {
        try { old.close(4000, "replaced"); } catch { /* already gone */ }
      }
      const [client, server] = Object.values(new WebSocketPair());
      this.ctx.acceptWebSocket(server, ["pc"]);
      server.serializeAttachment({ since: Date.now() });
      for (const f of this.relay.pendingFrames(now())) server.send(f);
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
    const m = /^\/v1\/results\/([^/]+)$/.exec(url.pathname);
    if (m && request.method === "GET") {
      const wait = Math.min(MAX_WAIT_S, Math.max(0, Number(url.searchParams.get("wait") || "0") | 0));
      const until = Date.now() + wait * 1000;
      for (;;) {
        const r = this.relay.takeResult(m[1], now(), client);
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
    if (typeof message !== "string") return;
    this.relay.fromPc(message, now());
    await this.schedule();
  }

  async webSocketClose(ws, code) {
    try { ws.close(code === 1005 ? 1000 : code, "bye"); } catch { /* already closed */ }
  }

  async webSocketError(ws) {
    try { ws.close(1011, "error"); } catch { /* already closed */ }
  }
}
