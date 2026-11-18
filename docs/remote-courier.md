# Remote courier: relay, tasks and results (2026-11-14)

Status: **wired** (owner approved transport (a), a Cloudflare Worker + Durable Object relay, 2026-11-14). The security core is `windows/src-tauri/src/remote/`, the PC's connection is `remote/courier.rs` + `net/relay.rs`, and the five commands live in `remote/exec.rs`. The relay is `relay/` (Worker `task-relay`, Workers Free plan). cloud Glim's client is `relay/courier.mjs`.

**Store-and-forward (owner, 2026-11-17; design and decisions: `docs/courier-store-and-forward.md`):** cloud Glim needn't stay connected. A task lives **24 h** by default (72 h at most; `start-session` and `send-to-session` 24 h at most) and is **sealed to the PC** (`GLIM_PC_ENC_KEY`). The PC's signed result is **sealed to a per-task result key** the task names. The relay keeps the result until the sending client **acknowledges** it, or 72 h at most, and can read neither. `send --async` / `result <id>` / `results` work across sessions. `send-to-session` names its target session.

Naming: **cloud Glim** is the agent (a cloud Claude session acting for the owner) that signs and sends tasks. **The Glim app** is the Windows app that checks them, acts, and signs results.

```
cloud Glim ──POST /v1/tasks (cloud token)──▶  relay (Worker + DO)  ◀──wss /v1/pc (PC token)── the Glim app
           ◀─GET /v1/results/<id> ──────────  keeps a task until         (connects out; nothing
              (signed by the PC, sealed        answered or expired        listens on the PC)
               to the task's result key)       (≤ 72 h), a result until
             ──DELETE /v1/results/<id>──────▶  acknowledged (≤ 72 h)
```

## Who holds what

| Secret / key | Made by | Kept | Never |
|---|---|---|---|
| Task-signing key (Ed25519 private), one per cloud client | `node relay/courier.mjs keygen` (writes `%LOCALAPPDATA%\Glim\task-signing-key.txt`, never a OneDrive-synced folder such as Desktop or Documents; delete it after copying) | that client's cloud environment, `GLIM_TASK_SIGNING_KEY` | on the PC once copied (Glim stores only its public key), in the repo or chat |
| Task-signing public key | the same | Glim: Settings → Remote → Remote tasks → Cloud clients (fingerprint shown) | n/a (public) |
| PC result key (Ed25519 private) | Glim, "Set up the relay" | Credential Manager `Glim/RemotePcSigningKey` (PKCS#8) | in a file, a log, the page or the network |
| PC result public key | the same | Settings (shown with its fingerprint); cloud Glim's `GLIM_PC_PUBLIC_KEY` | n/a (public) |
| PC encryption key (X25519 private; 2026-11-17) | Glim, "Set up the relay", or once at startup on a PC set up before | Credential Manager `Glim/RemotePcEncKey` | in a file, a log, the page or the network |
| PC encryption public key | the same | Settings → Remote → "This PC's encryption key (GLIM_PC_ENC_KEY)" (Copy button, fingerprint); each cloud environment's `GLIM_PC_ENC_KEY` | n/a (public) |
| Per-task keys (a task's ephemeral key, its result key) | derived by `courier.mjs` from `GLIM_TASK_SIGNING_KEY` and the task id (HKDF-SHA256, labels `glim-courier-v2 task-ephemeral\|<id>` / `result-key\|<id>`) | nowhere: re-derived when needed | n/a |
| PC relay token (32 random bytes, hex) | the same | Credential Manager `Glim/RemoteRelayPcToken`; only sent as the WebSocket upgrade's `Authorization: Bearer` | in a URL, a log or the page |
| Cloud relay token (32 random bytes, hex), one per cloud client | Glim, **Add cloud client** (account-1's is the one "Set up the relay" made before 2026-11-15) | shown **once** in Settings, then only in that client's claude.ai/code **network secret** (recommended, see below) or, as a fallback, its environment variable `RELAY_CLOUD_TOKEN`; Glim forgets it | on the PC after that, in the repo or chat |
| Token hashes (SHA-256 hex) | Glim | Settings → Secrets `REMOTE_PC_TOKEN_SHA256` → Worker secret `PC_TOKEN_SHA256`; `REMOTE_CLOUD_CLIENTS` (the list `account-1=legacy,account-2=<hash>`, rewritten on every Add and Remove) → Worker secret `CLOUD_CLIENTS`; account-1's own hash stays the Worker secret `CLOUD_TOKEN_SHA256` from before clients (`legacy`). The approved `relay-tokens` runbook pipes them to `wrangler secret put` on stdin | n/a (a hash of 32 random bytes reveals nothing) |

The relay holds only hashes: it tells the PC and each cloud client apart, and each token works only on its own side's routes. A stolen cloud token can queue tasks, but every task still needs **that client's** task-signing key. A stolen PC token can read the queue, but it can't produce results cloud Glim accepts without the PC result key.

## The relay (`relay/`)

- Worker `task-relay` on `*.workers.dev`, Workers Free plan, one SQLite-backed Durable Object (`Mailbox`). There are no cron triggers: purging uses a DO alarm, which doesn't count against the account's cron ceiling.
- `GET /v1/pc` (PC token): a hibernatable WebSocket. On connect, every pending task arrives at once; new ones arrive as they're queued. A new connection replaces the old one. The relay answers `{"t":"ping"}` with `{"t":"pong"}` without waking up.
- Cloud clients (owner 2026-11-15): one per Claude account (`account-1`, `account-2` …), each with its own relay token and task-signing key, each revocable on its own. `CLOUD_CLIENTS` lists them; without it, `CLOUD_TOKEN_SHA256` alone is `account-1` (`relay/src/core.js` `cloudClients`). The relay stamps every task it hands the PC with the client whose token queued it (`{"t":"task","env":…,"client":"account-2"}`; the Worker sets it, never the caller), and a client sees only its own results and pending count.
- `POST /v1/tasks` (cloud token): one envelope, ≤ 34 KB (a sealed one is the signed task in hex). It is refused when malformed, expired, valid for more than 72 h, a duplicate id, when 50 are already pending, or when that client has 200 unacknowledged results.
- `GET /v1/results/<id>?wait=1-25[&with=task]` (cloud token): the result of one of **that client's** tasks, kept until acknowledged; `with=task` answers `{task, result}` (the task envelope it answers too, so a later session can verify it). 202 = still pending, 404 = unknown, expired or acknowledged.
- `DELETE /v1/results/<id>` (cloud token): acknowledge (delete) it. `courier.mjs` does this only after it has opened and verified it.
- `GET /v1/results` (cloud token): that client's pending tasks and kept results, `[{id, state: "pending"|"ready", exp, readyAt}]`: ids, states and times only.
- `GET /v1/status` (cloud token): `{"pc": connected?, "pending": n}` (n = that client's own tasks).
- What is stored: a task until its result arrives or its expiry (no grace after it: an expired task is never handed over); a result, with the task envelope it answers, until acknowledged or 72 h after it arrived. Sealed envelopes (`{"v":2,…}`): the relay reads only their id and times. Nothing else. No `console.*` (a test checks), and Workers Logs/observability are off in `wrangler.toml`. The relay checks neither signature nor seal; the two ends do.

## Task (cloud Glim → the Glim app)

Envelope, as carried:

```json
{ "v": 1, "payload": "<the payload, as one JSON string>", "sig": "<128 hex characters>" }
```

- `sig` is an **Ed25519** signature over the exact UTF-8 bytes of `payload`.
- **Sealed** (2026-11-17, `courier.mjs` whenever `GLIM_PC_ENC_KEY` is set): `{ "v": 2, "id", "iat", "exp", "epk", "n", "ct" }`. `ct` is the v1 envelope above, sealed to the PC's encryption key (X25519 with an ephemeral key derived from the seed and id, HKDF-SHA256, ChaCha20-Poly1305, label `glim-courier-v2 task|<id>`, random 12-byte nonce `n`; `remote/seal.rs`). The PC opens it, checks the outside `id`/`iat`/`exp` equal the signed ones inside (else `bad-seal`), then checks the inner envelope like any other.
- Payload `{ "id", "iat", "exp", "cmd", "args", "rk"? }`:

| Field | Rule |
|---|---|
| `id` | 16-64 characters of `A-Z a-z 0-9 _ -`, unique. Accepted once; seen ids are kept on disk until their expiry has passed. |
| `iat`, `exp` | Unix seconds. `exp - iat` ≤ 72 h (`start-session`, `send-to-session`: ≤ 24 h) and `exp ≥ iat`; 60 s clock skew on `iat`, **none after `exp`** (a task expired at the relay never runs). `courier.mjs` uses 24 h (`--ttl`). |
| `rk` | Optional: the X25519 public key (64 hex) the result is sealed to. Signed, so the relay can't swap it. |
| `cmd`, `args` | One of the six commands. Unknown commands and unknown argument keys are refused. |

| `cmd` | `args` | What the Glim app does (`remote/exec.rs`) |
|---|---|---|
| `session-status` | `{"repo"?}` | Per allowed repo: active (transcript written in the last minute) or idle, the time of the last activity, the session's short id. Read only. |
| `transcript-tail` | `{"repo", "lines"?: 1-50}` (20) | The last N user/Claude messages of the repo's newest transcript (`~/.claude*/projects/<folder>/*.jsonl`), each cut to 400 characters, tool output and Claude Code's wrapped notices left out, redacted like runbook output, 12,000 characters at most. |
| `start-session` | `{"repo", "prompt"?: ≤ 2000}` | With a prompt: `claude -p --output-format text` in the repo folder, the prompt on **stdin** (never the command line), no window; Claude Code's own permission rules apply, so anything that needs approval is refused in that mode. Without one: the keep-alive's fixed `claude remote-control …` in a console, to continue from the Claude app. |
| `send-to-session` | `{"repo", "session", "text": ≤ 4000}` | `session` is the full session id `session-status` shows (a UUID). `claude -p --resume <session> --output-format text` where that session works (the repo folder or one of its worktrees), the text on stdin. Refused as `no-such-session` when it's gone or never existed, `not-your-session` when it's another account's, `busy` if it wrote in the last 30 s (owner 2026-11-17). |
| `runbook` | `{"name"}` | An approved, unchanged runbook without `ask` steps, exactly as `glim.exe --run` runs it (`remote` in runbooks.log). |
| `keepalive-restart` | `{"folder"?}` | Stops and starts again the **Keep Claude Code reachable** sessions (Settings → Remote, `keepalive.rs`): the one whose folder is named `folder` (its last part, e.g. `local-companion`), or all of them. Only folders already on that list, with keep-alive on, each with its fixed command; the Claude desktop app is never touched. Refused as `keepalive-off` when keep-alive is off or has no folders, `keepalive-folder-not-listed` for a name not on the list. At most 3 an hour. Added 2026-11-15 (owner). The result names the account, e.g. `keep-alive: restarted 1 session(s) [Claude2]` (2026-11-17). |

- The repo allowlist is `remote::REPOS` (`local-companion` for now). Its folder is a keep-alive folder (Settings → Remote) of that name, or `%USERPROFILE%\<repo>` with a `.git` folder; never a folder runbooks would refuse. `claude` is started by its full path and never from inside the folder.
- Text arguments may contain no control characters other than line breaks and tabs. They are the signer's own words. A future path that passes on someone else's text must wrap it as quoted material (`ai/quoted.rs`, `Source::Remote`).

### Checks, in order (`remote::Gate::accept`)

1. **Kill switch**: tray "Stop remote tasks" or Settings. While it's on, everything is refused and the connection is closed within ¼ s; nothing reconnects until "Allow again". A task sent while stopped waits at the relay and is handed over when the PC reconnects: Glim refuses every task **issued before "Allow again"** (or before Remote tasks was switched back on; its `iat` before `remote.allowedSince` in settings.json) with a signed `killed` result, so it is acknowledged and never runs (owner 2026-11-14). No clock-skew allowance on this check: a task from a cloud clock running behind, sent right after "Allow again", is refused too; just send it again.
2. **Channel on**: off by default, and it needs its one-time consent card first.
3. **The client is known**: the relay's client name is on Settings' Cloud clients list (else `unknown-client`), and at least one client exists (else `no-key`).
4. **Envelope**: size, JSON, `v` = 1.
5. **Ed25519 signature** with **that client's** public key. One that verifies only with another client's key is refused as `wrong-client`; one that verifies with none, `bad-signature`.
6. **Payload**: id shape, lifetime, not yet valid, expired, then replay.
7. **Command and arguments**, including the repo allowlist.
8. **Rate limits**, per rolling hour: per client 5 `start-session` and 30 commands; on this PC, all clients together, 8 `start-session`, 50 commands and 3 `keepalive-restart`.

Every decision goes to Settings → Remote → **Remote activity** and glim.log: the time, the client, accepted or refused, the command kind or the refusal code. The id appears only once the signature has verified. Never a payload, an argument, a signature, a token or a key.

## Accounts: which Claude account a task acts for (owner, 2026-11-15)

Each cloud client acts for one Claude account on the PC (Settings → Claude Code & Agents → **Accounts**, `accounts.rs`): `account-1` → **Claude** (Claude Code's default config folder, `~\.claude`), `account-2` → **Claude2** (`~\.claude2`), prefilled. The list shows each account's config folder and whether it's signed in: only whether its login file (`.credentials.json`) exists. Glim never opens it.

- `session-status`, `transcript-tail`: only the transcripts in that account's own config folder (the repo's, and its worktrees' under `<repo>\.claude\worktrees\`). Lines name the account (`local-companion: idle … [Claude2]`).
- `start-session`: with that account's `CLAUDE_CONFIG_DIR` (removed for the default one, never inherited). With a prompt, `claude -p -w`: a git worktree of its own. Without one, the remote-control command named `(PC, <account>)` for any account but the default one.
- `send-to-session`: that account's latest session only, continued where it works (the repo folder or one of its worktrees, checked); busy within 30 s.
- `keepalive-restart`: that account's keep-alive servers only.
- A repo whose sessions are only another account's answers `not-your-session`; a client no account acts for gets `not-your-session` for all of these. `runbook` doesn't depend on the account.
- Keep Claude Code reachable runs one remote-control server per account (the default one always, the others once signed in), side by side, each with its own `CLAUDE_CONFIG_DIR` and `--spawn worktree`, so every session it spawns works in its own git worktree.

## Staying connected (owner, 2026-11-15)

2026-11-15 ~11:25 the relay said `{"pc":true,"pending":1}` while a signed task got no answer within 60 s: a half-open line (after sleep or a network change, no close frame ever arrives), which the relay kept counting as the PC.

- **Glim** (`net/relay.rs`) sends `{"t":"ping"}` every 30 s and drops the line when **nothing at all** (pong or frame) has come back for 60 s, then reconnects (backoff 2, 4 … 60 s for lines that keep dying young).
- **Windows events** (`remote/wake.rs`): resume from sleep (`PowerRegisterSuspendResumeNotification`, callback, no window) and an IP address change (`NotifyAddrChange`, one kick per 5 s burst) drop the line and reconnect **at once**, also cutting a backoff wait short; a wall-clock jump does the same if an event is missed. They open no socket.
- **The relay** is a Durable Object with **hibernatable WebSockets**: pings are answered by its **auto-response** (`setWebSocketAutoResponse`), so they neither wake nor bill the Worker. Whenever it is awake anyway (a task queued, a status asked, the PC sending), it reads each PC socket's last ping (`getWebSocketAutoResponseTimestamp`, or the time it connected or last sent a frame, kept in the socket's attachment) and **closes any PC socket silent for 75 s** (code 4001, `core.js` `PC_SILENT_MS`): such a socket gets no task and `status` says `pc: false`. There is no alarm to sweep sockets on its own (it would wake the object every 75 s): the check is lazy, which is all `status` and delivery need.
- **Pending tasks** stay at the relay until answered or expired and are handed over on every connect. The PC runs a task id at most once: an id accepted once stays in the gate's replay cache (on disk) until its expiry, so a redelivered task gets its first result again (same bytes) or `replay` (same id, other bytes), and it never runs twice. A redelivered task passes every check again, the kill switch included: one issued before the last **Allow again** (or before Remote tasks was switched back on) is answered `killed`.
- **Phone** (Settings → Phone on; quiet hours respected): with Remote tasks on and set up, a line down for **10 minutes** while Glim runs pings once ("Glim's remote tasks link has been down for 10 minutes"), and its recovery once ("… is back"); at most one of each per outage, held until quiet hours end. Switching Remote tasks off or stopping them is no outage.

## Result (the Glim app → cloud Glim), signed by the PC

Every task the relay hands over gets one result, accepted or refused:

```json
{ "v": 1, "payload": "<json string>", "sig": "<128 hex>" }
payload: { "v": 1, "id": "<the task's id>", "task": "<SHA-256 hex of the exact task envelope answered>", "at": 1763140003, "ok": true, "kind": "session-status", "detail": "…", "error": "" }
```

- `sig` is Ed25519 over the payload's exact bytes, made with the **PC result key**. The PC sends it to the relay as `{"t":"result","id":…,"env":"<envelope>"}`. That frame answers and deletes the task.
- **Sealed** (2026-11-17): when the verified task named `rk`, the signed envelope goes inside `{"v":2,"id","epk","n","ct"}`, sealed to `rk` with a fresh ephemeral key (label `glim-courier-v2 result|<id>`). Refusals of tasks that didn't verify stay unsealed: their `rk` can't be trusted, and they carry only a fixed code. A sealed result too big for the relay answers `result-too-big`; an `rk` that can't be sealed to, `bad-result-key`. cloud Glim refuses an unsealed **success** for a task that named `rk` (no downgrade).
- `ok: false` carries a fixed `error` code: a refusal (`bad-signature`, `expired`, `replay`, `rate-limited`, `killed` …) or a failure (`no-folder`, `no-claude`, `busy`, `no-transcript`, `runbook-not-approved`, `runbook-failed-N-of-M` …).
- **cloud Glim must verify** (`courier.mjs verifyFull`): its **own** signature on the task envelope (the one it sent, or the relay's copy from `with=task`, re-opened if sealed), the result's seal, then the signature with `GLIM_PC_PUBLIC_KEY`, the `id` it asked about, `task` = SHA-256 of that envelope, and `at` within the task's lifetime. The `task` hash matters because the PC signs refusals of envelopes it couldn't verify too: without it, the relay could make up a look-alike with a real task's id and pass off the PC's refusal of it as the answer to the real task. A result failing any of these is refused (exit 2, `bad-result-signature` / `bad-result`).
- A task delivered twice (after a reconnect) gets the same result again, without running a second time.

Both directions are pinned by shared test vectors: `relay/test/relay.test.mjs` and `remote/courier_tests.rs` check the same bytes, using RFC 8032's published test seeds.

## Connection (the Glim app)

- Only while Remote tasks are on, consented, not stopped, a signer key and a relay address are saved, and the relay is set up. `net::relay::check` refuses anything but `wss://<name>.<subdomain>.workers.dev/v1/pc` on 443, the address typed in Settings, and only while allowed (`set_relay_allowed`).
- TLS uses rustls (ring) with the webpki roots.
- Pings every 25 s. When a pong is 10 s late, the socket closes, or the wall clock jumps (the PC slept), it reconnects at once, and then after 2, 4 … 60 s if connections keep dying within 5 s. Results that couldn't be sent wait and go first on the next connection. Settings shows the state (Connected / Reconnecting… / The relay refused this PC's token …).

## Setup

The exact owner steps are in HANDOFF (local). In order:

1. Glim: Settings → Remote → Remote tasks → **Set up the relay**. Copy the PC result key.
2. Make the task-signing key: `node relay/courier.mjs keygen`. It writes the seed to `%LOCALAPPDATA%\Glim\task-signing-key.txt` (it refuses OneDrive, Desktop and Documents, which OneDrive may sync, and an existing file). In Glim, Cloud clients: name (`account-1`, filled in) + the printed public key → **Add cloud client**. Copy the cloud token shown once.
3. Run the local runbook `relay-tokens` (approve it in Settings → Runbooks first). It pipes the hashes to `wrangler secret put PC_TOKEN_SHA256 / CLOUD_CLIENTS` on stdin.
4. Give cloud Glim the cloud token, preferably as a **network secret** in the claude.ai/code environment: host = the relay's **exact** host (`task-relay.<subdomain>.workers.dev`, never `*.workers.dev`, which would hand the token to every Worker on Cloudflare), path `/v1/`, header `Authorization: Bearer <token>`. The agent proxy adds the header on the way out, so the session never holds the token. `courier.mjs` then sends no Authorization header of its own (`RELAY_CLOUD_TOKEN` unset). Fallback: the environment variable `RELAY_CLOUD_TOKEN`; when set, `courier.mjs` sends it as the bearer token. A 401 with no token sent from the session prints a hint to check the network secret.
5. In cloud Glim's environment variables, set `RELAY_URL`, `GLIM_TASK_SIGNING_KEY` (the key file's contents; then **delete** `%LOCALAPPDATA%\Glim\task-signing-key.txt`), `GLIM_PC_PUBLIC_KEY`, `GLIM_PC_ENC_KEY` (Settings → Remote → "This PC's encryption key", Copy; public) and **`NODE_USE_ENV_PROXY=1`**. Without `GLIM_PC_ENC_KEY`, tasks go unsealed (with a warning). Without that last one, Node's built-in `fetch` ignores the agent proxy (`HTTPS_PROXY`), so requests skip the proxy and the network secret is never added: a 401, or no route to the relay at all (owner, 2026-11-14: the cloud runs worked only once it was set). Its network access must allow the relay's exact host.
6. Glim: Relay address → Save address; Accept remote tasks → on (consent card). The state should read Connected.
7. From cloud Glim: `node relay/courier.mjs send session-status` should print a verified result. Then test the kill switch: tray → Stop remote tasks. The next `send` exits 3 (no result), and Remote activity shows the stop.

### Another Claude account (a second cloud client)

Each Claude account's cloud sessions get their own client: their own relay token and task-signing key, so one can be revoked without touching the other, and each is rate-limited on its own.

1. On the PC: `node relay/courier.mjs keygen <file>` (a new file, not the first account's; same folder rules), note the printed public key.
2. Glim: Settings → Remote → Remote tasks → Cloud clients: name `account-2` (filled in) + that public key → **Add cloud client**. Copy the token shown once.
3. Run the `relay-tokens` runbook: the relay now takes both tokens.
4. In the second account's claude.ai/code: a new environment for this repo with the same `RELAY_URL`, `GLIM_PC_PUBLIC_KEY`, `GLIM_PC_ENC_KEY` and `NODE_USE_ENV_PROXY=1` as the first, **its own** `GLIM_TASK_SIGNING_KEY` (step 1's file; then delete the file) and **its own** network secret (the token from step 2; exact relay host, path `/v1/`, `Authorization: Bearer <token>`).
5. From a session there: `node relay/courier.mjs status`, then `send session-status`. Remote activity shows `account-2 · Accepted: session-status`.

Lesson (2026-11-17): the **relay token** goes in the environment's **network secret**; the **signing key file**'s contents go in `GLIM_TASK_SIGNING_KEY`, and that key's public half must be the one Settings shows for that client (compare fingerprints). Glim can't change a client's key without a new token, so never generate a fresh key for an existing client.

**Remove** in Cloud clients refuses that client's tasks at once; run `relay-tokens` again so the relay stops taking its token as well. The other clients keep working.
