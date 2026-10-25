# Glim learns: shortcuts, routines and remembered rules (plan)

Status: **design approved; decisions recorded in §9 (DECIDED 2026-10-24).** §1-§2 built 2026-10-25 (PR `learned-shortcuts`), §3 (PR `remember-rules`), §5-§6 (PR `capture-exclusion`).

**Credit.** The ideas are adapted from Atticus, an MIT-licensed voice assistant ([atticus-mac](https://github.com/SaarthurR/atticus-mac), [atticus-windows](https://github.com/FindlayHi/atticus-windows)): its README and `atticus.py`'s `learn`, `replay`, `like`, `gist`, `relevant_rules` and `remember_workflow`. No code is copied. If a later PR ports any, it carries the MIT notice in `NOTICE.md`.

The rules below come on top of `CLAUDE.md` and `docs/voice-actions-plan.md`, which stay as they are:

- **Ask lane only** (Ctrl+Alt+Shift+A, and typed answers to Glim's questions). **The dictation key never learns, never replays, never runs a routine, and its requests never carry rules or facts.**
- **Learning never makes a new power.** A shortcut or routine only repeats a tool call that Glim already validates and runs today, with the same checks. Nothing consequential is ever learned (sending, deleting, drafting, typing).
- **Everything is local, visible in Settings, and can be deleted.** The log gets counts, never words.

## 1. Learned shortcuts

**What it does.** After an Ask-lane request succeeds and Glim has checked that it worked, Glim keeps the validated tool call under the request's *gist*. The next time the user asks the same thing, Glim matches the gist **locally**, before any API call, and replays the call: no Claude, no `s-delegate`, no cost, about 0.2 s. The result card carries a small "Learned" note, so the user knows Claude wasn't asked.

**Which calls can be learned (an allowlist; anything else is never learned):**

| Tool | Learnable | Why |
|---|---|---|
| `open_app` | yes | an enum value (built-in or the user's own app from #40) |
| `resume_work` | yes | `repo` is a name from the known-repos list. A replay only shows the cards: opening Claude Code still waits for a click (plan §5) |
| window actions | not in v1 (§9, decision 5) | an enum action on one named app |
| `draft_email`, `edit_draft` | **never** | consequential, and free text |
| `dictate`, `clarify`, answers to a question | **never** | free text, or depends on the moment |

**The gist (the key).** Lowercased; punctuation dropped; filler ("um", "hey", "glim", "please", "can you", "quickly", …) and stop words ("a", "the", "to", "my", …) removed; a few verbs folded together ("launch"/"start" → "open"). The key is the sorted list of what's left, so word order doesn't matter: "um, can you open outlook for me" and "outlook, open" are both `[open, outlook]`.

**App swap.** If the gist names exactly one app Glim knows, it is stored with the app as a slot: `[open, {app}]`. A new ask with the same words about another known app (`[open, codex]`) replays the shortcut with the new app, and only for tools whose only field is that app. An app swap can never invent an app: the new one must be in today's enum.

**Never learned:**

- **Deictic asks**: any of this, that, these, those, it, here, there, them. "Close this" means something else every time.
- **Failed or unconfirmed results**: an error, a refusal, a "couldn't act on it" card, a check after acting that didn't pass, a Claude answer that needed a sound-alike correction, or a request that went through a question (`clarify`).
- **Anything risky** (the allowlist above), anything the sensitive scan (card/SSN/IBAN) or the §3 filter flags, and anything asked while learning is off in Settings.
- A gist with fewer than 2 words, or one that is just a command word from plan §4.1 ("open").

**Check after acting (before learning).** Glim learns only what it saw work:

- `open_app`: within 5 s, a visible top-level window belonging to the started process (or to the app's exe if it was already running) is in front.
- `resume_work`: at least one repo card was read and shown.
- window actions: the target window's rectangle afterwards matches the target zone.

**Replay.** The stored call goes through **today's** validation (the own app is still in Settings, the repo is still known, the enum still has the value), runs, then gets the same check. **If anything fails, the ask goes to Claude as if no shortcut existed** (the normal path, `s-delegate`, the 4 s rules of plan §4.1). If Claude's answer then works, it replaces the shortcut. The second failure in a row deletes it.

**"Try again"** on a shortcut's result card deletes that shortcut and sends the ask to Claude.

**Expiry.** A learned shortcut that hasn't been used for **30 days** is deleted when the file is next saved; each use restarts the clock. User-made entries (routines, §2) never expire. At most 200 shortcuts; past that, the least recently used goes.

## 2. Routines ("When I say X, …")

Builds on #40 (the user's own apps in `open_app`).

- **Saying it.** In the Ask lane, "When I say *work mode*, open Outlook and Codex and pick up my-repo" → Claude answers with a new tool, `save_routine { name, steps[] }`, each step being one allowed tool call with its normal fields. Glim validates every step, then shows a card: *"Save 'work mode'? 1. Open Outlook 2. Open Codex 3. Resume my-repo"* with **Save** and **Cancel**. Nothing runs and nothing is stored until Save (§9, decision 3).
- **Running it.** Saying the name (matched by gist, before shortcuts and before any API call) runs the steps in order. Each step is checked as in §1. On the first failure Glim stops and says which step: *"Step 2 of 3 (Open Codex) didn't work."* Steps after it don't run.
- **Allowed steps:** `open_app` (built-in or own app), `resume_work` (cards only; Claude Code still opens on a click), window actions once they exist. **Not allowed:** `dictate`, `draft_email`, `edit_draft`, `clarify`, another routine, or anything consequential.
- **Limits:** at most **5 steps** per routine and **20 routines**. A name is 2–40 characters (letters, digits, spaces, `-'`), not deictic, not a built-in or own app's name, not a §4.1 command word, and not another routine's name.
- **Settings → Ask Glim → Routines:** the list; each one opens to show its steps; **Rename** and **Delete**. Editing steps in place isn't in v1: delete and say it again.
- A step whose own app was removed from Settings fails at run time, with the step named on the card.

## 3. Remembered rules and "about me" facts

- **Saying it.** "Remember: when I say the office I mean Outlook" → a new tool, `remember { kind: rule | fact, text }`. Glim decides locally whether to keep it, and the card shows exactly what was saved.
- **Rules:** short instructions, at most 200 characters each and 50 in all.
- **Facts:** "about me" lines of **8 words or fewer** (e.g. "Prefers short replies", "Likes meetings after ten"), at most 30.
- **Filter (both kinds, run by Glim, not trusted to Claude):** the card/SSN/IBAN scan, plus a local refusal list: **health**, **money** (amounts, accounts, salary, debts), **IDs and numbers** (any run of 4 or more digits, email addresses, phone numbers) and **tenant or customer data** (unit/apt numbers, lease, tenant, resident, owner names next to a unit). A refusal shows *"Not saved: Glim doesn't keep health, money, ID or customer details."* `remember`'s tool description says the same, so Claude rarely offers one.
- **What goes in the prompt (Ask lane only).** If all rules and facts together are under **~4,000 characters**, all of them go in. Past that: every fact, then the rules sharing at least one non-stop word with the transcript, best overlap first, at most 15. Dictation requests never carry any of it.
  > **⚠ Amended 2026-10-25 (decision 2, "only relevant rules per request"):** rules are **always** chosen by relevance, not all of them under 4,000 characters: every fact, then only the rules sharing a non-stop word with the request, most shared first (then the newest), at most 15, the whole block within 4,000 characters (`memory::for_request`).
- **Correction tip.** When the user had to correct Glim (pressed "Try again", or their next Ask within 60 s starts with "no", "not that", "I meant" or "wrong"), the result card adds one line: *"Tip: say 'Remember: …' and I'll get it right next time."* At most once per correction, never on the dictation key. Glim doesn't save a rule by itself (§8).
- **Settings → Ask Glim → Memory:** rules and facts listed, each editable and deletable, plus **Forget everything** (rules, facts, shortcuts, routines).

## 4. Storage, privacy, logs

- **Where:** `%LOCALAPPDATA%\Glim\learn\`: `shortcuts.json`, `routines.json`, `memory.json`. Each write goes to a temp file, then a rename, so a crash can't truncate it. A file that doesn't parse is set aside as `*.bad` and treated as empty (logged as a count, never its text).
- **What's stored:** gists (the kept words, not the transcript), routine names, validated tool calls (enums and known names only, never paths: #40's paths stay in settings.json), and the user's rules and facts. **Never** audio, raw transcripts or Claude's text.
- **Network:** no new host, no new dependency. Replays and routines make **fewer** API calls. **Privacy change to call out in the PR (CLAUDE.md): rules and facts go to `api.anthropic.com` inside Ask-lane requests**, after the sensitive scan. Settings says so under the Memory list.
- **Settings switch:** "Learn shortcuts from what I ask" (default **on**, §9 decision 1). Off = no learning and no replay of learned shortcuts; routines and memory still work, since the user made them.
- **Logs: counts only**, e.g. `learn: replay ok (open_app)`, `learn: replay failed -> claude`, `learn: saved shortcut (resume_work)`, `learn: not learned (deictic)`, `learn: routine ran 3/3 steps`, `learn: memory refused (filter)`, and at startup `learn: 12 shortcuts, 3 routines, 7 rules, 4 facts`. **Never** a gist, routine name, app name, rule or fact (as #40's log never names an app).
- **CLAUDE.md** and `docs/voice-actions-plan.md` get the new tools (`save_routine`, `remember`) and the "rules and facts go to the API" line in the PR that builds this, not later.

## 5. Separate item (a): keep Glim's windows out of screen capture

- Call `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on every Glim window (island, pill, cards, mini lantern, Settings) when it's created. Screen shares, recordings and screenshots then show what's behind Glim, not Glim, so the user's cards (transcripts, drafts) don't leak into a Teams share.
- Needs Windows 10 2004 or later. If the call fails, log `capture-exclude: unsupported` once and carry on.
- Check before shipping: that it holds for the transparent island window, and whether `PrintWindow` (our test screenshots, CLAUDE.md) still sees Glim's own window. If it doesn't, a dev-only environment variable `GLIM_CAPTURABLE=1` turns the exclusion off for tests.
- **Checked 2026-10-25** (a window of the probe's own, Windows 11): once excluded, `PrintWindow` with `PW_RENDERFULLCONTENT` (the verify scripts' call; WebView2 needs it) comes back black; `PrintWindow` without the flag still drew that plain GDI window; a screen copy was black. So the override is needed, and the verify scripts set `GLIM_CAPTURABLE=1`. Not yet checked on Glim's own transparent island (needs a build).
- Settings switch "Hide Glim in screen shares and recordings" (default **on**, §9 decision 4; `GLIM_CAPTURABLE=1` turns it off for tests). Its own small PR, independent of §1–3.

## 6. Separate item (b): cost and latency budget

- Keep, in memory, the last **50** Claude requests: lane (dictate / ask / edit), total time in ms, cost in nanodollars (from `ai::spend`). No text.
- Every 50th request, and on quit, one log line: `budget: last 50 median 1.3 s, $0.0004/request (ask 18, dictate 32)`. If the median is over **2.0 s** (half of the 4 s cancel) or over **$0.001**, the line ends `OVER` (thresholds are settings constants, easy to change).
- Unit tests: median of odd/even/empty windows; the window drops the oldest; a request at the §3 prompt limit (4,000 characters of rules) stays under a fixed input-token estimate, so memory can't silently make every Ask expensive.
- Its own small PR. Shortcut replays are counted separately (`replays N`) so the saving is visible.

## 7. Fit with the two voice lanes

| | Dictate (Ctrl+Alt+Shift+V) | Ask Glim (Ctrl+Alt+Shift+A) |
|---|---|---|
| Learns shortcuts | never | yes (§1) |
| Matches shortcuts / routines locally | never | yes, before the API |
| Rules and facts in the prompt | never | yes (§3) |
| Correction tip | never | yes |

Paused (Ctrl+Alt+Shift+P) blocks dictation only, as today: shortcuts and routines still run, since none of them types.

## 8. Not in v1

- Saving a rule automatically from a correction (Atticus does; here the user says "Remember").
- Atticus's longer "procedures" that steer Claude on multi-step tasks.
- Learning from the dictation lane, or from typed (non-voice) asks.
- Editing a routine's steps in place; sharing, exporting or syncing anything.
- Glim picking up facts about the user without being told.
- The `window` tool itself (a separate plan and PR, §9 decision 5).

## 9. Decisions (DECIDED 2026-10-24, owner)

1. **"Learn shortcuts" is ON by default.** A learned result says so on its card ("Learned"), and **Try again** on it removes that shortcut (§1).
2. **Rules and facts go to `api.anthropic.com` with Ask requests: yes.** The sensitive filter (§3) runs when one is saved; only the rules relevant to the request go with it (§3's selection), and every rule and fact is listed in Settings with Delete. CLAUDE.md's privacy section says so in the PR that builds it.
3. **Routines: a confirmation card at creation** listing every step (Save / Cancel); once saved, saying the name runs it at once. Steps that would be risky still ask, as they do today.
4. **Hide from screen capture: a Settings switch, ON by default.** A test-only environment variable turns it off so the PrintWindow verify scripts keep working (§5).
5. **No `window` tool in v1.** Learning covers `open_app` and `resume_work` only.

## 10. Test plan (no live clicks on the owner's desktop)

All automated, in CI or locally, through the existing fakes (`ai/dictate.rs`'s local fake server, no real API key) and a fake "actor" trait for opening apps and finding windows. No mouse, keyboard or real app starts.

- **Gist:** a table test (filler, stop words, order, verb folding, punctuation); two words that differ only by order give the same key.
- **App swap:** `[open, {app}]` learned for Outlook replays for Codex; not for an unknown app; not for a tool with more than one field.
- **Never learned:** deictic words, failed results, a failed check after acting, a sound-alike correction, `clarify`, every `draft_email`/`edit_draft`/`dictate` call, learning off, a sensitive hit, a 1-word gist.
- **Replay:** fake actor succeeds → no request reaches the fake server; fake actor fails → exactly one request reaches it; two failures in a row delete the shortcut; "Try again" deletes it; an own app removed from Settings fails validation and falls back.
- **Expiry:** a 31-day-old entry goes on save, a user-made one stays; the 200 limit drops the least recently used.
- **Routines:** name rules; 6 steps refused; a forbidden step refused; nothing stored before Save; running stops at the first failing step and names it.
- **Memory:** a 9-word fact refused; each refusal-list category refused; the prompt holds everything under 4,000 characters and the top 15 matching rules over it; a dictation request body never contains a rule or fact.
- **Logs:** the requests above are run with marker words, then `glim.log` is searched: none of the marker words may appear.
- **Files:** a truncated or corrupt JSON file is set aside and treated as empty; writes are atomic.
- **Capture exclusion:** a test creates a hidden window of its own, sets the affinity and reads it back with `GetWindowDisplayAffinity` (no screen capture taken).
- **Budget:** the §6 unit tests.
- **Settings pages:** WebDriver (tauri-driver) against a test profile with seeded files: lists render, rename and delete change the files.
- **Hand tests for the owner** (when at the PC, by the owner, not Claude): a new `docs/manual-tests.md` section per PR, e.g. "say 'open Outlook' twice: the second has the "Learned" note and no `s-delegate`".
