# Phase 1.5 — Voice + Actions: plan

Status: **design approved by the owner 2026-10-13 (decisions in §8); not built.** Source: PROJECT_STATUS.md "Phase 1.5 — Voice + Actions" and `docs/vision.md` §2–4. Upstream Coucou's voice design (`docs/upstream-watch.md` row 4) is adapted, not ported: its code is Swift and its speech-to-text is Apple's.

Rules this plan keeps, unchanged:

- The microphone is open **only while the push-to-talk key is held**. A wake word comes later, off by default, on-device only.
- **Audio is never stored**: not on disk, not in the log. It lives in memory until it's transcribed, then it's dropped.
- **Transcripts are captured text**: never logged (the log gets outcome, durations, model, tokens, cost), and scanned for card/SSN/IBAN before anything leaves the machine.
- **Consequential actions wait for a click** (send, delete, purchase, publish). Low-risk ones (open, read, summarize) don't. **Sending an email always needs the user's explicit OK.** Voice can never approve an agent's permission request (it may deny one).
- `s-record` stays reserved for screen recording, `s-delegate` for data leaving the machine.

## 1. What the user sees: push-to-talk

> **⚠ Amended 2026-10-16 (Decision A, Anthony): two keys, not one router.** Anthony's live test on 2026-10-16 had Claude turn two plain dictated sentences ("This is a test of Glim dictation in my own voice", "Please send the updated lease to the tenant before Friday afternoon") into a question card. Now **Ctrl+Alt+Shift+V = dictate only** (the clean-up is offered only the `dictate` tool, forced; always typed) and **Ctrl+Alt+Shift+A = Ask Glim only** (every tool but `dictate`; never typed). The pill names the mode while a key is held ("Dictating" / "Ask Glim"). Both are in Settings → Shortcuts. The §4.1 command-word list now applies only to V without a clean-up. Ctrl+Alt+Shift+A probed 2026-10-17: free system-wide, unassigned in Word. **Decision B:** while Glim's question card waits, the next utterance from either key answers it ("Listening for your answer…", Cancel; a typed answer works too). Built on branch `voice-modes`.

1. **Hold the push-to-talk key**: proposed **Ctrl+Alt+Shift+V**, probed with `RegisterHotKey` and Word's `FindKey` the way the 1d keys were (Ctrl+Alt+Shift+Space is taken by "cycle visibility"). A global hotkey can't see "held", so Glim uses the key-down from the hotkey and polls `GetAsyncKeyState` every 30 ms for the release. No low-level keyboard hook.
   > **⚠ Conflict check run 2026-10-14 (unattended): Ctrl+Alt+Shift+V is free system-wide (`RegisterHotKey` probe, released at once) but Word's key table maps it to `InsertAutoText`** (`FindKey`, a hidden Word instance, quit after). A global hotkey takes it before Word sees it. **Decided 2026-10-14 (Anthony, via Jarvis): override Word.** Ctrl+Alt+Shift+V is registered globally for push-to-talk anyway; losing Word's InsertAutoText on that key is accepted. It stays rebindable in Settings → Shortcuts.
   > **Also found:** whisper.cpp's CLI reads audio from stdin but doesn't transcribe it (b5454), so a CLI child would need a temporary WAV (against "audio is never stored"). **`whisper-server`** works: a child process listening on 127.0.0.1 only, the audio POSTed from memory, the personal word list as its `prompt`, the model kept loaded (verified 2026-10-14 with the bake-off clips).
2. **While held:** the microphone records into memory, and the lantern shows the new **listening state** (§2). A thin level meter under the pill shows Glim is actually hearing something. Maximum 60 s, then it stops as if released.
3. **On release:** the mic closes at once. The lantern goes to `s-think` while transcribing locally, or `s-delegate` while audio or text is going to a cloud service.
4. **The transcript goes to the Claude API** (§4), which returns either **dictated text** or **one structured action**.
5. **What happens next** depends on the action (§4, §5): dictated text is typed into the focused field through 1b's insertion path; an open-app action just happens; a consequential one shows a card and waits.
6. **Under 0.4 s held** is treated as an accidental press: nothing is recorded or sent.
7. **Paused** (Ctrl+Alt+Shift+P) blocks dictation into fields (it's a writing feature) but not "open app" or "pick up from yesterday". The card says which. **Hidden** presence still allows push-to-talk; the result shows as a toast-sized island.

## 2. The listening state (new mascot state)

- **New state `s-hear`** (decided 2026-10-13), designed in `docs/brand/glim-mascot.html` first (a real state there since PR #24, which promoted the 2026-10-13 draft; the draft copy was deleted 2026-10-16) and ported with `node scripts/port-lantern.mjs` (never hand-edited, CLAUDE.md).
- Why not an existing one: `s-listen` already means "the island is open" (calm glance, `lantern.ts`); `s-record` is red and reserved for screen recording.
- Look (the design file's first `<style>`, promoted in PR #24): the lantern's warm glow turns **cool teal**, the flame breathes with the **microphone level** (one CSS variable set from the meter, clamped, ~15 updates a second), eyes look up. A small mic glyph on the badge spot. Reduced motion: a steady teal glow, no breathing.
- Precedence: while the key is held, `s-hear` wins over everything except a waiting approval's badge (the approval card still shows; the lantern still says it's listening).

## 3. Speech-to-text: how the comparison is run (first run 2026-10-14, §3.1)

Speech-to-text is **not decided** (owner, 2026-10-11). This is the method; the run and its results are PR 1.5a.

- **Candidates:**
  - **Local:** whisper.cpp through the `whisper-rs` crate (or the `whisper-cli` binary, run as a child process with no network). Models: `base.en`, `small.en`, `medium.en` (quantized `q5_1`). Model picked by speed on this CPU.
  - **Cloud:** two or three services, picked on the day from current docs and price pages (not from memory), each needing an API key in Windows Credential Manager like Anthropic's.
- **The clips:** 20 clips of the owner's own speech, 5–40 s each, recorded once and used for every option:
  - 8 dictation clips (emails and notes, with property-management words: unit numbers like "4B", tenant and vendor names, "lease renewal", "HOA", dollar amounts, dates);
  - 8 command clips ("open Outlook", "pick up where I left off in dale-cuts", "draft an email to …");
  - 4 hard clips: fast, noisy room, a pause mid-sentence, numbers spelled two ways.
  > **Note 2026-10-16:** the list to record is `docs/voice-clips-to-record.md`: 22 clips, mixed 6 dictation / 12 commands / 4 hard so that every approved command word and each `open_app` target gets a clip.
  - Each clip has a hand-written reference transcript.
  - **The clips are kept only for the comparison**, in a gitignored folder, and deleted after the owner picks. Recording them is the owner's call; Glim's "audio is never stored" rule applies to the product, and the clips are made with a separate recorder, not Glim.
- **Measured per option per clip:**
  - **Accuracy:** word error rate against the reference, plus a separate count of **errors in names, numbers and domain words** (those matter more than "the/a").
  - **Speed:** time from the end of audio to the transcript (cloud: including upload; local: on this PC, plugged in and on battery), median and worst.
  - **Cost:** cloud: the listed price × the owner's estimated use (e.g. 30 min of dictation a day); local: $0, plus RAM and CPU while transcribing and model size on disk.
- **Output:** `docs/voice-stt-results.md`, a table and a recommendation; the owner chooses. A cloud winner means a new host in the checkpoint (§6), called out in its PR.
- **Tooling:** a dev-only binary (`windows/src-tauri/examples/stt_compare.rs` or a script) that reads the clips and writes the table. It never ships in the app.

### 3.1 First run, 2026-10-14 (unattended): results and recommendation

Full write-up: **`docs/voice-stt-results.md`**. Tool: `windows/dev/stt-compare/` (dev-only).

> **⚠ Limits.** The 20 clips are **Windows' offline synthetic voices**, not Anthony's (his recordings don't exist yet), and the **cloud candidates couldn't be measured**: no Deepgram or AssemblyAI key on this machine. Their prices were read from the official pages. Latency carries over; the accuracy ranking is indicative only.

- **Measured (whisper.cpp b5454, 8 threads, Ryzen AI 7 350):**

  | | base.en q5_1 | small.en q5_1 | medium.en q5_0 |
  |---|---|---|---|
  | Lenient WER, plain / with vocabulary hint | 2.7% / 1.4% | 3.2% / 0.9% | 2.7% / 0.9% |
  | Command word heard first (12 clips) | 12/12 | 12/12 | 12/12 |
  | Processing, median (3.6 s clip) | **0.7–1.1 s** | 2.4–3.4 s | 6.6–10.2 s (slower than real time) |

- **Every model** got every command word first, so the §4.1 rule held on these clips. **Every model** misheard "Glim" ("glimpse") and "HOA board" ("ho aboard") without a vocabulary hint; with whisper's `--prompt` word list, most name errors went away.
- **Cloud, priced not measured:** Deepgram Nova-3 ≈ $5/month and AssemblyAI Universal-2 ≈ $3/month at 30 min/day with custom vocabulary.
- **Recommendation (the owner chooses):**
  - **local `base.en` + a personal vocabulary list** as the default for now (fast, free, nothing leaves the PC);
  - **not `medium.en`** on this CPU;
  - **re-run with Anthony's own 20 clips** to settle base vs small;
  - try **one** cloud key only if real-speech accuracy disappoints.
- New for 1.5b if base wins: a **"Words Glim should know"** list in Settings, passed as whisper's initial prompt (it never leaves the PC). Not decided; listed for the owner.

## 4. From speech to a structured action (Claude API)

> **⚠ Amended 2026-10-16 (Decision A):** the key picks the tool set, Claude no longer decides between dictation and a command. Dictation: `tools: [dictate]`, `tool_choice: {"type": "tool", "name": "dictate"}`. Ask Glim and answers: the other four tools, `tool_choice: {"type": "any"}`; an answer's request also carries the earlier request and Glim's question. A tool the mode didn't offer doesn't validate. See §1's note.

- **One request per utterance**, through 1c's client (`ai::claude`, the same key, model, monthly cap and spend accounting). Default model `claude-haiku-5-5` (as 1c). The lantern is `s-delegate` while it runs.
- **Input:** the transcript, plus a small context block: the focused app's exe and field kind (from the writing layer, never the field's text), today's date, the list of known repos (names only), and the known apps.
- **Output: tool use with exactly one tool call**, `tool_choice: {"type": "any"}`. Claude never acts; it only picks a tool and fills its fields. Glim validates the call against the schema and decides. Anything that doesn't validate, or a command Glim can't carry out, shows **"Here's what I heard: … (couldn't act on it)"** with the transcript on the card, and nothing happens (§4.1).
- **The tools (the action schema):**

| Tool | Fields | Risk | What Glim does |
|---|---|---|---|
| `dictate` | `text` (cleaned: filler words out, punctuation in, tone matched to the app kind) | low | Types `text` into the focused field through 1b's insertion path (with its checks: the field still focused, editable, not password, not paused). |
| `open_app` | `app`: one of `claude_code`, `claude`, `codex`, `outlook`, or the name of one of the user's own apps (Settings → Writing → Apps Glim can open) (an enum, never a free path) | low | Opens it. No confirmation (owner, PROJECT_STATUS). An own app's path never goes to Claude; Glim looks the name up and checks the path again before opening it (2026-10-19). |
| `resume_work` | `repo` (optional; a name from the known-repos list) | low (reads) | Reads HANDOFF.md across repos (the `resume` module, PR "voice-resume-reader"), shows a card per repo with where it stopped; on a click on one, opens Claude Code in it, told to read HANDOFF.md and continue. With `repo` set, that repo's card first. |
| `draft_email` | `to` (names or addresses, as said), `subject`, `body` | **consequential** | Creates a **draft** in classic Outlook via COM and shows it in a Glim card. **Never sends.** Sending is the user's click on Send, in Outlook or on the card, every time. |
| `clarify` | `question` | — | Shows the question; the user answers by holding the key again. |

- **Examples** (transcript → tool call):
  - "hey can you say thanks for the quick turnaround on unit 4B we'll get the lease to you by friday" in Outlook → `dictate { text: "Thanks for the quick turnaround on unit 4B. We'll get the lease to you by Friday." }`
  - "open outlook" → `open_app { app: "outlook" }`
  - "pick up from yesterday" → `resume_work {}`; "where was I on dale cuts" → `resume_work { repo: "dale-cuts" }`
  - "draft an email to Maria saying the plumber comes Tuesday at ten" → `draft_email { to: ["Maria"], subject: "Plumber visit Tuesday", body: "Hi Maria,\n\nThe plumber is coming Tuesday at 10am.\n\n…" }`
  - "send it" → **not a tool**. There is no `send_email` tool; Claude can't produce one. The reply is `clarify { question: "Click Send on the draft to send it." }`.
- **Sensitive data:** the transcript goes through the card/SSN/IBAN scan before the API call. A hit stops the request ("Not sent: …", the 1c wording). For `dictate`, the text Claude returns is scanned again before typing.

### 4.1 When a step fails (decided 2026-10-13, Anthony; command-word list confirmed 2026-10-14 — §4.1 fully confirmed)

**The Claude clean-up step fails** when any of these happens:

- the API is unreachable or returns an error (an outage);
- no answer within **~4 s** of sending (the request is cancelled; a late answer is discarded, so nothing is ever typed twice; a cancelled request may still be billed and is counted like any other);
- **no API key** is set;
- the **monthly spend cap** is reached (1c's cap; nothing is sent).

What happens then:

- **Dictation only: the raw transcript is typed**, after the sensitive scan, through 1b's insertion path with all its checks.
- **The "not cleaned up" marker is shown by Glim**, on the result card and as the lantern's badge, **never inserted into the typed text.** The text typed is exactly the transcript.
- **A spoken command is never typed as raw text.** If a command can't be acted on, the card shows **"Here's what I heard: … (couldn't act on it)"** and nothing happens. Without Claude, Glim can't classify the utterance, so it uses a cautious local rule: the transcript is typed **only** if an editable, non-password field had the focus when the key was pressed **and** the transcript doesn't start like a command (a short fixed list: "open", "pick up", "resume", "where was I", "draft", "email", "send", "reply", "delete", "go to", in any case, after leading filler like "hey Glim" or "um"). Anything else gets the "Here's what I heard" card. This is not a phrase parser: it never runs an action, it only decides whether raw typing is safe. *(Local rule added 2026-10-13 to carry out the decision; **the list was confirmed as-is by Anthony on 2026-10-14**.)*

**Speech-to-text fails** (the cloud service is down, slow or refuses):

- If a **local whisper model is installed**, Glim **falls back to it automatically** and the card notes it ("Transcribed on this PC"). Then the steps above run as usual.
- If **neither works**, the card says so, and **nothing is typed** and nothing is sent to Claude.
- If whisper.cpp wins the comparison and is the main speech-to-text, there is no cloud fallback (none is configured): a local failure is "neither works".

**Sensitive scan hit** (unchanged): nothing is sent or typed; "Not sent: …" (the 1c wording).

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

## 8. Decided 2026-10-13 (Anthony)

1. **Push-to-talk = Ctrl+Alt+Shift+V, held.** Still needs the usual conflict check when Anthony is at the PC: `RegisterHotKey` probe plus Word's key table (`Application.FindKey`), as the 1d keys got. If it's taken, the plan is amended before 1.5b, not worked around.
2. **The listening state is `s-hear`** (not `s-record`, which stays screen recording only; not `s-listen`, which means "island open"). **Its look is designed in `docs/brand/glim-mascot.html` first** (the source of truth). A draft is there (2026-10-13, for Anthony's review); no code uses `s-hear` and it isn't ported (`port-lantern.mjs` not run) until he approves the look.
3. **Speech-to-text comparison:** **two cloud candidates** against **whisper.cpp `base.en` / `small.en` / `medium.en`**, on the same clips (§3).
4. **If the Claude clean-up step fails, dictation types the raw transcript** (after the sensitive scan). Confirmed by Anthony 2026-10-13, with details: the trigger is any clean-up failure (outage, no answer within ~4 s, no API key, monthly cap reached); dictation only, a command is never typed (it gets "Here's what I heard: … (couldn't act on it)" and nothing happens); "not cleaned up" shows on Glim's card and badge, never in the typed text; a failed cloud speech-to-text falls back to a local whisper model when one is installed (noted on the card), and with neither, the card only, nothing typed. Full rules: §4.1.
5. **`resume_work` opens Claude Code only after a click on that repo's card.**
6. **Resume reader (`resume_work`'s data, PR #21): a card-like number (card, SSN, IBAN, 12+ digit run) withholds only the section it is in**, marked as withheld; true secrets (API keys, tokens, private keys, `password=`-style values) still skip the whole HANDOFF.md.
