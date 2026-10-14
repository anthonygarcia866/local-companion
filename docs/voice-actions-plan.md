# Phase 1.5 — Voice + Actions: plan

Status: **design for the owner's approval (2026-10-13), not built.** Source: PROJECT_STATUS.md "Phase 1.5 — Voice + Actions" and `docs/vision.md` §2–4. Upstream Coucou's voice design (`docs/upstream-watch.md` row 4) is adapted, not ported: its code is Swift and its speech-to-text is Apple's.

Rules this plan keeps, unchanged:

- The microphone is open **only while the push-to-talk key is held**. A wake word comes later, off by default, on-device only.
- **Audio is never stored**: not on disk, not in the log. It lives in memory until it's transcribed, then it's dropped.
- **Transcripts are captured text**: never logged (the log gets outcome, durations, model, tokens, cost), and scanned for card/SSN/IBAN before anything leaves the machine.
- **Consequential actions wait for a click** (send, delete, purchase, publish). Low-risk ones (open, read, summarize) don't. **Sending an email always needs the user's explicit OK.** Voice can never approve an agent's permission request (it may deny one).
- `s-record` stays reserved for screen recording, `s-delegate` for data leaving the machine.

## 1. What the user sees: push-to-talk

1. **Hold the push-to-talk key**: proposed **Ctrl+Alt+Shift+V**, probed with `RegisterHotKey` and Word's `FindKey` the way the 1d keys were (Ctrl+Alt+Shift+Space is taken by "cycle visibility"). A global hotkey can't see "held", so Glim uses the key-down from the hotkey and polls `GetAsyncKeyState` every 30 ms for the release. No low-level keyboard hook.
2. **While held:** the microphone records into memory, and the lantern shows the new **listening state** (§2). A thin level meter under the pill shows Glim is actually hearing something. Maximum 60 s, then it stops as if released.
3. **On release:** the mic closes at once. The lantern goes to `s-think` while transcribing locally, or `s-delegate` while audio or text is going to a cloud service.
4. **The transcript goes to the Claude API** (§4), which returns either **dictated text** or **one structured action**.
5. **What happens next** depends on the action (§4, §5): dictated text is typed into the focused field through 1b's insertion path; an open-app action just happens; a consequential one shows a card and waits.
6. **Under 0.4 s held** is treated as an accidental press: nothing is recorded or sent.
7. **Paused** (Ctrl+Alt+Shift+P) blocks dictation into fields (it's a writing feature) but not "open app" or "pick up from yesterday". The card says which. **Hidden** presence still allows push-to-talk; the result shows as a toast-sized island.

## 2. The listening state (new mascot state)

- **New state `s-hear`**, added to `docs/brand/glim-mascot.html` first and ported with `node scripts/port-lantern.mjs` (never hand-edited, CLAUDE.md).
- Why not an existing one: `s-listen` already means "the island is open" (calm glance, `lantern.ts`); `s-record` is red and reserved for screen recording.
- Proposed look: the lantern's warm glow turns **cool teal**, the flame breathes with the **microphone level** (one CSS variable set from the meter, clamped, ~15 updates a second), eyes look up. A small mic glyph on the badge spot. Reduced motion: a steady teal glow, no breathing.
- Precedence: while the key is held, `s-hear` wins over everything except a waiting approval's badge (the approval card still shows; the lantern still says it's listening).

## 3. Speech-to-text: how the comparison will be run (not run yet)

Speech-to-text is **not decided** (owner, 2026-10-11). This is the method; the run and its results are PR 1.5a.

- **Candidates:**
  - **Local:** whisper.cpp through the `whisper-rs` crate (or the `whisper-cli` binary, run as a child process with no network). Models: `base.en`, `small.en`, `medium.en` (quantized `q5_1`). Model picked by speed on this CPU.
  - **Cloud:** two or three services, picked on the day from current docs and price pages (not from memory), each needing an API key in Windows Credential Manager like Anthropic's.
- **The clips:** 20 clips of the owner's own speech, 5–40 s each, recorded once and used for every option:
  - 8 dictation clips (emails and notes, with property-management words: unit numbers like "4B", tenant and vendor names, "lease renewal", "HOA", dollar amounts, dates);
  - 8 command clips ("open Outlook", "pick up where I left off in dale-cuts", "draft an email to …");
  - 4 hard clips: fast, noisy room, a pause mid-sentence, numbers spelled two ways.
  - Each clip has a hand-written reference transcript.
  - **The clips are kept only for the comparison**, in a gitignored folder, and deleted after the owner picks. Recording them is the owner's call; Glim's "audio is never stored" rule applies to the product, and the clips are made with a separate recorder, not Glim.
- **Measured per option per clip:**
  - **Accuracy:** word error rate against the reference, plus a separate count of **errors in names, numbers and domain words** (those matter more than "the/a").
  - **Speed:** time from the end of audio to the transcript (cloud: including upload; local: on this PC, plugged in and on battery), median and worst.
  - **Cost:** cloud: the listed price × the owner's estimated use (e.g. 30 min of dictation a day); local: $0, plus RAM and CPU while transcribing and model size on disk.
- **Output:** `docs/voice-stt-results.md`, a table and a recommendation; the owner chooses. A cloud winner means a new host in the checkpoint (§6), called out in its PR.
- **Tooling:** a dev-only binary (`windows/src-tauri/examples/stt_compare.rs` or a script) that reads the clips and writes the table. It never ships in the app.

## 4. From speech to a structured action (Claude API)

- **One request per utterance**, through 1c's client (`ai::claude`, the same key, model, monthly cap and spend accounting). Default model `claude-haiku-5-5` (as 1c). The lantern is `s-delegate` while it runs.
- **Input:** the transcript, plus a small context block: the focused app's exe and field kind (from the writing layer, never the field's text), today's date, the list of known repos (names only), and the known apps.
- **Output: tool use with exactly one tool call**, `tool_choice: {"type": "any"}`. Claude never acts; it only picks a tool and fills its fields. Glim validates the call against the schema and decides. Anything that doesn't validate is shown as "Didn't understand" with the transcript, and nothing happens.
- **The tools (the action schema):**

| Tool | Fields | Risk | What Glim does |
|---|---|---|---|
| `dictate` | `text` (cleaned: filler words out, punctuation in, tone matched to the app kind) | low | Types `text` into the focused field through 1b's insertion path (with its checks: the field still focused, editable, not password, not paused). |
| `open_app` | `app`: one of `claude_code`, `codex`, `outlook` (an enum, never a free path) | low | Opens it. No confirmation (owner, PROJECT_STATUS). |
| `resume_work` | `repo` (optional; a name from the known-repos list) | low (reads) | Reads HANDOFF.md across repos (the `resume` module, PR "voice-resume-reader"), shows a card per repo with where it stopped; on a click on one, opens Claude Code in it, told to read HANDOFF.md and continue. With `repo` set, that repo's card first. |
| `draft_email` | `to` (names or addresses, as said), `subject`, `body` | **consequential** | Creates a **draft** in classic Outlook via COM and shows it in a Glim card. **Never sends.** Sending is the user's click on Send, in Outlook or on the card, every time. |
| `clarify` | `question` | — | Shows the question; the user answers by holding the key again. |

- **Examples** (transcript → tool call):
  - "hey can you say thanks for the quick turnaround on unit 4B we'll get the lease to you by friday" in Outlook → `dictate { text: "Thanks for the quick turnaround on unit 4B. We'll get the lease to you by Friday." }`
  - "open outlook" → `open_app { app: "outlook" }`
  - "pick up from yesterday" → `resume_work {}`; "where was I on dale cuts" → `resume_work { repo: "dale-cuts" }`
  - "draft an email to Maria saying the plumber comes Tuesday at ten" → `draft_email { to: ["Maria"], subject: "Plumber visit Tuesday", body: "Hi Maria,\n\nThe plumber is coming Tuesday at 10am.\n\n…" }`
  - "send it" → **not a tool**. There is no `send_email` tool; Claude can't produce one. The reply is `clarify { question: "Click Send on the draft to send it." }`.
- **Dictation without the API** (offline, cap reached, or the API off): the raw transcript is typed as is, after the sensitive scan. Actions need the API; without it the card says so. No local phrase parser in the first version (upstream has one; it's a later fallback).
- **Sensitive data:** the transcript goes through the card/SSN/IBAN scan before the API call. A hit stops the request ("Not sent: …", the 1c wording). For `dictate`, the text Claude returns is scanned again before typing.

## 5. Approval rule

- **Low-risk, done at once:** `dictate` (it types into the field the user is in, and Ctrl+Z undoes it), `open_app`, `resume_work`'s read and summary.
- **Waits for a click:** `resume_work`'s "open Claude Code in this repo" (it starts a process in a folder), and every `draft_email` card.
- **Never possible by voice:** sending, deleting, purchasing, publishing; approving an agent's permission request. These have no tool, so no transcript can trigger them.
- A power only moves to "do it without asking" when the owner says so for that one power (vision.md §3).

## 6. Network checkpoint changes

- **Anthropic: no change.** `api.anthropic.com` (https, 443) is already allowed since 1c; voice reuses `net::request`.
- **Local whisper: no change.** It runs in-process or as a child process. The child must not reach the network: it's launched with no proxy variables, and the PR's empirical check is netstat over `glim.exe` and every child during a transcription (only loopback and Anthropic). **The model file is downloaded by hand** (the owner, from a link in the docs, checked against a published SHA-256), not by Glim: Glim makes no download.
- **A cloud speech-to-text winner:** one new host constant beside `ANTHROPIC_HOST`, the same rules (https on 443 only, no wildcard, no redirect to another host), its own refusal tests, and the PR description says so in its first line. Audio is uploaded only while the key was held, and only to that host.
- **Microphone:** WASAPI through the `windows` crate (no new crate if possible). Windows' microphone privacy setting applies; if Glim is denied, the card says so and links to the setting.
- **Outlook COM:** local, no network from Glim (Outlook syncs the draft itself, as it would any draft).

## 7. PR split

1. **voice-resume-reader** (draft, 2026-10-13): the UI-free `resume` module that reads and parses HANDOFF.md files. No mic, no network, no UI.
2. **1.5a — speech-to-text comparison:** the dev-only compare tool, the clips' method, `docs/voice-stt-results.md`. Nothing ships in the app. The owner picks.
3. **1.5b — push-to-talk and dictation:** the hotkey + release polling, WASAPI capture to memory, the chosen speech-to-text, `s-hear` (design file first), `dictate` through Claude, typing through 1b's path. Settings: the key, mic device, on/off.
4. **1.5c — actions:** the tool schema and validation, `open_app`, `resume_work` cards (on top of PR 1), `clarify`.
5. **1.5d — Outlook drafts:** `draft_email` through classic Outlook COM, the draft card, Send only by a click.

## 8. Decisions for Anthony

1. **Push-to-talk key: Ctrl+Alt+Shift+V (held)?** Recommended: **yes**, after the same `RegisterHotKey` + Word `FindKey` probe the 1d keys got.
2. **New mascot state `s-hear` (cool teal, breathing with the mic level)?** Recommended: **yes**; `s-listen` already means "island open".
3. **Cloud candidates for the comparison:** Recommended: **two**, picked from current price pages on the day, plus whisper.cpp `base.en` / `small.en` / `medium.en`.
4. **Dictation when the API is unavailable: type the raw transcript, or refuse?** Recommended: **type the raw transcript** (after the sensitive scan), and say "not cleaned up" on the card.
5. **`resume_work` opens Claude Code only after a click on a repo's card?** Recommended: **yes**, even though "open app" needs no click: it starts work in a specific folder.
