// Phone approval slots (relay/src/approvals.js; owner 2026-11-21).
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Approvals, MemorySlots, MAX_SLOTS, MAX_APPROVAL_S, MAX_PAIR_S, MAX_ANSWER_BYTES, ipKey, phoneRateLimit, readCapped } from "../src/approvals.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const NOW = 1_795_238_795;
const ID = "0123456789abcdef0123456789abcdef";
const ID2 = "fedcba9876543210fedcba9876543210";

function pcSocket() {
  const sent = [];
  return { sent, ws: { send: (f) => sent.push(JSON.parse(f)) } };
}

test("the PC opens a sealed request; the page reads it and answers once; the PC gets it until it acks", () => {
  const pc = pcSocket();
  const a = new Approvals(new MemorySlots(), () => [pc.ws]);
  a.fromPc({ t: "approval", id: ID, exp: NOW + 3600, body: "sealed-blob" }, NOW);
  assert.deepEqual(a.read("approval", ID, NOW).body, { kind: "approval", exp: NOW + 3600, body: "sealed-blob" });
  assert.equal(a.read("pair", ID, NOW).status, 404, "kinds don't mix");
  const answer = JSON.stringify({ answer: "approve", cred: "x", ad: "y", cd: "z", sig: "w" });
  assert.deepEqual(a.answer("approval", ID, answer, NOW + 1), { status: 202, body: { delivered: true } });
  assert.deepEqual(pc.sent.at(-1), { t: "approval-answer", id: ID, answer });
  assert.equal(a.answer("approval", ID, JSON.stringify({ answer: "stop" }), NOW + 2).status, 409, "single use");
  assert.equal(a.read("approval", ID, NOW + 2).status, 409, "answered: no longer readable");
  // Not acknowledged: sent again on the next connect; acknowledged: gone.
  assert.equal(a.pendingFrames(NOW + 3).length, 1);
  a.fromPc({ t: "approval-ack", id: ID }, NOW + 4);
  assert.equal(a.pendingFrames(NOW + 5).length, 0);
  assert.equal(a.read("approval", ID, NOW + 5).status, 404);
});

test("the kill switch's void, expiry, bad ids, sizes and bad answers", () => {
  const a = new Approvals(new MemorySlots(), () => []);
  a.fromPc({ t: "approval", id: ID, exp: NOW + 60, body: "b" }, NOW);
  a.fromPc({ t: "approval-void", id: ID }, NOW + 1);
  assert.equal(a.read("approval", ID, NOW + 1).status, 404, "voided");
  a.fromPc({ t: "pair", id: ID2, exp: NOW + 60 }, NOW);
  assert.equal(a.read("pair", ID2, NOW + 61).status, 404, "expired");
  for (const bad of ["", "../x", ID.toUpperCase(), ID + "0"]) assert.equal(a.read("approval", bad, NOW).status, 400, bad);
  // Lifetimes are capped by kind; an answer must be a JSON object of bounded size.
  a.fromPc({ t: "approval", id: ID, exp: NOW + MAX_APPROVAL_S + 3600, body: "b" }, NOW);
  assert.equal(a.read("approval", ID, NOW).status, 404, "too long-lived");
  a.fromPc({ t: "pair", id: ID, exp: NOW + MAX_PAIR_S + 3600 }, NOW);
  assert.equal(a.read("pair", ID, NOW).status, 404);
  a.fromPc({ t: "approval", id: ID, exp: NOW + 60, body: "b" }, NOW);
  assert.equal(a.answer("approval", ID, "not json", NOW).status, 400);
  assert.equal(a.answer("approval", ID, "[1]", NOW).status, 400);
  assert.equal(a.answer("approval", ID, "x".repeat(MAX_ANSWER_BYTES + 1), NOW).status, 413);
  // The PC is offline: the answer waits.
  assert.deepEqual(a.answer("approval", ID, "{}", NOW).body, { delivered: false });
  assert.equal(a.pendingFrames(NOW).length, 1);
});

test("at most MAX_SLOTS slots, and an id can't be reopened while it exists", () => {
  const a = new Approvals(new MemorySlots(), () => []);
  for (let i = 0; i < MAX_SLOTS + 5; i++) a.fromPc({ t: "pair", id: i.toString(16).padStart(32, "0"), exp: NOW + 60 }, NOW);
  assert.equal(a.store.count(), MAX_SLOTS);
  a.fromPc({ t: "approval", id: ID, exp: NOW + 60, body: "first" }, NOW);
  assert.equal(a.read("approval", ID, NOW).status, 404, "full");
});

test("the Worker routes phone requests without a token, with CORS only for the page's origin, and never logs", () => {
  const index = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  const approvals = fs.readFileSync(path.join(here, "../src/approvals.js"), "utf8");
  assert.match(index, /PAGE_ORIGIN = "https:\/\/anthonygarcia866\.github\.io"/);
  assert.ok(!/access-control-allow-origin": "\*"/.test(index), "never a wildcard origin");
  assert.match(index, /\[CLIENT_HEADER, "authorization", "content-length", "transfer-encoding"\]\) headers\.delete\(h\)/, "a phone request never carries a token (or a client name) inward");
  for (const src of [index, approvals]) assert.ok(!/console\./.test(src), "nothing printed");
});

// A request whose stream yields `chunks` and whose headers say what we like.
function fakeRequest(headers, chunks) {
  const enc = new TextEncoder();
  let i = 0;
  const body = chunks === null ? null : new ReadableStream({
    pull(c) { if (i < chunks.length) c.enqueue(enc.encode(chunks[i++])); else c.close(); },
  });
  return { headers: new Headers(headers), body };
}

test("phone POST bodies: Content-Length required and checked against the stream, hard cap", async () => {
  assert.deepEqual(await readCapped(fakeRequest({ "content-length": "2" }, ["{}"]), 10), { ok: true, text: "{}" });
  assert.equal((await readCapped(fakeRequest({}, ["{}"]), 10)).status, 411, "no length");
  assert.equal((await readCapped(fakeRequest({ "transfer-encoding": "chunked", "content-length": "2" }, ["{}"]), 10)).status, 411, "chunked");
  assert.equal((await readCapped(fakeRequest({ "content-length": "-1" }, ["{}"]), 10)).status, 400);
  assert.equal((await readCapped(fakeRequest({ "content-length": "11" }, ["x".repeat(11)]), 10)).status, 413, "declared too big");
  // Lies about its length: the stream is cut as soon as it passes it.
  assert.equal((await readCapped(fakeRequest({ "content-length": "2" }, ["{}", "x".repeat(100000)]), 10)).status, 413, "longer than declared");
  assert.equal((await readCapped(fakeRequest({ "content-length": "5" }, ["{}"]), 10)).status, 400, "shorter than declared");
  assert.deepEqual(await readCapped(fakeRequest({ "content-length": "0" }, null), 10), { ok: true, text: "" });
  assert.equal(MAX_ANSWER_BYTES, 8 * 1024);
  // The Durable Object's check: no Content-Length needed (the Worker drops
  // it), the stream cap still holds.
  assert.deepEqual(await readCapped(fakeRequest({}, ["{}"]), 10, true), { ok: true, text: "{}" });
  assert.equal((await readCapped(fakeRequest({}, ["x".repeat(6), "x".repeat(6)]), 10, true)).status, 413);
  assert.deepEqual(await readCapped(fakeRequest({}, null), 10, true), { ok: true, text: "" });
});

test("the Worker reads a phone body capped before forwarding it", () => {
  const index = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  const entry = index.slice(index.indexOf("export default"), index.indexOf("class SqlStore"));
  assert.match(entry, /readCapped\(request, MAX_ANSWER_BYTES\)/, "in the Worker, not only the Durable Object");
  assert.ok(!/request\.text\(\)/.test(entry), "never an uncapped read");
});

// A stand-in for a Workers rate-limit binding: `limit` calls per key.
function fakeLimiter(limit) {
  const seen = new Map();
  return { keys: seen, async limit({ key }) { const n = (seen.get(key) || 0) + 1; seen.set(key, n); return { success: n <= limit }; } };
}
const ipReq = (ip) => ({ headers: new Headers(ip ? { "cf-connecting-ip": ip } : {}) });

test("phone rate limits: per IP, then overall; fails closed", async () => {
  const env = { PHONE_IP_LIMIT: fakeLimiter(2), PHONE_ALL_LIMIT: fakeLimiter(3) };
  assert.equal(await phoneRateLimit(ipReq("192.0.2.1"), env), null);
  assert.equal(await phoneRateLimit(ipReq("192.0.2.1"), env), null);
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.1"), env), { status: 429, error: "rate-limited" }, "per IP");
  assert.equal(env.PHONE_IP_LIMIT.keys.get("ip:192.0.2.1"), 3);
  assert.equal(env.PHONE_ALL_LIMIT.keys.get("phone-all"), 2, "an IP refusal doesn't count overall");
  assert.equal(await phoneRateLimit(ipReq("192.0.2.2"), env), null);
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.3"), env), { status: 429, error: "rate-limited" }, "overall");
  assert.equal(await phoneRateLimit(ipReq(null), { PHONE_IP_LIMIT: fakeLimiter(5), PHONE_ALL_LIMIT: fakeLimiter(5) }), null, "no IP header: one shared key");
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.1"), {}), { status: 503, error: "rate-limit-unavailable" }, "no binding");
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.1"), { PHONE_IP_LIMIT: fakeLimiter(5) }), { status: 503, error: "rate-limit-unavailable" }, "one binding missing");
  const boom = { async limit() { throw new Error("x"); } };
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.1"), { PHONE_IP_LIMIT: boom, PHONE_ALL_LIMIT: fakeLimiter(5) }), { status: 503, error: "rate-limit-unavailable" }, "throws");
  assert.deepEqual(await phoneRateLimit(ipReq("192.0.2.1"), { PHONE_IP_LIMIT: { async limit() { return {}; } }, PHONE_ALL_LIMIT: fakeLimiter(5) }), { status: 429, error: "rate-limited" }, "no success field");
});

test("the Worker rate-limits phone routes before reading the body or waking the mailbox", () => {
  const index = fs.readFileSync(path.join(here, "../src/index.js"), "utf8");
  const entry = index.slice(index.indexOf("export default"), index.indexOf("class SqlStore"));
  const lim = entry.indexOf("phoneRateLimit(request, env)");
  assert.ok(lim > 0, "called in the Worker");
  assert.ok(lim < entry.indexOf("readCapped(request, MAX_ANSWER_BYTES)"), "before the body read");
  assert.ok(lim < entry.indexOf("env.RELAY.get"), "before the mailbox");
  const toml = fs.readFileSync(path.join(here, "../wrangler.toml"), "utf8");
  assert.match(toml, /name = "PHONE_IP_LIMIT"/);
  assert.match(toml, /name = "PHONE_ALL_LIMIT"/);
});

test("the per-IP key: IPv4 as is, IPv6 by its /64, junk shares one key", () => {
  assert.equal(ipKey("192.0.2.7"), "192.0.2.7");
  assert.equal(ipKey("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
  assert.equal(ipKey("2001:0db8:0001:0002:ffff:ffff:ffff:ffff"), "2001:db8:1:2::/64", "same /64, other host");
  assert.equal(ipKey("2001:db8::1"), "2001:db8:0:0::/64");
  assert.equal(ipKey("::1"), "0:0:0:0::/64");
  for (const bad of [null, "", "x", "1::2::3", "1:2:3", "12345::1", "a".repeat(80)]) assert.equal(ipKey(bad), "unknown", String(bad));
});
