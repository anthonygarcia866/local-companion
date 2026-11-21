// Glim approvals: the phone's side (owner 2026-11-21, docs/remote-runbooks.md §8).
// OWNER-MERGE ONLY (CLAUDE.md, .github/CODEOWNERS).
//
// Pair:    #pair=<slot>&relay=<host>&pk=<the PC's one-time P-256 key>
//          (from the QR code on the PC's screen). Creates a Face ID passkey,
//          derives this approver's key from its PRF output, proves the
//          passkey by signing the slot's challenge, seals all of it to the
//          PC's key and posts it to the relay. Shows the passkey's
//          fingerprint to compare with the PC.
// Approve: #r=<request>&h=<the relay's host, sealed to this passkey's key>
//          (from the Pushover link). Face ID unlocks (PRF), the request is
//          fetched and opened, its details and match code are shown; Approve
//          asks Face ID again to sign SHA-256 of exactly those details.
// Nothing is stored on the phone. Every check that matters is the PC's.
"use strict";

const RP_ID = location.hostname;
const RELAY_HOST = /^[a-z0-9-]{1,63}\.[a-z0-9-]{1,63}\.workers\.dev$/;
const SLOT = /^[0-9a-f]{32}$/;
const te = new TextEncoder();
const td = new TextDecoder();
const view = document.getElementById("view");

// ── helpers ──────────────────────────────────────────────────────────────────

function b64u(bytes) {
  let s = "";
  for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function unb64u(s) {
  const t = s.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(t + "=".repeat((4 - (t.length % 4)) % 4));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}
async function sha256(bytes) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}
async function hkdf(ikm, salt, info) {
  const k = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info: te.encode(info) }, k, 256));
}
async function aes(raw) {
  return crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
}
async function aesOpen(raw, aad, n, ct) {
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: n, additionalData: te.encode(aad) }, await aes(raw), ct));
}
async function aesSeal(raw, aad, plain) {
  const n = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: n, additionalData: te.encode(aad) }, await aes(raw), plain));
  return { n, ct };
}
function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let i = 0;
  for (const p of parts) { out.set(p, i); i += p.length; }
  return out;
}
async function fingerprint(point) {
  const h = await sha256(point);
  const hex = [...h.slice(0, 8)].map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join("");
  return hex.match(/.{4}/g).join(" ");
}
/** The approver key: from the passkey's PRF output (same on every device the passkey syncs to). */
async function approverKey(prfFirst) {
  return hkdf(new Uint8Array(prfFirst), new Uint8Array(0), "glim-approve-v1 key");
}
const PRF_SALT = sha256(te.encode("glim-approve-enc-v1"));

function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "text") e.textContent = v;
    else e.setAttribute(k, v);
  }
  for (const k of kids) e.append(k);
  return e;
}
function show(...nodes) {
  view.replaceChildren(...nodes);
}
function fail(msg) {
  show(el("div", { class: "card" }, el("p", { class: "warn", text: msg })));
}
function button(text, cls, onClick) {
  const b = el("button", { type: "button", class: cls, text });
  b.addEventListener("click", async () => {
    for (const x of view.querySelectorAll("button")) x.disabled = true;
    try {
      await onClick();
    } catch (e) {
      fail(e && e.name === "NotAllowedError" ? "Cancelled, or Face ID didn't confirm." : `Something went wrong: ${e && e.message ? e.message : e}`);
    }
  });
  return b;
}
async function post(host, path, body) {
  const r = await fetch(`https://${host}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), credentials: "omit", referrerPolicy: "no-referrer", cache: "no-store" });
  if (r.status === 409) throw new Error("this request was already answered");
  if (r.status === 404) throw new Error("this request is gone (expired, stopped or answered)");
  if (!r.ok && r.status !== 202) throw new Error(`the relay answered ${r.status}`);
}

// ── pairing ──────────────────────────────────────────────────────────────────

async function pair(slot, host, pcKeyRaw) {
  const cred = await navigator.credentials.create({
    publicKey: {
      rp: { id: RP_ID, name: "Glim approvals" },
      user: { id: crypto.getRandomValues(new Uint8Array(16)), name: "glim-approver", displayName: "Glim approver" },
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      pubKeyCredParams: [{ type: "public-key", alg: -7 }],
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
      attestation: "none",
      extensions: { prf: {} },
    },
  });
  const created = cred.getClientExtensionResults();
  if (!created.prf || created.prf.enabled !== true) throw new Error("this phone's passkeys can't do PRF (needs iOS 18.4 or later, passkey in iCloud Keychain)");
  if (typeof cred.response.getPublicKey !== "function") throw new Error("this browser can't give the passkey's public key (update iOS)");
  const spki = new Uint8Array(cred.response.getPublicKey());
  // Prove the passkey (Face ID) over this slot's challenge, and get its PRF output.
  const got = await navigator.credentials.get({
    publicKey: {
      rpId: RP_ID,
      challenge: await sha256(te.encode(`glim-pair-v1|${slot}`)),
      allowCredentials: [{ type: "public-key", id: cred.rawId }],
      userVerification: "required",
      extensions: { prf: { eval: { first: await PRF_SALT } } },
    },
  });
  const prf = got.getClientExtensionResults().prf;
  if (!prf || !prf.results || !prf.results.first) throw new Error("the passkey gave no PRF output (needs iOS 18.4 or later)");
  const key = await approverKey(prf.results.first);
  const plain = te.encode(JSON.stringify({
    cred: b64u(cred.rawId),
    spki: b64u(spki),
    key: b64u(key),
    name: "iPhone",
    kind: "passkey",
    ad: b64u(got.response.authenticatorData),
    cd: b64u(got.response.clientDataJSON),
    sig: b64u(got.response.signature),
  }));
  // Sealed to the PC's one-time key from the QR code: the relay can't read or forge it.
  const eph = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
  const epk = new Uint8Array(await crypto.subtle.exportKey("raw", eph.publicKey));
  const pc = await crypto.subtle.importKey("raw", pcKeyRaw, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "ECDH", public: pc }, eph.privateKey, 256));
  const info = `glim-pair-v1|${slot}`;
  const k = await hkdf(shared, concat(epk, pcKeyRaw), info);
  const { n, ct } = await aesSeal(k, info, plain);
  await post(host, `/v1/pair/${slot}`, { epk: b64u(epk), n: b64u(n), ct: b64u(ct) });
  const fp = await fingerprint(spki.slice(26));
  show(el("div", { class: "card" },
    el("p", { text: "Sent to your PC. Compare this fingerprint with the one Glim shows:" }),
    el("div", { class: "fp", text: fp }),
    el("p", { class: "small", text: "Only if they match, click “Fingerprints match: add” on the PC. If they don't, click Cancel there." }),
  ));
}

// ── approving ────────────────────────────────────────────────────────────────

/** Exactly what the PC rebuilds and checks (remote/approve.rs `binding`). */
function binding(d) {
  return ["glim-approve-v1", d.rid, d.run, d.runbook, d.hash, String(d.step), d.kind, String(d.total), d.client, String(d.exp), d.code].join("\n");
}

async function unlock(rid, hints) {
  const got = await navigator.credentials.get({
    publicKey: { rpId: RP_ID, challenge: crypto.getRandomValues(new Uint8Array(32)), userVerification: "required", extensions: { prf: { eval: { first: await PRF_SALT } } } },
  });
  const prf = got.getClientExtensionResults().prf;
  if (!prf || !prf.results || !prf.results.first) throw new Error("the passkey gave no PRF output (needs iOS 18.4 or later)");
  const credId = b64u(got.rawId);
  const key = await approverKey(prf.results.first);
  const hint = hints.find((x) => typeof x.c === "string" && credId.startsWith(x.c));
  if (!hint) throw new Error("this passkey isn't an approver for this request");
  const host = td.decode(await aesOpen(key, `glim-approve-v1 relay|${rid}`, unb64u(hint.n), unb64u(hint.ct)));
  if (!RELAY_HOST.test(host)) throw new Error("bad relay address");
  const r = await fetch(`https://${host}/v1/approve/${rid}`, { credentials: "omit", referrerPolicy: "no-referrer", cache: "no-store" });
  if (r.status === 404) throw new Error("this request is gone (expired, stopped or answered)");
  if (r.status === 409) throw new Error("this request was already answered");
  if (!r.ok) throw new Error(`the relay answered ${r.status}`);
  const slot = await r.json();
  const body = JSON.parse(slot.body);
  const item = (body.items || []).find((x) => x.c === credId);
  if (!item) throw new Error("this passkey isn't an approver for this request");
  const d = JSON.parse(td.decode(await aesOpen(key, `glim-approve-v1|${rid}`, unb64u(item.n), unb64u(item.ct))));
  if (d.rid !== rid) throw new Error("the request doesn't match its link");
  if (d.exp * 1000 < Date.now()) throw new Error("this request has expired");
  const when = new Date(d.exp * 1000).toLocaleString();
  show(el("div", { class: "card" },
    el("dl", {},
      el("dt", { text: "Runbook" }), el("dd", { text: d.runbook }),
      el("dt", { text: "Step" }), el("dd", { text: `${d.step} of ${d.total} (${d.kind})` }),
      el("dt", { text: "Asked by" }), el("dd", { text: d.client }),
      el("dt", { text: "Expires" }), el("dd", { text: when }),
    ),
    el("div", { class: "code", text: d.code }),
    el("p", { class: "warn", text: "Compare this match code with the one in the Pushover message. Don't approve if it doesn't match." }),
    button("Approve (Face ID)", "primary", async () => {
      const sig = await navigator.credentials.get({
        publicKey: { rpId: RP_ID, challenge: await sha256(te.encode(binding(d))), allowCredentials: [{ type: "public-key", id: got.rawId }], userVerification: "required" },
      });
      await post(host, `/v1/approve/${rid}`, { answer: "approve", cred: b64u(sig.rawId), ad: b64u(sig.response.authenticatorData), cd: b64u(sig.response.clientDataJSON), sig: b64u(sig.response.signature) });
      show(el("div", { class: "card" }, el("p", { text: "Approved. Glim on your PC checks the signature and runs the step." })));
    }),
    button("Stop the run", "stop", async () => {
      await post(host, `/v1/approve/${rid}`, { answer: "stop" });
      show(el("div", { class: "card" }, el("p", { text: "Stopped. Nothing more runs." })));
    }),
  ));
}

// ── start ────────────────────────────────────────────────────────────────────

(function start() {
  const f = new URLSearchParams(location.hash.slice(1));
  // The fragment held keys and ids: drop it from the address bar and history.
  history.replaceState(null, "", location.pathname);
  if (!window.PublicKeyCredential || !crypto.subtle) return fail("This browser can't use passkeys. Open the link in Safari on iOS 18.4 or later.");
  const slot = f.get("pair");
  if (slot) {
    const host = f.get("relay") || "";
    const pk = f.get("pk") || "";
    if (!SLOT.test(slot) || !RELAY_HOST.test(host) || !/^[A-Za-z0-9_-]{87}$/.test(pk)) return fail("This pairing link isn't valid. Start again on the PC.");
    return show(el("div", { class: "card" },
      el("p", { text: "Pair this phone with Glim. It creates a passkey for Glim approvals (Face ID) and sends its public key to your PC." }),
      button("Create passkey", "primary", () => pair(slot, host, unb64u(pk))),
    ));
  }
  const rid = f.get("r");
  if (rid) {
    let hints = null;
    try {
      hints = JSON.parse(td.decode(unb64u(f.get("h") || "")));
    } catch { /* below */ }
    if (!SLOT.test(rid) || !Array.isArray(hints)) return fail("This approval link isn't valid.");
    return show(el("div", { class: "card" },
      el("p", { text: "A remote runbook step is waiting for your OK. Unlock with Face ID to see what it is." }),
      button("Unlock (Face ID)", "primary", () => unlock(rid, hints)),
    ));
  }
  fail("Open this page from Glim: a pairing QR code on your PC, or an approval link in Pushover.");
})();
