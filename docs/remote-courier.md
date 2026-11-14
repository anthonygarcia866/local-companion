# Remote courier: task and result formats (draft, 2026-11-14)

Status: the **security core** is built (`windows/src-tauri/src/remote/`) and is **not wired to any transport**. Nothing in Glim receives tasks yet. The transport (a relay the PC connects out to) waits on the owner's choice; the recommendation is in the owner's local notes, not in this repo.

Naming: **cloud Glim** is the agent (a cloud Claude session acting for the owner) that signs and sends tasks. **The Glim app** is the Windows app that checks them and acts.

## Envelope (what the transport carries)

```json
{ "v": 1, "payload": "<the payload, as one JSON string>", "sig": "<128 hex characters>" }
```

- `sig` is an **Ed25519** signature over the exact UTF-8 bytes of `payload`. The string is signed as sent, so there is nothing to canonicalize.
- At most 16 KB in all.
- The Glim app stores only the signer's **public key**: 32 bytes as 64 hex characters, pasted in Settings → Remote → Remote tasks, with its fingerprint shown (the first 16 bytes of SHA-256(key), as 8 groups of 4 hex). The private key never touches the PC.

## Payload

```json
{ "id": "task-2026-11-14-0001", "iat": 1763140000, "exp": 1763140300, "cmd": "session-status", "args": {} }
```

| Field | Rule |
|---|---|
| `id` | 16-64 characters of `A-Z a-z 0-9 _ -`, unique. Accepted once; seen ids are kept on disk until their expiry has passed. |
| `iat`, `exp` | Unix seconds. `exp - iat` ≤ 600 (10 minutes) and `exp ≥ iat`. 60 s clock skew is allowed on each side. |
| `cmd`, `args` | One of the commands below. Unknown commands and unknown argument keys are refused. |

| `cmd` | `args` |
|---|---|
| `session-status` | `{"repo"?: name}` |
| `transcript-tail` | `{"repo": name, "lines"?: 1-50}` (default 20) |
| `start-session` | `{"repo": name, "prompt"?: 1-2000 characters}` |
| `send-to-session` | `{"repo": name, "text": 1-4000 characters}` |
| `runbook` | `{"name": runbook name}`: approved, non-asking runbooks only, as `glim.exe --run` allows |

- Repos come from an allowlist (`remote::REPOS`). For now that is `local-companion` only.
- Text arguments may contain no control characters other than line breaks and tabs.
- A transport that passes on text the owner didn't write must wrap it as quoted material (`ai/quoted.rs`, `Source::Remote`).

## Checks, in order (`remote::Gate::accept`)

1. **Kill switch**: one click in the tray ("Stop remote tasks") or Settings. While it's on, everything is refused until "Allow again" in Settings.
2. **Channel on**: off by default, and it needs its one-time consent card first.
3. **A key is saved.**
4. **Envelope**: size, JSON, `v` = 1.
5. **Ed25519 signature.**
6. **Payload**: the id's shape, lifetime, not yet valid, expired, then replay.
7. **Command and arguments**, including the repo allowlist.
8. **Rate limits**, per rolling hour: at most 5 `start-session` and 30 commands in all (counted on accepted tasks).

Every decision is recorded in Settings → Remote → **Remote activity** and in glim.log. An entry has the time, accepted or refused, the command kind or the refusal code (`expired`, `bad-signature`, `replay`, `rate-limited` …), and the id only once the signature verified. It never holds a payload, an argument, a signature or a key. The state file is `%LOCALAPPDATA%\Glim\remote-state.json` (seen ids, recent times, the last 50 entries).

## Result (Glim app → cloud Glim), proposed for the transport PR

```json
{ "v": 1, "id": "<the task's id>", "at": 1763140003, "ok": true, "kind": "session-status", "detail": "…", "error": "" }
```

`detail` is short and command-specific (a status line, the last N transcript lines). `error` is a refusal or failure code. Results are to be signed by the Glim app with its own Ed25519 key (kept in Credential Manager) so the relay can't forge them. That key and the result signing come with the transport.
