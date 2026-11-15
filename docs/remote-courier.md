# Remote courier: relay, tasks and results (2026-11-14)

Status: **wired** (owner approved transport (a), a Cloudflare Worker + Durable Object relay, 2026-11-14). The security core is `windows/src-tauri/src/remote/`, the PC's connection is `remote/courier.rs` + `net/relay.rs`, and the five commands live in `remote/exec.rs`. The relay is `relay/` (Worker `task-relay`, Workers Free plan). cloud Glim's client is `relay/courier.mjs`.

Naming: **cloud Glim** is the agent (a cloud Claude session acting for the owner) that signs and sends tasks. **The Glim app** is the Windows app that checks them, acts, and signs results.

```
cloud Glim ──POST /v1/tasks (cloud token)──▶  relay (Worker + DO)  ◀──wss /v1/pc (PC token)── the Glim app
           ◀─GET /v1/results/<id> ──────────  keeps each envelope       (connects out; nothing
              (signed by the PC)               until answered or          listens on the PC)
                                               expired, ≤ 10 min
```

## Who holds what

| Secret / key | Made by | Kept | Never |
|---|---|---|---|
| Task-signing key (Ed25519 private) | `node relay/courier.mjs keygen` (writes `%LOCALAPPDATA%\Glim\task-signing-key.txt`, never a OneDrive-synced folder such as Desktop or Documents; delete it after copying) | cloud Glim's environment, `GLIM_TASK_SIGNING_KEY` | on the PC once copied (Glim stores only its public key), in the repo or chat |
| Task-signing public key | the same | Glim: Settings → Remote → Remote tasks (fingerprint shown) | n/a (public) |
| PC result key (Ed25519 private) | Glim, "Set up the relay" | Credential Manager `Glim/RemotePcSigningKey` (PKCS#8) | in a file, a log, the page or the network |
| PC result public key | the same | Settings (shown with its fingerprint); cloud Glim's `GLIM_PC_PUBLIC_KEY` | n/a (public) |
| PC relay token (32 random bytes, hex) | the same | Credential Manager `Glim/RemoteRelayPcToken`; only sent as the WebSocket upgrade's `Authorization: Bearer` | in a URL, a log or the page |
| Cloud relay token (32 random bytes, hex) | the same | shown **once** in Settings, then only in a claude.ai/code **network secret** (recommended, see below) or, as a fallback, cloud Glim's environment variable `RELAY_CLOUD_TOKEN`; Glim forgets it | on the PC after that, in the repo or chat |
| Token hashes (SHA-256 hex) | the same | Settings → Secrets `REMOTE_PC_TOKEN_SHA256`, `REMOTE_CLOUD_TOKEN_SHA256` → Worker secrets `PC_TOKEN_SHA256`, `CLOUD_TOKEN_SHA256` (an approved runbook pipes them to `wrangler secret put` on stdin) | n/a (a hash of 32 random bytes reveals nothing) |

The relay holds only the two hashes: it tells the PC and cloud Glim apart, and each token works only on its own side's routes. A stolen cloud token can queue tasks, but every task still needs the task-signing key. A stolen PC token can read the queue, but it can't produce results cloud Glim accepts without the PC result key.

## The relay (`relay/`)

- Worker `task-relay` on `*.workers.dev`, Workers Free plan, one SQLite-backed Durable Object (`Mailbox`). There are no cron triggers: purging uses a DO alarm, which doesn't count against the account's cron ceiling.
- `GET /v1/pc` (PC token): a hibernatable WebSocket. On connect, every pending task arrives at once; new ones arrive as they're queued. A new connection replaces the old one. The relay answers `{"t":"ping"}` with `{"t":"pong"}` without waking up.
- `POST /v1/tasks` (cloud token): one envelope, ≤ 16 KB. It is refused when malformed, expired, valid for more than 10 minutes, a duplicate id, or when 50 are already pending.
- `GET /v1/results/<id>?wait=1-25` (cloud token): hands the PC's signed result over **once**, then deletes it. 202 = still pending, 404 = unknown or gone.
- `GET /v1/status` (cloud token): `{"pc": connected?, "pending": n}`.
- What is stored: a task until its result arrives or its expiry (+60 s); a result until it's fetched or the task's expiry. Nothing else. No `console.*` (a test checks), and Workers Logs/observability are off in `wrangler.toml`. The relay checks neither signature; the two ends do.

## Task (cloud Glim → the Glim app)

Envelope, as carried:

```json
{ "v": 1, "payload": "<the payload, as one JSON string>", "sig": "<128 hex characters>" }
```

- `sig` is an **Ed25519** signature over the exact UTF-8 bytes of `payload`.
- Payload `{ "id", "iat", "exp", "cmd", "args" }`:

| Field | Rule |
|---|---|
| `id` | 16-64 characters of `A-Z a-z 0-9 _ -`, unique. Accepted once; seen ids are kept on disk until their expiry has passed. |
| `iat`, `exp` | Unix seconds. `exp - iat` ≤ 600 and `exp ≥ iat`; 60 s clock skew each side. `courier.mjs` uses 300 s. |
| `cmd`, `args` | One of the five commands. Unknown commands and unknown argument keys are refused. |

| `cmd` | `args` | What the Glim app does (`remote/exec.rs`) |
|---|---|---|
| `session-status` | `{"repo"?}` | Per allowed repo: active (transcript written in the last minute) or idle, the time of the last activity, the session's short id. Read only. |
| `transcript-tail` | `{"repo", "lines"?: 1-50}` (20) | The last N user/Claude messages of the repo's newest transcript (`~/.claude*/projects/<folder>/*.jsonl`), each cut to 400 characters, tool output and Claude Code's wrapped notices left out, redacted like runbook output, 12,000 characters at most. |
| `start-session` | `{"repo", "prompt"?: ≤ 2000}` | With a prompt: `claude -p --output-format text` in the repo folder, the prompt on **stdin** (never the command line), no window; Claude Code's own permission rules apply, so anything that needs approval is refused in that mode. Without one: the keep-alive's fixed `claude remote-control …` in a console, to continue from the Claude app. |
| `send-to-session` | `{"repo", "text": ≤ 4000}` | `claude -p --continue --output-format text` in the repo folder, the text on stdin. Refused (`busy`) if a session there wrote in the last 30 s. |
| `runbook` | `{"name"}` | An approved, unchanged runbook without `ask` steps, exactly as `glim.exe --run` runs it (`remote` in runbooks.log). |

- The repo allowlist is `remote::REPOS` (`local-companion` for now). Its folder is a keep-alive folder (Settings → Remote) of that name, or `%USERPROFILE%\<repo>` with a `.git` folder; never a folder runbooks would refuse. `claude` is started by its full path and never from inside the folder.
- Text arguments may contain no control characters other than line breaks and tabs. They are the signer's own words. A future path that passes on someone else's text must wrap it as quoted material (`ai/quoted.rs`, `Source::Remote`).

### Checks, in order (`remote::Gate::accept`)

1. **Kill switch**: tray "Stop remote tasks" or Settings. While it's on, everything is refused and the connection is closed within ¼ s; nothing reconnects until "Allow again". A task sent while stopped waits at the relay and is handed over when the PC reconnects: Glim refuses every task **issued before "Allow again"** (or before Remote tasks was switched back on; its `iat` before `remote.allowedSince` in settings.json) with a signed `killed` result, so it is acknowledged and never runs (owner 2026-11-14). No clock-skew allowance on this check: a task from a cloud clock running behind, sent right after "Allow again", is refused too; just send it again.
2. **Channel on**: off by default, and it needs its one-time consent card first.
3. **A signer key is saved.**
4. **Envelope**: size, JSON, `v` = 1.
5. **Ed25519 signature** with the saved public key.
6. **Payload**: id shape, lifetime, not yet valid, expired, then replay.
7. **Command and arguments**, including the repo allowlist.
8. **Rate limits**, per rolling hour: 5 `start-session`, 30 commands in all.

Every decision goes to Settings → Remote → **Remote activity** and glim.log: the time, accepted or refused, the command kind or the refusal code. The id appears only once the signature has verified. Never a payload, an argument, a signature, a token or a key.

## Result (the Glim app → cloud Glim), signed by the PC

Every task the relay hands over gets one result, accepted or refused:

```json
{ "v": 1, "payload": "<json string>", "sig": "<128 hex>" }
payload: { "v": 1, "id": "<the task's id>", "task": "<SHA-256 hex of the exact task envelope answered>", "at": 1763140003, "ok": true, "kind": "session-status", "detail": "…", "error": "" }
```

- `sig` is Ed25519 over the payload's exact bytes, made with the **PC result key**. The PC sends it to the relay as `{"t":"result","id":…,"env":"<envelope>"}`. That frame answers and deletes the task.
- `ok: false` carries a fixed `error` code: a refusal (`bad-signature`, `expired`, `replay`, `rate-limited`, `killed` …) or a failure (`no-folder`, `no-claude`, `busy`, `no-transcript`, `runbook-not-approved`, `runbook-failed-N-of-M` …).
- **cloud Glim must verify** (`courier.mjs verifyResult`): the signature with `GLIM_PC_PUBLIC_KEY`, the `id` it asked about, `task` = SHA-256 of the envelope it sent, and `at` within the task's lifetime. The `task` hash matters because the PC signs refusals of envelopes it couldn't verify too: without it, the relay could make up a look-alike with a real task's id and pass off the PC's refusal of it as the answer to the real task. A result failing any of these is refused (exit 2, `bad-result-signature` / `bad-result`).
- A task delivered twice (after a reconnect) gets the same result again, without running a second time.

Both directions are pinned by shared test vectors: `relay/test/relay.test.mjs` and `remote/courier_tests.rs` check the same bytes, using RFC 8032's published test seeds.

## Connection (the Glim app)

- Only while Remote tasks are on, consented, not stopped, a signer key and a relay address are saved, and the relay is set up. `net::relay::check` refuses anything but `wss://<name>.<subdomain>.workers.dev/v1/pc` on 443, the address typed in Settings, and only while allowed (`set_relay_allowed`).
- TLS uses rustls (ring) with the webpki roots.
- Pings every 25 s. When a pong is 10 s late, the socket closes, or the wall clock jumps (the PC slept), it reconnects at once, and then after 2, 4 … 60 s if connections keep dying within 5 s. Results that couldn't be sent wait and go first on the next connection. Settings shows the state (Connected / Reconnecting… / The relay refused this PC's token …).

## Setup

The exact owner steps are in HANDOFF (local). In order:

1. Glim: Settings → Remote → Remote tasks → **Set up the relay**. Copy the cloud token shown once. Copy the PC result key.
2. Run the local runbook `relay-tokens` (approve it in Settings → Runbooks first). It pipes the two hashes to `wrangler secret put PC_TOKEN_SHA256 / CLOUD_TOKEN_SHA256` on stdin.
3. Make the task-signing key: `node relay/courier.mjs keygen`. It writes the seed to `%LOCALAPPDATA%\Glim\task-signing-key.txt` (it refuses OneDrive, Desktop and Documents, which OneDrive may sync, and an existing file). Paste the printed public key in Glim (Signer's public key → Save key).
4. Give cloud Glim the cloud token, preferably as a **network secret** in the claude.ai/code environment: host = the relay's **exact** host (`task-relay.<subdomain>.workers.dev`, never `*.workers.dev`, which would hand the token to every Worker on Cloudflare), path `/v1/`, header `Authorization: Bearer <token>`. The agent proxy adds the header on the way out, so the session never holds the token. `courier.mjs` then sends no Authorization header of its own (`RELAY_CLOUD_TOKEN` unset). Fallback: the environment variable `RELAY_CLOUD_TOKEN`; when set, `courier.mjs` sends it as the bearer token. A 401 with no token sent from the session prints a hint to check the network secret.
5. In cloud Glim's environment variables, set `RELAY_URL`, `GLIM_TASK_SIGNING_KEY` (the key file's contents; then **delete** `%LOCALAPPDATA%\Glim\task-signing-key.txt`), `GLIM_PC_PUBLIC_KEY` and **`NODE_USE_ENV_PROXY=1`**. Without that last one, Node's built-in `fetch` ignores the agent proxy (`HTTPS_PROXY`), so requests skip the proxy and the network secret is never added: a 401, or no route to the relay at all (owner, 2026-11-14: the cloud runs worked only once it was set). Its network access must allow the relay's exact host.
6. Glim: Relay address → Save address; Accept remote tasks → on (consent card). The state should read Connected.
7. From cloud Glim: `node relay/courier.mjs send session-status` should print a verified result. Then test the kill switch: tray → Stop remote tasks. The next `send` exits 3 (no result), and Remote activity shows the stop.
