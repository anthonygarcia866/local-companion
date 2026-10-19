# Phone notifications (phone companion, step 1)

Status: design 2026-10-18 (owner's brief, relayed in chat), built the same day as a draft PR (#38) behind a setting that is **off by default**. Owner's decisions the same afternoon in §0. Nothing has been sent to a real phone yet: tests use a fake server only. The first live check is the owner's, on a retest build: real Pushover keys in Settings → Phone, then **Send test notification**.

## 0. Decisions (owner, 2026-10-18)

1. **Service: Pushover.** The owner installed the app and set up an account.
2. **Include the project name:** yes (on by default; a switch can turn it off).
3. **Approval delay: 60 s** (the default; 15–100 s in Settings).
4. **Only when I'm away:** send only when this PC has had no keyboard or mouse input for **2 minutes** or more. A setting: on by default, 2 min, 1–60 min, can be turned off (§5). Plus **quiet hours** (already in, off by default).
5. **Step 2 (approve or deny from the phone)** needs a two-way channel: options recorded in §9, **not built**.

## 1. What it does

A one-way push to the owner's phone when something on the PC needs him and he isn't at it:

| Event | When | Default |
|---|---|---|
| Approval waiting | a permission card (or a question card) has waited **N s** without an answer and you are away from the PC (§5); N = 60 by default, 15–100 | on (once the feature is on) |
| Session finished | an agent session's turn ended (Claude Code `Stop`, the other agents' equivalent) | on |
| Limit hit | a usage or spend limit stopped a session (`StopFailure` with `rate_limit` / `billing_error`, or the limit notice) | on |
| Next meeting | 10 min before the next Outlook event | not built: needs the calendar glance (#34) merged first; the setting is listed but disabled until then |

One-way only: the phone can't answer, approve or reach the PC. (Answering from the phone is step 2: options in §9.)

## 2. What a notification says

Short status only, built in Rust from **fixed templates**. The island only sends Rust an event kind, an agent name and a project name; Rust picks the template, so no code path can put other text in a notification.

- Approval: `Claude Code in local-companion needs approval`
- Finished: `Claude Code in local-companion finished`
- Limit: `Claude Code in local-companion hit a usage limit` / `… a spend limit`
- Test: `Test notification from Glim`
- Title: `Glim`
- The templates are English, whatever Glim's language: short, fixed and easy to audit (the Settings page around them is translated).

**Never** in a notification: commands, code, file names or paths, prompts, Claude's answers, captured or typed text, window titles, meeting titles.

- Agent name: one of a fixed list (Claude Code, Codex, Copilot CLI, Muse Code, Hermes, Gemini CLI, Cursor, an agent's own display name from `agents.ts`); anything else becomes "An agent".
- Project name: the pill's name, which is the session folder's last segment (e.g. `local-companion`). It is cut to 40 characters, control characters and path separators are dropped, and it can be switched off ("Include the project name", on by default; off gives `Claude Code needs approval`). This is the one piece of PC-side information that leaves the machine; kept on by the owner (decision 2).

## 3. The service: Pushover (decided)

| | **Pushover** | **ntfy.sh** (hosted) | **self-hosted ntfy** | **Telegram bot** |
|---|---|---|---|---|
| Cost | $4.99 once for the iPhone app (30-day free trial); 10,000 messages/month free per account | free; paid plans (about $5–6/month and up) add reserved topics and access control | free software; you pay for a server that is always on | free |
| Privacy | messages go to your account only (user key + app token); Pushover and Apple's push service carry them | free topics are **public by name**: anyone who guesses or learns the topic can read and send. Private only on a paid plan with access control | your server holds the messages, **but on iPhone** it still sends a wake-up to ntfy.sh (topic and message id, not the text) for instant delivery | Telegram's servers store the chat; bot chats are not end-to-end encrypted; needs a Telegram account |
| Setup | install the app, create an account, register an "application" on pushover.net, paste two keys into Glim | install the app, pick a long random topic, paste it into Glim | a public server with TLS, plus the above | create a bot with BotFather, message it once, find your chat id, paste token + id |
| iPhone | first-class app; priorities, quiet hours and sounds on the phone | app exists; instant delivery goes through ntfy.sh's push relay | as hosted, plus the upstream relay setting | Telegram app; works well |
| New host for Glim | `api.pushover.net` | `ntfy.sh` | the owner's own domain | `api.telegram.org` |

**Recommendation: Pushover** (the owner's pick, decision 1). It is the only option that is private by default without a subscription or a server (ntfy.sh's free topics are public by name; self-hosting still relays through ntfy.sh on iPhone; Telegram keeps the chat on its servers and needs an account). It costs $4.99 once, has a reliable iPhone app, and the API is one HTTPS POST. Runner-up: ntfy.sh with a long random topic, if paying nothing matters more than privacy by default (the content is short status only either way).

Sources checked 2026-10-18: pushover.net/pricing and support.pushover.net (limits; per-account since May 2026); docs.ntfy.sh terms (free topics public, reserved topics and access control paid) and self-hosting guides (iOS upstream poll via ntfy.sh); Telegram bot privacy write-ups (bot chats not end-to-end encrypted). Prices change; check before buying.

## 4. Network: the choke point

`net/mod.rs` gets one more host, like Anthropic's in 1c:

- `PUSH_HOST = "api.pushover.net"`, **https on 443 only**, exact name, no subdomains, no redirects to another host (the existing `hop_allowed` rule).
- **Allowed only while the setting is on.** `net::check` refuses `api.pushover.net` unless phone notifications are switched on (an atomic flag set when settings load and save). With the feature off (the default) Glim's allowlist is what it was: this computer and `api.anthropic.com`. A test checks both states.
- One request type: `POST https://api.pushover.net/1/messages.json`, a JSON body with exactly `token`, `user`, `title`, `message`, `priority` (0; quiet hours are Glim's own, see §5). Nothing else is ever sent.
- The answer is read with the usual ceiling. The log line says `phone: sent <kind>` or `phone: failed <kind> (<status or short reason>)`: never the text, the keys or the project.
- Keys: the Pushover **user key** and **application token** are kept in Windows Credential Manager (`Glim/PushoverKeys`, this user on this PC, like the Anthropic key), never in settings.json, never logged, never sent to the page; Settings sees only "set" and a hint (last four characters of each).
- CLAUDE.md's privacy paragraph and the Settings → About line get the new host, with this note.

## 5. When it sends (rules)

All in one small pure module (`src/core/phone.ts`), tested in Node:

- **Master switch off** (default): nothing, ever. Rust checks the setting again before sending, and the choke point refuses the host.
- **Approval:** once per request, when it has waited `approvalAfterSec` (default 60) **and** you are away (below). A request answered, withdrawn or timed out before that never pings. The relay hands a card to the terminal after ~108 s, so delays above 100 s mean "only ever a ping for requests that are already back in the terminal"; Settings caps the field at 100 s for that reason.
- **Finished:** on each `Stop` of a session. Not when an approval for that session is still waiting (that one gets its own ping).
- **Limit:** once per limit (per pill and reset time).
- **Quiet hours** (optional, off by default; e.g. 22:00–07:00, can cross midnight): nothing is sent inside them; nothing is queued for later either (a stale "needs approval" at 7 am would be wrong).
- **Rate limit:** at most one notification every 10 s and 30 per hour in total; extra events are dropped and counted in the log. This protects the phone and the 10,000/month allowance.
- **Only when I'm away** (decision 4; `idleOnly`, on by default, `idleAfterMin` 2, 1–60): nothing is sent unless the PC has had no keyboard or mouse input for that long. Rust reads it with Windows' `GetLastInputInfo` (`phone_idle_ms`, a number of ms, nothing else); the island asks only when something is about to go (an approval past its delay, a Stop, a limit), never on a timer of its own.
  - An approval that comes due while you're at the PC is **not** counted: it is checked again every second while its card waits, and pings once you have been away long enough. So with the defaults, a request that arrives while you're away pings at 60 s if you left 2 min before, or as soon as you reach 2 min idle, and never once its card has gone back to the terminal (~108 s).
  - "Finished" and limits at the PC are dropped (you saw them); a limit still counts as seen.
  - If Windows can't say (the call fails), Glim treats you as away: a broken check must not silence the phone.
  - Input Glim itself types (a fix, a dictation) also counts as input; that only happens while you're using it.

## 6. Settings → Phone (new section)

- **Send notifications to my phone** — off by default.
- **Pushover keys** — user key and app token fields (masked once saved, Remove), a link to pushover.net to make them.
- **Notify me when:** an approval waits [60] s · a session finishes · a usage or spend limit is hit · my next meeting is in 10 min (disabled, "needs the calendar glance").
- **Include the project name** — on.
- **Only when I'm away from this PC** — on; [2] minutes without keyboard or mouse input (the field is greyed while the switch is off).
- **Quiet hours** — off; from [22:00] to [07:00].
- **Send test notification** — sends the fixed test text; shows "Sent" or the error. Works only with the switch on and keys saved.

Settings keys: one `phone` object in settings.json: `{ enabled, approval, approvalAfterSec, finished, limit, meeting, includeProject, idleOnly, idleAfterMin, quietHours, quietFrom, quietTo }`.

## 7. Tests (no real notifications)

- Rust: the message templates (every kind; unknown agent; project sanitising; project off); the idle reading (a live `GetLastInputInfo` call returns a value); the choke point (host refused while off, allowed while on, http and other ports and subdomains refused); the send against `serve_once` on 127.0.0.1 (the form fields, nothing else; an error status becomes a `failed` log line without the text); keys never in Debug output, settings.json or the log (source test); settings round trip and defaults.
- JS: `core/phone.ts` rules (delay, once per request, answered-before-delay, finished, limit once, quiet hours across midnight, away: threshold, at the PC not counted then sent once away, unknown counts as away, switched off); the hooks ask for the idle time only while the feature is on, and a Stop at the PC sends nothing; the island wiring (tick, Stop, StopFailure call it); the Settings section (off by default, fields, test button calls the bridge).
- No test talks to api.pushover.net. The live check is the owner's: buy the app, make the keys, press **Send test notification**.

## 8. Not in step 1

Answering from the phone (§9), any phone app of our own, rich notifications (buttons, deep links), the meeting reminder (after #34), other services (the design keeps one `Provider` so ntfy or Telegram can be added behind the same rules if the owner picks one).

## 9. Step 2: approve or deny from the phone (options only, not built)

Step 1 is one-way: Glim talks, the phone listens. Approving from the phone needs the phone's answer to come **back to the PC**, i.e. a two-way channel. Whatever carries it, the answer is an Allow/Deny on a real command, so the rules would be at least: the phone's answer names one request id and expires with it; it is signed or carried over an authenticated channel so nobody else can approve; destructive commands (`core/allow.ts` DESTRUCTIVE, the Claude Code "ask" rules) stay click-only at the PC; and the notification would need more than "needs approval" (at least the command's first word or a risk label), which changes §2's rule and needs its own decision.

| Option | How the answer comes back | Pushover fit | Inbound to the PC? | Notes |
|---|---|---|---|---|
| **A. Glim polls a relay** | phone taps a button that opens a URL on a small relay (e.g. a Cloudflare Worker); Glim polls it over https | Pushover messages can carry a URL (`url` field), but no action buttons | no (outbound polling only) | new host in the allowlist; a server to run and secure; polling latency/cost |
| **B. Pushover Open Client / receipts** | Pushover's emergency-priority **receipts** report only "acknowledged" (one bit, no Allow vs Deny); the Open Client API lets a desktop *receive* messages, not phone replies | partial | no | too thin for Allow/Deny; at most "seen" |
| **C. ntfy with action buttons** | ntfy notifications support `http` action buttons that POST to a URL; Glim subscribes to a reply topic (outbound) | switch service, or run both | no | needs access-controlled topics (paid hosted plan, or self-hosted); buttons are real Allow/Deny |
| **D. Telegram bot** | inline keyboard buttons; Glim long-polls `getUpdates` (outbound) | switch service | no | button answers are tied to the chat id; Telegram stores the chat; needs an account |
| **E. Direct to the PC** | phone reaches Glim on the LAN or through a tunnel (Tailscale, Cloudflare Tunnel) | n/a | **yes** | Glim would listen beyond 127.0.0.1: a big change to the privacy model (CLAUDE.md); a tunnel is a third party |
| **F. Our own phone app / PWA** | a page or app talking to a relay as in A, with its own login | could keep Pushover for the ping | no (with a relay) | most work; best UX and control |

(Service capabilities above are from memory, 2026-10-18; check each before step 2.) Leaning, to be decided with the owner when step 2 starts: **C or D** give real buttons with only outbound connections from the PC; **A** keeps Pushover but adds a server we own. **E** is the least attractive because it opens the PC to inbound traffic. Nothing here is built; step 1 stays one-way.
