# Phase 1 — writing assistant: plan

Status: **reviewed by the owner 2026-10-10**; the answers are in §11. PR 1a is being built on `phase-1a-checker`. Inputs: the owner's Phase 1 brief, `docs/capture-results.md` (the capture spike and the model timings), PR #8 (payment and ID filtering), and a Harper trial run on this machine (below).

If a step here turns out wrong while building, this file gets a warning note next to that step; the code is not quietly worked around it.

## 1. What the user sees

1. They type in a supported app (Outlook, Word, Gmail in Chrome, Notepad, a chat app).
2. About **0.6 s after they stop typing** (owner, 2026-10-10: 1 s felt slow), Glim checks the paragraph the caret is in. Only the text they are writing counts: the email signature and quoted reply text are left out.
3. **Issues found:** the lantern goes to **`s-suggest`** with the attention pop, and the pill shows the **count** ("3").
   **No issues:** the lantern goes back to **`s-listen`** (island open) or `s-idle` (in the notch), and the count disappears.
4. **Clicking the lantern** (or the ember) opens the island on a **suggestion list**, nearest the caret first, three at a time with Previous / Next. Each entry shows the problem text, a short explanation and the replacement options.
5. **1b:** clicking a replacement applies it in the app, with native undo.
6. **1c:** **Ctrl+Alt+Shift+R** sends the paragraph (or the selection) to the local model (`gemma3:4b`) with a tone: clearer, friendlier, more formal or shorter. The lantern shows **`s-think`** while it generates.

Glim never takes focus at any point (the island, panel and pill are non-activating, as today).

## 2. PR split

| PR | Scope | Done when |
|---|---|---|
| **1a — instant checker** | Capture runs in normal sessions for apps that are on. 600 ms debounce. Paragraph extraction without the signature or quote. Harper. Lantern `s-suggest` + count. A **read-only** suggestion list in the island. Per-app defaults hardcoded. | Notepad self-test passes; <50 ms per check measured; tests below pass; the owner tries it in Outlook, Word and Gmail. |
| **1b — apply fixes** | Clicking a replacement writes it back (design in §7). The floating bubble at the field (§7a). | The write-back is verified in Notepad, Word, Outlook and Gmail; undo works; it refuses when the text changed. The bubble follows the field and never takes focus. |
| **1c — AI rewrite** | Ctrl+Alt+Shift+R and the four tones; `s-think`; the result is shown in the island, with apply going through 1b's path. | Rewrite in ≤ ~4.5 s for a typical paragraph on this machine; model calls only through `net::request`. |
| **1d — settings** | Per-app on/off, default tone, strictness (which Harper rule groups run), editing the pause list (from PR #8), the pause hotkey **Ctrl+Alt+Shift+P**, a one-time "Check writing in [app]?" prompt for apps on neither list (§5), and an optional underline overlay. | Settings are kept in `settings.json`; the overlay never takes focus or clicks. |

Each PR is opened, verified and stopped for the owner, never merged without approval.

## 3. The instant checker: Harper

**Harper** (`harper-core`, Automattic, Rust) was tried on 2026-10-10 in a scratch crate (release build, this machine):

- **License, upkeep:** Apache-2.0. The repo has ~16k stars and is not archived, with daily commits and a release every 1–3 weeks. crates.io serves 2.11.0, though GitHub already has a v2.12.0 tag; pin the version when adding it.
- **Offline:** its dependency tree has **no** networking crate (no HTTP client, sockets, tokio or telemetry). The dictionary is compiled in. That gets checked again in the PR (§9), not taken on trust.
- **Speed:**
  - Setup is **~440 ms once** (dictionary and rules), done at startup or when the first supported field gets focus.
  - A check takes **0.3–0.4 ms** for a ~120-character paragraph and **1.2 ms** for 600 characters.
  - That is far inside the 50 ms target, which 1a still measures in the real app.
- **API:** `Document::new_plain_english_curated(text)` and then `LintGroup::new_curated(dict, Dialect::American).lint(&doc)`. Each lint gives:
  - a character **span** (start..end);
  - a **kind** (Spelling, Grammar, Repetition, Capitalization, Punctuation, WordChoice…);
  - a **message**;
  - **suggestions** (replace with text, remove, insert).

  That maps directly onto the suggestion list.
- **What it found** in sample property-management sentences:
  - **Right:** "the the", "recieved", "tomorow", "dont", "its" → "it's", "Their is" → "There is", "friday" → "Friday", "posible".
  - **Missed:** "to you're account" (should be "your").
  - **Wrong:**
    - "unit **4B**" → "Did you mean `bytes`?" (WordChoice);
    - "need to **discus**" → a replacement of "the".
  - **Duplicates:** two or more lints on the same span ("friday" as both spelling and capitalization; "Their" twice, as there and as they're).

**What Harper misses** (a second trial, 20 sample sentences, same day). It checks spelling, repeated words and a set of fixed patterns; it does not understand the sentence:

| Missed | Example |
|---|---|
| Real-word mix-ups | "to **you're** account", "**Your** welcome", "the rent is **to** high", "don't **loose** the key", "we will **except** your payment" |
| "Its" at the start of a sentence | "**Its** going to rain" (caught mid-sentence: "its been cold") |
| Agreement with a compound subject | "the tenant and her son **is** moving out" |
| Verb tense | "Yesterday the plumber **come**" |
| Missing words | "send the invoice **[to]** the owner" |
| Double negatives | "we don't have **no** openings" |
| Run-ons and comma splices | "The lease ends in May, we will send a renewal." |
| A lowercase first word | "**please** call me back." |

It did catch: there/their, then/than, affect/effect, a/an, "the units has", misspellings and repeated words.

**These are expected to be covered by the gemma layer (1c),** which reads the whole sentence. In 1a the instant checker reports only what Harper finds, and the PR says so. Whether 1c runs on demand only (the rewrite hotkey) or also as a slower background check after the instant one is decided in 1c, from measured timings (~2–3.5 s per paragraph on this CPU).

**Plan for 1a:**
- Merge lints that cover the same span into one entry, with the suggestions combined and duplicates removed.
- Drop suggestions that only add or remove spaces.
- Start with a short list of rules switched off (the unit/number WordChoice rule), and a fixture of real-world sentences (unit numbers, addresses, names, amounts) where nothing may be reported.
- 1d's "strictness" turns rule groups on and off.

**Fallback if Harper turns out unsuitable:** LanguageTool would need a local Java server (heavy). Hunspell plus a small rule set would be spelling-only. Any switch is raised with the owner before anything is built.

## 4. Capture in Phase 1

Today the capture thread runs only with `GLIM_DEV=1` and re-reads the focused field every 300 ms for the debug panel. In 1a:

- **It runs in normal sessions**, but only for apps that are on (§5). The debug panel stays dev-only.
- **Every PR #8 rule applies first, unchanged:** paused apps, password fields, payment and ID labels, paused sites, read-only content, and the card, SSN, IBAN and 12+ digit checks. A reading they drop is never checked by Harper and never counted.
- **Debounce:**
  - The thread keeps reading every 300 ms, but in-thread it compares only a hash of the field and the caret position.
  - A check runs once the field has been unchanged for **600 ms** (was 1 s; owner, 2026-10-10). Readings come every 300 ms, so a check lands 600-900 ms after typing stops.
  - While the user keeps typing nothing is sent: the shown suggestions stay as they are until the pause (no flicker; `typing_continuously_sends_nothing_until_the_pause`).
  - It runs again only when the text changes.
  - Moving to another field clears the suggestions.
- **The paragraph:**
  - It is the current paragraph around the caret, as in `caret_window` today (≤500 characters before the caret, ≤200 after).
  - Its offsets in the field are kept, so 1b can find the text again.
- **Signature and quoted text are cut before the check:**
  - signature delimiters (`-- `, a line of `—` / `__`, and lines like "Best regards," / "Thanks," / "Sent from my…" followed by short name/contact lines at the end of the field);
  - Outlook's separators ("From: … Sent: …", the underscore rule);
  - `>`-quoted lines;
  - "On … wrote:" headers.

  A caret inside the signature or the quote means nothing is checked.
- **Context tiers** (from the measured model timings):
  - Grammar is the paragraph only.
  - Rewrite (1c) is the paragraph plus up to ~1,000–1,500 characters of context (subject line, the message being answered). That is ~3.5–4.5 s on this CPU; 5,000 characters is ~10 s, too slow.
  - Quoted text may be *context* for a rewrite, never something Glim corrects.
- **What leaves the capture thread** in normal sessions: only the paragraph (not the whole field) goes to the checker. Harper runs in the Rust process. The island gets the **suggestions**: problem text, message, replacements, offsets. Nothing else.

## 5. Which apps are on

Hardcoded in 1a, editable in 1d. Matched by executable name, any case:

| On by default | Off by default |
|---|---|
| Outlook (`OUTLOOK.EXE`, `olk.exe`) | VS Code (`Code.exe`) and other editors/IDEs |
| Word (`WINWORD.EXE`) | Terminals: Windows Terminal, `cmd.exe`, `powershell.exe`, `pwsh.exe`, `conhost.exe` |
| Chrome (`chrome.exe`) and Edge (`msedge.exe`) — Gmail and web forms | Password managers (already paused by PR #8) |
| Notepad (`Notepad.exe`) | Glim itself (already excluded) |
| Chat apps: Slack, Teams (`ms-teams.exe`), Discord, WhatsApp, Claude desktop, ChatGPT, Grok | |

In browsers, PR #8's paused sites still apply on top of "on".

**Chat apps get relaxed rules** (owner, 2026-10-10; `writing::apps::CHAT`, `checker::Rules::Chat`): Slack, Teams, Discord, WhatsApp, Claude, ChatGPT and Grok skip every Capitalization-kind lint (sentence starts, lowercase "i", the all-caps "canonical spelling" of "u", "btw", "omg") and any finding on a common chat word (`CHAT_WORDS`: lol, idk, tmrw, ok, tho, i'm …). Real misspellings are still found. Outlook, Word, browsers and Notepad keep the full rules.

**Apps on neither list are off** (owner, 2026-10-10). From 1d on, the first time the user types in one, the island asks once: "Check writing in [app]?" with **Yes** / **Not now** / **Never**. Yes and Never are kept per app in settings; Not now asks again next session. In 1a they simply stay off.

**Payment and ID data:** PR 1a's first commit narrows PR #8's 12-digit rule. Digits split by spaces or dashes count only when grouped like a card being typed (4-4-4, 4-4-4-4, Amex 4-6-x), so unit lists such as "Units 101 102 103 104" in AppFolio notes are read. An unbroken run of 12+ digits is still dropped.

## 6. Mascot and island

- **`s-suggest` is shared with agents** (decided by the owner, 2026-10-10): it already shows an agent waiting for an approval or an answer (Claude Code hooks).
  - Agent states always win: approval, question, working, error and rate-limit go first.
  - The writing checker shows `s-suggest` only while no agent state is active.
  - The **count on the pill** tells the two apart: a number means writing suggestions, no number means an agent is waiting.
- **Ember** (decided by the owner, 2026-10-10, after 1a showed nothing in Ember): with suggestions waiting, the ember **brightens** (no bounce) and shows a small **count** at its top right ("9+" past nine; its label reads "Glim: N writing suggestions").
  - **Clicking it opens the island straight on the list.** This is a *peek*: Rust shows the pill without saving a mode (`presence::set_peek`), and when the island closes the ember comes back.
  - Without suggestions the click is unchanged (back to Normal). **Hidden** still shows nothing.
- **The attention pop** plays when the count goes from 0 to more than 0, not on every recount.
- **Island:** a new "Suggestions" view, shown when the lantern is clicked while there are suggestions (otherwise the island opens as today).
  - Each row shows the problem text in context, the message and up to three replacements.
  - **Nearest the caret first**, three rows a page, with **Previous / Next** buttons and "1–3 of 12" (owner, 2026-10-10). Buttons, not scrolling: the island's window never takes focus, so the mouse wheel can't be relied on (the first 1a build cut the list off with no way to see the rest).
  - Read-only in 1a: the only buttons turn pages, and there are no keyboard hooks.
  - Every new string needs its 9 translations.
- **Reserved:** `s-record` and `s-delegate` are never used.

## 7. Write-back (1b)

- Before replacing anything, the field is **read again**, and Glim checks that the target text at the stored offsets is still exactly what was checked. If not, it refuses and rechecks.
- **Select** the range via TextPattern (`Select()` on the range), then **replace** it by typing the replacement with `SendInput`, so the app's own undo works. **Never `ValuePattern.SetValue`**: it bypasses undo, and in many apps it replaces the whole field.
- The replacement is typed only in response to the user's click on that suggestion, and only into the field it came from. If focus moved, nothing is typed.
- If the app gives no TextPattern for the field (ValuePattern-only), 1b offers copy-to-clipboard instead of typing.

## 7a. The floating bubble at the field (1b, owner 2026-10-10)

A Grammarly-style marker where the user is typing, so suggestions don't need a trip to the top of the screen:

- **A small amber count bubble anchored to the edge of the focused text field** (bottom right, inside the field's bounds), placed from the field's and the caret's bounding rectangles (UIA `BoundingRectangle`, TextPattern `GetBoundingRectangles` on the caret range). It shows the count only, no text.
- **Clicking it opens the suggestions in a popover right there**, next to the bubble (the same rows as the island's list, with paging). In 1b, clicking a fix applies it in place through §7's path.
- **Never steals focus:** a separate, non-activating (`WS_EX_NOACTIVATE`), topmost, tool window; the popover too. The text field keeps focus and the caret throughout.
- **Hidden** in password fields, read-only content and everything PR #8 skips (it only exists when the checker has suggestions, which those fields never produce), when the field loses focus, and while there are no suggestions.
- **Follows the field** as it moves or scrolls (re-placed on each reading, every 300 ms, and on UIA bounding-rectangle changes), and hides when the field is off screen or covered by a fullscreen app.
- To check in 1b: per-monitor DPI, fields inside browser pages (bounds in screen pixels), multi-line fields where the caret's rectangle is the better anchor, and that the bubble never covers the text being typed.

## 8. Settings (1d) and Windows' own suggestions

- **Per-app on/off**, the default rewrite tone, and strictness (Harper rule groups).
- **The pause list** from PR #8, made editable.
- **The pause hotkey Ctrl+Alt+Shift+P** ("not now"), registered and checked for conflicts the same way as the other Glim hotkeys (Settings warns if it can't register).
- **The one-time "Check writing in [app]?" prompt** for apps on neither list (§5).
- **Optional underline overlay:** a click-through, non-activating window that draws under the problem spans, placed using the TextRange bounding rectangles. It is redrawn on scroll and move, and hidden when the field loses focus.
- **Windows' text suggestions** (Settings → Time & language → Typing → "Show text suggestions when typing on the physical keyboard") compete with Glim for the same moment and spot on screen. Onboarding detects the setting and suggests turning it off. If the user keeps it, Glim's UI stays out of its way.

## 9. Privacy

- **Captured text stays in memory** for one check and is dropped. It is never logged, never stored, and has no history or cache. **Suggestions contain text too:** they get the same rule (in memory, panel and island only; never in glim.log or the results log).
- **Network:** Harper runs in-process. Model calls (1c) go only through `windows/src-tauri/src/net/mod.rs` (localhost only), with `keep_alive` so the model stays loaded while the assistant is on.
- **Checked empirically, not trusted:** with 1a running, netstat over Glim's whole process tree (`glim.exe` and its `msedgewebview2.exe` children) during checks shows no connection. The existing guard test still forbids sockets outside `src/net/`.
- **PR #8's filters run before the checker.** Their tests stay and new ones are added: a reading they drop never produces a suggestion.

## 10. Tests and verification for 1a

- **Unit tests:**
  - signature and quote exclusion (Outlook, Gmail and plain-text samples);
  - debounce (no check while the text keeps changing; one check 1 s after it stops; none on an unchanged poll);
  - per-app on/off;
  - merging lints that share a span;
  - the "nothing reported" fixture of real-world sentences;
  - suggestion data never in logs (the same source scan and sentinel tests as capture).
- **Performance:** median and max check time per paragraph length, measured in the release build on this machine, reported in the PR.
- **Self-test, without the owner's mouse or keyboard:**
  - Glim launches Notepad with a known misspelled sentence and moves focus with UIA SetFocus, only after 4 s of user idle.
  - It confirms the count on the pill and the island's list. Glim's own windows are captured with PrintWindow only.
- **Then the owner tries it in Outlook, Word and Gmail.**

## 11. Decisions (owner, 2026-10-10)

1. **`s-suggest`:** agent approval keeps priority; the pill count shows writing suggestions (§6).
2. **Apps on neither list:** off by default, plus the one-time "Check writing in [app]?" prompt (yes / not now / never) in 1d (§5).
3. **Edge:** on by default, like Chrome.
4. **Pause key:** Ctrl+Alt+Shift+P, checked for conflicts like the other hotkeys (1d).
5. **Dialect:** American English only for now.
6. **12-digit rule:** narrowed to card-shaped groups as PR 1a's first commit (§5).
7. **Harper's misses** are left to the gemma layer (1c) (§3).
