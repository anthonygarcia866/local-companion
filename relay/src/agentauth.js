// Token-less agent clients (owner 2026-11-21): a cloud client registered with
// only its Ed25519 PUBLIC key (Settings → Remote → Cloud clients → "Add agent
// client"). Instead of a bearer token, every request it makes is signed with
// its task-signing key; the relay checks the signature here, the Durable
// Object remembers the nonce. Token clients work as before (core.js caller).
//
// CLOUD_CLIENTS entry: "<name>=key:<64 hex public key>".
//
// Authorization: GlimSig client=<name>,ts=<unix s>,nonce=<32 hex>,sig=<128 hex>
//
// The signature covers, one per line (signedString):
//   glim-relay-v1 / client / METHOD / host / path?query / sha256(body) hex /
//   ts / nonce
// so a request can't be replayed against another route, body, relay or
// client. The first line can't start a task payload (JSON), so a relay
// signature is never a valid task signature and the other way round.
// ts must be within SKEW_S of the relay's clock; each (client, nonce) is
// accepted once while it could still be valid (NONCE_KEEP_S).

import { CLIENT_NAME, LEGACY_CLIENT, MAX_CLIENTS } from "./core.js";

export const SCHEME = "GlimSig";
export const DOMAIN = "glim-relay-v1";
export const SKEW_S = 60;
/** A nonce is remembered while its ts could still pass: ±SKEW_S. */
export const NONCE_KEEP_S = 2 * SKEW_S + 1;
const HEADER = /^GlimSig client=([a-z0-9][a-z0-9-]{0,31}),ts=([0-9]{1,12}),nonce=([0-9a-f]{32}),sig=([0-9a-f]{128})$/;
const KEY = /^key:([0-9a-f]{64})$/;

/** name -> 64-hex public key, from CLOUD_CLIENTS' "key:" entries. Names
 * also used by a token entry, a key two names share, a bad entry, the
 * legacy name: dropped. At most MAX_CLIENTS. */
export function keyClients(env) {
  const out = new Map();
  if (env.CLOUD_CLIENTS === undefined || env.CLOUD_CLIENTS === null) return out;
  const names = new Map();
  for (const part of String(env.CLOUD_CLIENTS).split(",")) {
    const [name, value, extra] = part.trim().split("=");
    if (extra !== undefined || !CLIENT_NAME.test(name || "")) continue;
    names.set(name, (names.get(name) || 0) + 1);
    const m = KEY.exec(value || "");
    if (m && name !== LEGACY_CLIENT && !out.has(name) && out.size < MAX_CLIENTS) out.set(name, m[1]);
  }
  const counts = new Map();
  for (const k of out.values()) counts.set(k, (counts.get(k) || 0) + 1);
  for (const [name, k] of out) if (names.get(name) > 1 || counts.get(k) > 1) out.delete(name);
  return out;
}

export function signedString({ client, method, host, path, bodyHash, ts, nonce }) {
  return [DOMAIN, client, method, host, path, bodyHash, String(ts), nonce].join("\n");
}

async function sha256HexBytes(bytes) {
  const d = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function bodyHashOf(text) {
  return sha256HexBytes(new TextEncoder().encode(text || ""));
}

const bytesOf = (hex) => new Uint8Array(hex.match(/../g).map((b) => parseInt(b, 16)));

/** A "GlimSig …" header, parsed, or null. */
export function parseHeader(authorization) {
  const m = HEADER.exec(authorization || "");
  return m ? { client: m[1], ts: Number(m[2]), nonce: m[3], sig: m[4] } : null;
}

/**
 * The agent client a signed request comes from: { client, nonce } or null.
 * `req`: { method, host, path (with ?query), body (text) }. The nonce is
 * NOT checked here (the Durable Object remembers them: Relay.useNonce).
 * Refuses everyone while the PC has no token hash (as core.js caller).
 */
export async function signedCaller(authorization, req, env, nowS) {
  const h = parseHeader(authorization);
  if (!h) return null;
  if (!/^[0-9a-f]{64}$/.test(String(env.PC_TOKEN_SHA256 || "").trim().toLowerCase())) return null;
  const key = keyClients(env).get(h.client);
  if (!key) return null;
  if (!Number.isSafeInteger(h.ts) || Math.abs(nowS - h.ts) > SKEW_S) return null;
  const text = signedString({ client: h.client, method: req.method, host: req.host, path: req.path, bodyHash: await bodyHashOf(req.body), ts: h.ts, nonce: h.nonce });
  let ok = false;
  try {
    const k = await crypto.subtle.importKey("raw", bytesOf(key), { name: "Ed25519" }, false, ["verify"]);
    ok = await crypto.subtle.verify({ name: "Ed25519" }, k, bytesOf(h.sig), new TextEncoder().encode(text));
  } catch {
    ok = false;
  }
  return ok ? { client: h.client, nonce: h.nonce } : null;
}
