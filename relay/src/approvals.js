// Phone approvals (owner 2026-11-21, docs/remote-runbooks.md §8): the relay
// side. Free of Cloudflare APIs so node can test it (relay/test).
//
// Two kinds of slot, each named by a 32-hex id the PC made (128 random bits:
// the id is the only thing that lets the phone reach it):
//   approval  the PC posts a SEALED request (`body`: details encrypted to the
//             owner's phone key; the relay can't read them). The approval
//             page fetches it, and posts ONE answer: "stop", or "approve"
//             with a WebAuthn assertion the PC verifies (the relay can't
//             check or forge it: it doesn't hold the phone's key).
//   pair      the PC opens a pairing slot (10 min). The page posts ONE
//             answer, encrypted to a key only the PC's QR code carried.
// Answers go to the PC over its WebSocket and are kept, and sent again on
// every connect, until the PC acknowledges them. The PC can void a slot
// (kill switch, run ended). Nothing here is a cloud route: cloud tokens
// can't reach a slot, and the phone's routes can reach nothing else.

export const SLOT_ID = /^[0-9a-f]{32}$/;
export const MAX_SLOTS = 20;
/** A request's sealed body: details for up to 3 approvers. */
export const MAX_BODY_BYTES = 8 * 1024;
/** An answer: an assertion (authenticator data, client data, signature). */
export const MAX_ANSWER_BYTES = 8 * 1024;
export const MAX_APPROVAL_S = 24 * 3600;
export const MAX_PAIR_S = 10 * 60;

/** Phone routes carry no token, so anyone can call them: two Workers rate
 * limits sit in front of the mailbox (owner 2026-11-21). PHONE_IP_LIMIT is
 * keyed by the caller's IP (cf-connecting-ip, set by Cloudflare), and
 * PHONE_ALL_LIMIT by one fixed key for everyone. The limits themselves live
 * in wrangler.toml. Cloudflare's counters are approximate and per location:
 * this stops floods from waking the one Durable Object, not a careful
 * attacker spread worldwide (the slot id and the assertion still do the
 * guarding). Known tradeoff (owner asked for an overall cap): a flood from
 * many IPs can fill the overall bucket and delay a real approval for as long
 * as it lasts; an approval waits up to 24 h, so it is delayed, never lost or
 * forged. Fails CLOSED: a missing binding or a throwing limit() refuses
 * the request. Returns null to go on, else {status, error}. */
export async function phoneRateLimit(request, env) {
  try {
    if (!env.PHONE_IP_LIMIT || !env.PHONE_ALL_LIMIT) return { status: 503, error: "rate-limit-unavailable" };
    const one = await env.PHONE_IP_LIMIT.limit({ key: `ip:${ipKey(request.headers.get("cf-connecting-ip"))}` });
    if (!one || one.success !== true) return { status: 429, error: "rate-limited" };
    const all = await env.PHONE_ALL_LIMIT.limit({ key: "phone-all" });
    if (!all || all.success !== true) return { status: 429, error: "rate-limited" };
    return null;
  } catch {
    return { status: 503, error: "rate-limit-unavailable" };
  }
}

/** The per-IP limit's key: an IPv4 address as is, an IPv6 address by its
 * /64 (one subscriber usually holds a whole /64, so per-address keys could be
 * rotated around the limit). Anything unparsable shares one key. */
export function ipKey(ip) {
  if (typeof ip !== "string" || ip.length === 0 || ip.length > 64) return "unknown";
  if (/^[0-9.]+$/.test(ip)) return ip;
  if (!/^[0-9a-fA-F:]+$/.test(ip) || !ip.includes(":")) return "unknown";
  const halves = ip.toLowerCase().split("::");
  if (halves.length > 2) return "unknown";
  const head = halves[0] ? halves[0].split(":") : [];
  const tail = halves.length === 2 && halves[1] ? halves[1].split(":") : [];
  const missing = 8 - head.length - tail.length;
  if (missing < 0 || (halves.length === 1 && missing !== 0)) return "unknown";
  const groups = [...head, ...Array(halves.length === 2 ? missing : 0).fill("0"), ...tail];
  if (groups.length !== 8 || groups.some((g) => g.length === 0 || g.length > 4)) return "unknown";
  return groups.slice(0, 4).map((g) => g.replace(/^0+(?=.)/, "")).join(":") + "::/64";
}

/** Reads a phone POST body with a hard cap, before anything else sees it
 * (review 2026-11-21: Content-Length alone can't be trusted). A body needs a
 * Content-Length (no chunked / unknown length), at most `max` bytes, and the
 * stream is cut off the moment it goes past that. Returns {ok: true, text}
 * or {ok: false, status, error}. `inner`: the Durable Object's second
 * check of a body the Worker already read capped and forwarded without its
 * Content-Length; the stream cap still applies, the header isn't needed. */
export async function readCapped(request, max, inner = false) {
  const raw = request.headers.get("content-length");
  let declared = max;
  if (!inner) {
    if (request.headers.has("transfer-encoding") || raw === null) return { ok: false, status: 411, error: "length-required" };
    if (!/^[0-9]{1,9}$/.test(raw)) return { ok: false, status: 400, error: "bad-length" };
    declared = Number(raw);
    if (declared > max) return { ok: false, status: 413, error: "too-big" };
  }
  if (!request.body) return declared === 0 || inner ? { ok: true, text: "" } : { ok: false, status: 400, error: "bad-length" };
  const reader = request.body.getReader();
  const parts = [];
  let total = 0;
  for (;;) {
    let step;
    try { step = await reader.read(); } catch { return { ok: false, status: 400, error: "bad-body" }; }
    if (step.done) break;
    total += step.value.byteLength;
    if (total > declared || total > max) {
      try { await reader.cancel(); } catch { /* gone */ }
      return { ok: false, status: 413, error: "too-big" };
    }
    parts.push(step.value);
  }
  if (!inner && total !== declared) return { ok: false, status: 400, error: "bad-length" };
  const buf = new Uint8Array(total);
  let at = 0;
  for (const p of parts) { buf.set(p, at); at += p.byteLength; }
  let text;
  try { text = new TextDecoder("utf-8", { fatal: true }).decode(buf); } catch { return { ok: false, status: 400, error: "bad-body" }; }
  return { ok: true, text };
}

const KINDS = ["approval", "pair"];

/** The frame that hands an answer to the PC. */
export function answerFrame(kind, id, answer) {
  return JSON.stringify({ t: `${kind}-answer`, id, answer });
}

export class Approvals {
  /** `store`: { get(id), put(row), setAnswer(id, answer), remove(id), count(), answered(), expire(before) }. */
  constructor(store, livePc) {
    this.store = store;
    this.livePc = livePc;
  }

  /** A frame from the PC: open a slot, void one, or acknowledge an answer. */
  fromPc(f, now) {
    if (!f || typeof f.id !== "string" || !SLOT_ID.test(f.id)) return;
    this.store.expire(now);
    if (f.t === "approval-void" || f.t === "approval-ack" || f.t === "pair-ack" || f.t === "pair-void") {
      this.store.remove(f.id);
      return;
    }
    const kind = f.t === "approval" ? "approval" : f.t === "pair" ? "pair" : null;
    if (kind === null || typeof f.exp !== "number" || !Number.isInteger(f.exp)) return;
    const max = kind === "approval" ? MAX_APPROVAL_S : MAX_PAIR_S;
    if (f.exp <= now || f.exp > now + max + 60) return;
    const body = kind === "approval" ? f.body : "";
    if (typeof body !== "string" || body.length > MAX_BODY_BYTES) return;
    if (this.store.get(f.id) !== null) return;
    if (this.store.count() >= MAX_SLOTS) return;
    this.store.put({ id: f.id, kind, exp: f.exp, body, answer: null });
  }

  /** The approval page reads a request (a pairing slot has no contents). */
  read(kind, id, now) {
    if (!KINDS.includes(kind) || !SLOT_ID.test(id || "")) return { status: 400, body: { error: "bad-id" } };
    this.store.expire(now);
    const row = this.store.get(id);
    if (row === null || row.kind !== kind) return { status: 404, body: { error: "gone" } };
    if (row.answer !== null) return { status: 409, body: { error: "answered" } };
    return { status: 200, body: { kind, exp: row.exp, body: row.body } };
  }

  /** The page's one answer. Later ones are refused (single use). */
  answer(kind, id, text, now) {
    if (!KINDS.includes(kind) || !SLOT_ID.test(id || "")) return { status: 400, body: { error: "bad-id" } };
    if (typeof text !== "string" || text.length === 0 || text.length > MAX_ANSWER_BYTES) return { status: 413, body: { error: "too-big" } };
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch {
      return { status: 400, body: { error: "bad-answer" } };
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return { status: 400, body: { error: "bad-answer" } };
    this.store.expire(now);
    const row = this.store.get(id);
    if (row === null || row.kind !== kind) return { status: 404, body: { error: "gone" } };
    if (row.answer !== null) return { status: 409, body: { error: "answered" } };
    this.store.setAnswer(id, text);
    let delivered = false;
    for (const ws of this.livePc(now)) {
      try {
        ws.send(answerFrame(kind, id, text));
        delivered = true;
      } catch {
        // Sent again on the next connect.
      }
    }
    return { status: 202, body: { delivered } };
  }

  /** The PC connected: every answer it hasn't acknowledged yet. */
  pendingFrames(now) {
    this.store.expire(now);
    return this.store.answered().map((r) => answerFrame(r.kind, r.id, r.answer));
  }
}

/** A Map store for tests (index.js's SQLite store follows its shape). */
export class MemorySlots {
  constructor() {
    this.m = new Map();
  }
  get(id) { return this.m.get(id) || null; }
  put(row) { this.m.set(row.id, { ...row }); }
  setAnswer(id, answer) { const r = this.m.get(id); if (r) r.answer = answer; }
  remove(id) { this.m.delete(id); }
  count() { return this.m.size; }
  answered() { return [...this.m.values()].filter((r) => r.answer !== null); }
  expire(before) { for (const [k, v] of this.m) if (v.exp < before) this.m.delete(k); }
}
