# Phone notifications (phone companion, step 1)

Status: design 2026-10-18 (owner's brief, relayed in chat), built the same day as a draft PR behind a setting that is **off by default**. Nothing has been sent to a real phone yet: tests use a fake server only.

## 1. What it does

A one-way push to the owner's phone when something on the PC needs him and he isn't at it:

| Event | When | Default |
|---|---|---|
| Approval waiting | a permission card (or a question card) has waited **N s** without an answer; N = 60 by default, 15–100 (§5) | on (once the feature is on) |
| Session finished | an agent session's turn ended (Claude Code `Stop`, the other agents' equivalent) | on |
| Limit hit | a usage or spend limit stopped a session (`StopFailure` with `rate_limit` / `billing_error`, or the limit notice) | on |
| Next meeting | 10 min before the next Outlook event | not built: needs the calendar glance (#34) merged first; the setting is listed but disabled until then |

One-way only: the phone can't answer, approve or reach the PC. (Answering from the phone is a later step with its own design.)

## 2. What a notification says

Short status only, built in Rust from **fixed templates**. The island only sends Rust an event kind, an agent name and a project name; Rust picks the template, so no code path can put other text in a notification.

- Approval: `Claude Code in local-companion needs approval`
- Finished: `Claude Code in local-companion finished`
- Limit: `Claude Code in local-companion hit a usage limit` / `… a spend limit`
- Test: `Test notification from Glim`
- Title: `Glim`

**Never** in a notification: commands, code, file names or paths, prompts, Claude's answers, captured or typed text, window titles, meeting titles.

- Agent name: one of a fixed list (Claude Code, Codex, Copilot CLI, Muse Code, Hermes, Gemini CLI, Cursor, an agent's own display name from `agents.ts`); anything else becomes "An agent".
- Project name: the pill's name, which is the session folder's last segment (e.g. `local-companion`). It is cut to 40 characters, control characters and path separators are dropped, and it can be switched off ("Include the project name", on by default; off gives `Claude Code needs approval`). This is the one piece of PC-side information that leaves the machine; see decision 3.

## 3. The service: Pushover (recommended)

| | **Pushover** | **ntfy.sh** (hosted) | **self-hosted ntfy** | **Telegram bot** |
|---|---|---|---|---|
| Cost | $4.99 once for the iPhone app (30-day free trial); 10,000 messages/month free per account | free; paid plans (about $5–6/month and up) add reserved topics and access control | free software; you pay for a server that is always on | free |
| Privacy | messages go to your account only (user key + app token); Pushover and Apple's push service carry them | free topics are **public by name**: anyone who guesses or learns the topic can read and send. Private only on a paid plan with access control | your server holds the messages, **but on iPhone** it still sends a wake-up to ntfy.sh (topic and message id, not the text) for instant delivery | Telegram's servers store the chat; bot chats are not end-to-end encrypted; needs a Telegram account |
| Setup | install the app, create an account, register an "application" on pushover.net, paste two keys into Glim | install the app, pick a long random topic, paste it into Glim | a public server with TLS, plus the above | create a bot with BotFather, message it once, find your chat id, paste token + id |
| iPhone | first-class app; priorities, quiet hours and sounds on the phone | app exists; instant delivery goes through ntfy.sh's push relay | as hosted, plus the upstream relay setting | Telegram app; works well |
| New host for Glim | `api.pushover.net` | `ntfy.sh` | the owner's own domain | `api.telegram.org` |

**Recommendation: Pushover.** It is the only option that is private by default without a subscription or a server (ntfy.sh's free topics are public by name; self-hosting still relays through ntfy.sh on iPhone; Telegram keeps the chat on its servers and needs an account). It costs $4.99 once, has a reliable iPhone app, and the API is one HTTPS POST. Runner-up: ntfy.sh with a long random topic, if paying nothing matters more than privacy by default (the content is short status only either way).

Sources checked 2026-10-18: pushover.net/pricing and support.pushover.net (limits; per-account since May 2026); docs.ntfy.sh terms (free topics public, reserved topics and access control paid) and self-hosting guides (iOS upstream poll via ntfy.sh); Telegram bot privacy write-ups (bot chats not end-to-end encrypted). Prices change; check before buying.

## 4. Network: the choke point

`net/mod.rs` gets one more host, like Anthropic's in 1c:

- `PUSH_HOST = "api.pushover.net"`, **https on 443 only**, exact name, no subdomains, no redirects to another host (the existing `hop_allowed` rule).
- **Allowed only while the setting is on.** `net::check` refuses `api.pushover.net` unless phone notifications are switched on (an atomic flag set when settings load and save). With the feature off (the default) Glim's allowlist is what it was: this computer and `api.anthropic.com`. A test checks both states.
- One request type: `POST https://api.pushover.net/1/messages.json`, form fields `token`, `user`, `title`, `message`, `priority` (0; quiet hours are Glim's own, see §5). Nothing else is ever sent.
- The answer is read with the usual ceiling. The log line says `phone: sent <kind>` or `phone: failed <kind> (<status or short reason>)`: never the text, the keys or the project.
- Keys: the Pushover **user key** and **application token** are kept in Windows Credential Manager (`Glim/PushoverKeys`, this user on this PC, like the Anthropic key), never in settings.json, never logged, never sent to the page; Settings sees only "set" and a hint (last four characters of each).
- CLAUDE.md's privacy paragraph and the Settings → About line get the new host, with this note.

## 5. When it sends (rules)

All in one small pure module (`src/core/phone.ts`), tested in Node:

- **Master switch off** (default): nothing, ever. Rust checks the setting again before sending, and the choke point refuses the host.
- **Approval:** once per request, when it has waited `approvalAfterSec` (default 60). A request answered, withdrawn or timed out before that never pings. The relay hands a card to the terminal after ~108 s, so delays above 100 s mean "only ever a ping for requests that are already back in the terminal"; Settings caps the field at 100 s for that reason (decision 4).
- **Finished:** on each `Stop` of a session. Not when an approval for that session is still waiting (that one gets its own ping).
- **Limit:** once per limit (per pill and reset time).
- **Quiet hours** (optional, off by default; e.g. 22:00–07:00, can cross midnight): nothing is sent inside them; nothing is queued for later either (a stale "needs approval" at 7 am would be wrong).
- **Rate limit:** at most one notification every 10 s and 30 per hour in total; extra events are dropped and counted in the log. This protects the phone and the 10,000/month allowance.
- **Only when away?** Not in step 1: Glim doesn't know whether the owner is at the PC. The approval delay is the stand-in (decision 5).

## 6. Settings → Phone (new section)

- **Send notifications to my phone** — off by default.
- **Pushover keys** — user key and app token fields (masked once saved, Remove), a link to pushover.net to make them.
- **Notify me when:** an approval waits [60] s · a session finishes · a usage or spend limit is hit · my next meeting is in 10 min (disabled, "needs the calendar glance").
- **Include the project name** — on.
- **Quiet hours** — off; from [22:00] to [07:00].
- **Send test notification** — sends the fixed test text; shows "Sent" or the error. Works only with the switch on and keys saved.

Settings keys: one `phone` object in settings.json: `{ enabled, approval, approvalAfterSec, finished, limit, meeting, includeProject, quietHours, quietFrom, quietTo }`.

## 7. Tests (no real notifications)

- Rust: the message templates (every kind; unknown agent; project sanitising; project off); the choke point (host refused while off, allowed while on, http and other ports and subdomains refused); the send against `serve_once` on 127.0.0.1 (the form fields, nothing else; an error status becomes a `failed` log line without the text); keys never in Debug output, settings.json or the log (source test); settings round trip and defaults.
- JS: `core/phone.ts` rules (delay, once per request, answered-before-delay, finished, limit once, quiet hours across midnight, rate limit); the island wiring (tick, Stop, StopFailure call it); the Settings section (off by default, fields, test button calls the bridge).
- No test talks to api.pushover.net. The live check is the owner's: buy the app, make the keys, press **Send test notification**.

## 8. Not in step 1

Answering from the phone, any phone app of our own, rich notifications (buttons, deep links), the meeting reminder (after #34), "only when I'm away" detection, other services (the design keeps one `Provider` so ntfy or Telegram can be added behind the same rules if the owner picks one).
