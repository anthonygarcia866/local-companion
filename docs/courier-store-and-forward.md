# Courier store-and-forward: design note (2026-11-17)

Status: **APPROVED by the owner 2026-11-17 and BUILT** (PR #137: relay → Glim app → `courier.mjs`). Roadmap item 1 of "connect cloud Glim to the Glim app" (PROJECT_STATUS.md). Builds on `docs/remote-courier.md`, which now describes the result.

## The owner's decisions (2026-11-17)

- **D1** 24 h default, 72 h cap; `start-session` and `send-to-session` 24 h at most. **`send-to-session` must name its target session id**; the PC refuses it when that session no longer exists (`no-such-session`) or isn't that account's (`not-your-session`).
- **D2** results kept 72 h. **D3** deleted only on the client's acknowledgement, after it has opened and verified them. **D4** tasks sealed to the PC in this PR; `GLIM_PC_ENC_KEY` (public) in Settings → Remote, with a Copy button, and in the per-account setup docs. **D5** the `results` listing shows ids, states and times only.
- Also: domain-separated key derivation (HKDF with distinct labels for task and result keys; the signing key is never used directly); the kill switch and duplicate-id rules apply to long-lived tasks; **a task expired at the relay never runs**.

## ⚠ Changes from the reviewed design, found while building (2026-11-17)

- **One new crate, `x25519-dalek` 2** (plus `curve25519-dalek`, `curve25519-dalek-derive`, `fiat-crypto`, `zeroize_derive`), in `remote/seal.rs` only. §3 said "no new crate", which held for results only: `ring` offers X25519 **ephemeral** keys only, and D4 needs the PC to keep a **static** encryption key. Pure Rust, no network, no telemetry, no I/O; default features off, secrets zeroized. Called out in the PR.
- **Random nonces**, not a zero nonce: the task's ephemeral key is derived from the seed and id (so the sender can re-open its own task), which makes the key repeat if a task were ever re-sealed. A random 12-byte nonce removes that worry for both directions.
- **The AEAD label binds the task id, not the client name**: cloud Glim doesn't know the name Glim gave it. Nothing is lost: only the per-task result key opens a result, and the gate checks a task's signature with the key of the client the relay named.
- **The PC's encryption key is its own random key** (`Glim/RemotePcEncKey`), not derived from the result key: made by "Set up the relay", or once at startup on a PC set up before.
- **Expiry: no clock skew after `exp`** at the relay or on the PC (was 60 s), so "expired at the relay" and "refused on the PC" are the same second.
- **`GET /v1/results/<id>` still returns the bare result** (old couriers keep working, sync `send` at every step); `?with=task` returns `{task, result}`.
- **Unsealed tasks are still accepted** (an older courier, or `GLIM_PC_ENC_KEY` not set yet: `courier.mjs` warns). Results are sealed whenever the signed task names `rk`.

## The problem

Cloud Glim is not a live socket. A cloud session sends a task, may end, and a later session (maybe tomorrow, maybe the other account's) follows up. Today:

- A task lives at most **10 minutes** (`exp - iat` ≤ 600 at the relay and in `remote::Gate`). A PC that is asleep for longer never sees it.
- `courier.mjs send` waits for the result in the same process (60 s by default). The relay already keeps a result until it is fetched or the task's expiry passes, and deletes it on the first `GET`. If the response is lost on the way, the result is gone.
- Verifying a result needs the exact task envelope that was sent (`result.task` = its SHA-256), which only the sending process holds.
- Results sit on the relay **in plaintext**, readable by anyone who can read the Durable Object (Cloudflare, or whoever holds the account).

## What changes, in one picture

```
cloud Glim (session A)                  relay                         the Glim app
  send --async  ── task (rk in payload, ──▶ keeps the task ≤ 24 h ──▶ checks, runs,
                   signed) ◀── {id}          (cap 72 h)                 signs the result,
                                                                        SEALS it to rk
                                        keeps the sealed result ◀───  (ChaCha20-Poly1305)
cloud Glim (session B, any time later)    ≤ 72 h, or until acked
  result <id> ──────────────────────────▶ {task, sealed result}
     re-derives rk's private key from GLIM_TASK_SIGNING_KEY + id, opens it,
     verifies the PC's signature + its own signature on the task, then ACKs (deleted)
```

## 1. Task lifetime

- `exp - iat` may be up to **72 h** (cap; was 600 s), checked at the relay (`taskMeta`) and on the PC (`remote::MAX_LIFETIME_S`). `courier.mjs` defaults to **24 h**; `--ttl <30s..72h>` sets it per task.
- Unchanged: the 60 s clock skew, the replay cache (an id stays on disk until its own expiry, so at most 72 h; bounded by the rate limits, ≤ 50 commands an hour on the PC), the kill switch (a task issued before **Allow again** is answered `killed`), the rate limits (counted when the PC **runs** a task, not when it was queued), the 50-pending cap at the relay.
- **Stale intent** is the real cost of long lifetimes: a `send-to-session` written at noon could run at 11:00 the next day, into whatever that session is now doing. The defaults below (D1) give it a shorter cap; the result's `started` time (roadmap item 2) makes it visible.

## 2. Results at rest: kept, then deleted

- The relay keeps a result for **72 h after it arrives** (`result_exp`, D2) or until the client **acknowledges** it, whichever is first; an alarm purges expired ones (no cron, as now).
- `GET /v1/results/<id>` returns it **without deleting**; `DELETE /v1/results/<id>` deletes it (D3). `courier.mjs result` deletes it only after it decrypted and verified, so a dropped response or a crash can't lose a result. Only the client that queued the task can fetch, list or delete it (`404` for anyone else, as now).
- `GET /v1/results` lists the client's own tasks and results: `[{id, state: "pending" | "ready", exp, readyAt}]`. Ids, states and times only, never a command, an argument or a result.
- Limits: ≤ 200 unacknowledged results per client (new tasks `429 too-many-results` beyond that), a sealed result ≤ 48 KB (was 32 KB plain; the seal adds ~40 bytes plus base64).
- Still nothing logged, observability stays off.

## 3. Results encrypted to the sending client

The relay can carry a result but never read it.

- **Per-task key, derived, nothing to store.** For each task, cloud Glim derives an X25519 key pair from its signing seed and the task id: `rk_priv = HKDF-SHA256(ikm = GLIM_TASK_SIGNING_KEY seed, salt = "glim-courier-v2", info = "result-key" || id)`, clamped as X25519 requires. The public half goes into the **signed** payload as `rk` (64 hex). A later session re-derives the private half from the same seed and id: no key file, no state between sessions.
- **The PC seals** (only for tasks whose signature verified with that client's key):
  1. Build and sign the result exactly as today (v1 envelope, PC result key).
  2. Ephemeral X25519 key `e` (`ring::agreement`), `shared = X25519(e, rk)`.
  3. `key = HKDF-SHA256(shared, salt = epk || rk, info = "glim-courier-v2 result")`, single use.
  4. `ct = ChaCha20-Poly1305(key, nonce = 12 zero bytes, aad = "glim-result-v2|" + id + "|" + client, plaintext = the signed v1 envelope)`. A zero nonce is safe because the key is never used twice (a fresh `e` every time).
  5. Send `{"t":"result","id":…,"env":"{\"v\":2,\"id\":…,\"epk\":…,\"ct\":…}"}`.
- **Cloud Glim opens and verifies**: re-derive `rk_priv`, open, then run today's `verifyResult` on the inner envelope (PC signature, id, task hash, time). The AAD binds the client name and id: a result the relay files under another id or client fails to open.
- **Refusals of tasks that didn't verify** (bad signature, wrong client, unknown client) stay **v1, unsealed**: their `rk` came from an unverified source, and they carry only a fixed error code. Cloud Glim accepts an unsealed result only if it is such a refusal; an unsealed success when it sent `rk` is `bad-result` (no downgrade).
- **Crypto on the PC uses `ring` 0.17**, already a direct dependency (rustls): X25519, HKDF-SHA256, ChaCha20-Poly1305. **No new crate.** cloud Glim uses Node's built-in `crypto` (X25519, HKDF, chacha20-poly1305): no new package.
- Shared test vectors (fixed seed, id, `e`): `relay/test/` and `remote/courier_tests.rs` check the same bytes, as the signatures do today.

## 4. Verifying later, in another session

`verifyResult` needs the task envelope that was sent. A later cloud session doesn't have it, so the relay **keeps the task envelope next to its result** and returns both: `{"task": "<envelope>", "result": "<sealed>"}`. Cloud Glim then checks, in order:

1. The task envelope verifies with **its own** public key (derived from `GLIM_TASK_SIGNING_KEY`) and carries the id it asked for. The relay can't make one up: it can't sign.
2. The result opens with the key derived from that id, and the inner envelope passes `verifyResult` against that task (`task` = its SHA-256; `at` between the task's `iat - 60` and `exp + 60`, was "within 5 minutes of now").

## 5. Courier commands (`relay/courier.mjs`)

| Command | Does | Exit |
|---|---|---|
| `send <cmd> [args] [--ttl 24h] [--wait 60]` | As today, now with `rk` and the lifetime; if the wait runs out, prints the id: fetch it later with `result` | 0 ok, 1 refused/failed, 2 bad result, **3 still pending (id printed)**, 4 setup |
| `send --async <cmd> [args] [--ttl 24h]` | Queues and prints `{"id","exp"}` | 0 queued, 1 refused, 4 setup |
| `result <id> [--wait 25] [--keep]` | Fetches, opens, verifies, prints, then acks (`--keep`: no ack) | 0 / 1 / 2 as above, 3 pending, **5 unknown or expired** |
| `results` | This client's pending and ready ids, states, times | 0 |
| `status` | Unchanged here (`last_seen` is roadmap item 3) | 0 |

## 6. What changes where

- **Relay** (`relay/src/core.js`, `index.js`): lifetime cap 72 h; the results table gains `task` (the envelope), `ready_at`, `result_exp`; a result envelope may be v1 or v2 (`{v:2,id,epk,ct}`, the id must match); `GET /v1/results` (list), `DELETE /v1/results/<id>` (ack), GET no longer deletes; the alarm purges by `min(exp, result_exp)`; the per-client result cap. Deployed through its existing path (wrangler, OAuth).
- **The Glim app** (`remote/mod.rs`, `courier.rs`): `MAX_LIFETIME_S` 72 h; an optional `rk` in the payload (64 hex, refused as `bad-args` otherwise); sealing in a new `remote/seal.rs` (the only code using `ring::agreement` / `ring::aead`; a test scans). **No new network path**: the sealed result goes over the existing relay socket.
- **Docs**: `docs/remote-courier.md` (routes, lifetimes, the result format), CLAUDE.md's courier paragraph (results sealed at rest, kept ≤ 72 h until acked), PRIVACY.md, PROJECT_STATUS.md.
- **Rollout**, each step leaving everything working: relay first (takes v1 and v2, old clients unaffected) → the Glim app (glim-update) → `courier.mjs` on main (cloud sessions run it from the repo). An older Glim ignores `rk` (the payload isn't strict about extra fields) and answers unsealed; the new courier refuses that as `bad-result`, so update the PC before using `--async`.

## 7. Threat review (Claude, transport and signatures)

| Who / what | Can | Can't |
|---|---|---|
| The relay, or someone reading its storage | See tasks (D4), ids, times, which client; delay a task until its expiry (now up to 72 h); delete or withhold results (availability only) | Read a result; swap `rk` (it's signed); file a result under another id or client (AAD); make a task or result either end accepts |
| A stolen **cloud token** | Queue tasks (unsigned ones are refused), list that client's ids, fetch its sealed results, **ack (delete) them** (a nuisance: revoke with Remove) | Read results; get a task run |
| A stolen **task-signing key** | Everything that client can do already, **plus read its results still on the relay (≤ 72 h)**, since the per-task keys derive from the seed | Act as the other client |
| A stolen **PC token** | Read pending tasks (D4) | Produce a result cloud Glim accepts (needs the PC result key) |

- Using the Ed25519 seed as HKDF input for X25519 keys is standard domain separation (distinct salt and info; Ed25519 itself only ever feeds the seed to SHA-512). The cost: no forward secrecy for results still on the relay if a seed leaks. Accepted, because results are deleted once acked and live 72 h at most.
- The relay can replay a task, but the replay cache holds every id until its expiry, so one runs once.

## Decisions for the owner

- **D1 Lifetimes**: default **24 h**, cap **72 h**. Recommended: also cap `send-to-session` and `start-session` at **24 h**, since they act into live work (stale intent, §1).
- **D2 Result retention**: **72 h** after it arrives, then purged even if it was never fetched.
- **D3 Delete on ack** (recommended; a lost response can't lose a result) or delete on the first fetch (today's behaviour, simpler).
- **D4 Tasks are still plaintext at rest**, and with 24 h lifetimes a `start-session` prompt (≤ 2,000 characters) or `send-to-session` text (≤ 4,000) can sit on the relay for a day. Recommended: **also seal tasks to the PC in this PR**: the Glim app shows a PC encryption key (X25519, derived from its result key, next to it in Settings), each cloud environment gets one more public value, `GLIM_PC_ENC_KEY`; a task is sealed the same way with an ephemeral key derived from the seed and id, so the sender can still re-open its own task for step 4. Otherwise, a separate PR later.
- **D5 `results` listing**: ids, states and times only (recommended), or also the command kind.
