# Phase 1 — writing assistant: plan

Status: **reviewed by the owner 2026-10-10**; the answers are in §11. PR 1a merged 2026-10-11 (PR #9). **§7 (1b, applying fixes) approved by the owner 2026-10-11** with the answers in §7.9; 1b-apply merged 2026-10-11 (PR #12, `b99651b`). **Direction change 2026-10-11:** AI features use the Claude API by default, not a local model; see §12 and `docs/vision.md`. Inputs: the owner's Phase 1 brief, `docs/capture-results.md` (the capture spike and the model timings), PR #8 (payment and ID filtering), and a Harper trial run on this machine (below).

If a step here turns out wrong while building, this file gets a warning note next to that step; the code is not quietly worked around it.

## 1. What the user sees

1. They type in a supported app (Outlook, Word, Gmail in Chrome, Notepad, a chat app).
2. About **0.6 s after they stop typing** (owner, 2026-10-10: 1 s felt slow), Glim checks the paragraph the caret is in. Only the text they are writing counts: the email signature and quoted reply text are left out.
3. **Issues found:** the lantern goes to **`s-suggest`** with the attention pop, and the pill shows the **count** ("3").
   **No issues:** the lantern goes back to **`s-listen`** (island open) or `s-idle` (in the notch), and the count disappears.
4. **Clicking the lantern** (or the ember) opens the island on a **suggestion list**, in reading order from the start of the paragraph, three at a time with Previous / Next. Each entry shows the problem text, a short explanation and the replacement options.
5. **1b:** clicking a replacement applies it in the app, with native undo.
6. **1c:** **Ctrl+Alt+Shift+R** sends the paragraph (or the selection) to the **Claude API** (a fast Haiku-class model by default; §12) with a tone: clearer, friendlier, more formal or shorter. The lantern shows **`s-think`** while it generates. *(Was the local `gemma3:4b`; changed 2026-10-11.)*

Glim never takes focus at any point (the island, panel and pill are non-activating, as today).

## 2. PR split

| PR | Scope | Done when |
|---|---|---|
| **1a — instant checker** | Capture runs in normal sessions for apps that are on. 600 ms debounce. Paragraph extraction without the signature or quote. Harper. Lantern `s-suggest` + count. A **read-only** suggestion list in the island. Per-app defaults hardcoded. | Notepad self-test passes; <50 ms per check measured; tests below pass; the owner tries it in Outlook, Word and Gmail. |
| **1b — apply fixes** | Clicking a replacement writes it back (design in §7). The floating bubble at the field (§7a); proposed as a separate PR, §7.9. | The write-back is verified in Notepad, Word, Outlook and Gmail; undo works; it refuses when the text changed. The bubble follows the field and never takes focus. |
| **1c — AI rewrite** | Ctrl+Alt+Shift+R and the four tones, sent to the Claude API (§12); `s-think`; the result is shown in the island, with apply going through 1b's path. Settings: API key (Windows Credential Manager), model per task, this month's spend, optional monthly cap. `api.anthropic.com` added to the checkpoint. | Rewrite in ≤ ~4.5 s for a typical paragraph (measured on the Claude path); model calls only through `net::request`; a request to any other host still refused; the card/SSN/IBAN scan runs on every request body; nothing sent without the user's action; no request or response text in any log. |
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
| Wrong-but-real words | "**He** how many times…" for "Hey how…" (owner's retest, 2026-10-10) |

> **Out of scope for the instant checker:** wrong-but-real-word errors such as "He how" → "Hey how". Every word is in the dictionary and no fixed pattern matches, so Harper (and any spell checker) can't see them. They are a **1c (AI rewrite)** case: the model reads the sentence. The same holds for the real-word mix-ups in the table above.

It did catch: there/their, then/than, affect/effect, a/an, "the units has", misspellings and repeated words.

**These are expected to be covered by the AI layer (1c),** which reads the whole sentence. In 1a the instant checker reports only what Harper finds, and the PR says so.

> ⚠️ **Amended 2026-10-11 (direction change, §12):** the AI layer is the Claude API, not gemma. It runs **on demand only**: the background-check option is dropped, because nothing is sent without the user's action.

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
  - **The suggestions belong to the last field that was checked** (owner, 2026-10-10). They stay while the user is anywhere nothing is checked (another app, Glim itself, a password field), and they are still there, with no gap, when the user comes back to that field. Another checked field replaces them with its own check about 0.6 s later; a checked field with nothing to check (a blank paragraph, the caret in the signature) clears them.
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
    > ⚠️ **Amended 2026-10-11:** those timings are gemma3 on this CPU. 1c now uses the Claude API (§12); the context limit is set again from timings measured on that path. Context is sent only with the user's action, after the same scan.
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
  - **Clicking it opens the island straight on the list** (a waiting approval or question still comes first). This is a *peek*: Rust shows the pill without saving a mode (`presence::set_peek`), and when the island closes the ember comes back.
  - Without suggestions the click is unchanged (back to Normal). **Hidden** still shows nothing.
- **The attention pop** plays when the count goes from 0 to more than 0, not on every recount.
- **What the lantern opens** (owner, 2026-10-10; `State.lanternView`): while the badge shows writing suggestions, a click on the lantern (the closed pill's, the open island's, or the ember) opens the suggestions, even when an agent's finished or error card has taken the island. A waiting **approval or question still comes first** (urgent). Agent cards stay one click away under Home.
- **Island:** a new "Suggestions" view, shown when the lantern is clicked while there are suggestions (otherwise the island opens as today).
  - Each row shows the problem text in context, the message and up to three replacements.
  - **In reading order** (left to right, top to bottom, from the start of the paragraph), three rows a page, with **Previous / Next** buttons (32×28 px, amber) and "1–3 of 12" (owner, 2026-10-10). The first round sorted nearest the caret, which put an early typo ("timez") on page 2 where it looked missed. Buttons, not scrolling: the island's window never takes focus, so the mouse wheel can't be relied on (the first 1a build cut the list off with no way to see the rest).
  - Read-only in 1a: the only buttons turn pages, and there are no keyboard hooks.
  - Every new string needs its 9 translations.
- **Reserved:** `s-record` is never used. `s-delegate` is used only while text is going to the Claude API (from 1c, §12) or, later, Phase 3 delegation.

## 7. Write-back (1b)

Status: **approved by the owner 2026-10-11** (answers in §7.9); built as **1b-apply** on `phase-1b-apply`, the bubble (§7a) follows as its own PR. This section replaces the four-line sketch that was here. 1b types into the owner's real apps, so it is written to **fail closed**: at each step it either proves the text is what was checked or does nothing.

### 7.1 The rules (all of them, every apply)

1. **Only on a click.** A fix is typed only when the user clicks one replacement in Glim (the island list, or the bubble's popover in 7a). Nothing is applied automatically, in bulk, or from a keyboard shortcut in 1b.
2. **Only Glim's own words.** The island sends back **which** fix was clicked (check id, row, replacement index), never the text to type. Rust looks the replacement up in what the checker produced and kept. A webview message can't make Glim type arbitrary text.
3. **Only into the field it came from**, and only while that field still has focus. Glim never moves focus or brings a window forward to apply a fix.
4. **Only if the text is unchanged.** The field is read again and the checked paragraph must still be there, word for word, before anything is selected; the selection is read back before anything is typed; the result is read back after.
5. **Every PR #8 rule runs again at apply time** on the field as it is now, not as it was when it was checked (§7.5).
6. **Native undo only.** Glim types the replacement over a selection with `SendInput`, the same as the user typing it, so the app's own Ctrl+Z undoes it. Never `ValuePattern.SetValue` (bypasses undo; many apps replace the whole field), never the clipboard (except the Copy button of §7.7, which the user clicks), never a scripted Ctrl+Z.
7. **When unsure, refuse and recheck.** Every refusal leaves the field untouched, says so in one line in the island, and runs the check again at once so the list matches the field.

### 7.2 What Glim keeps between the check and the click

Today (1a) the paragraph is dropped right after the check (§9). 1b needs it to find the text again, so **while its suggestions are showing**, the capture thread keeps, in memory only:

- the checked paragraph's text (≤ ~700 characters, not the whole field) and a hash of it;
- the field's identity: process id, top-level window handle, and the UIA element (or its RuntimeId);
- the suggestions with their replacements, under a **check id** that changes on every new check.

**When it is dropped** (owner, 2026-10-11; each one has a test):
- the suggestions are **dismissed** (a Dismiss button on the list; new in 1b-apply), replaced by a new check, or cleared;
- the user **moves to different text**: focus goes to another field or app, the caret moves to another paragraph, or the paragraph is edited. The list may stay on screen (1a's rule), but its fix buttons say "Click in the field first" until the field is checked again;
- **Glim is paused** (the tray's Pause; the island tells Rust, which also stops checking while paused);
- **10 minutes** after it was kept, at most (the field is then checked again if the user is still in it);
- on quit.

**The sensitive-data check runs before it is kept:** the paragraph goes through the card / SSN / IBAN / 12+ digit scan again (on top of the reading's own scan); if it hits, nothing is kept and no suggestions are shown.

Only fields where fixes can be typed keep anything: in view-only apps and Copy-only fields (§7.7) nothing is kept.

Never logged, never written to disk; the same source-scan and sentinel tests as 1a cover it. **This is a change to §9's "dropped after one check" and is called out in the PR.**

The island's click carries the check id. A click on a list that is no longer current (a newer check arrived in between) is refused: "The text changed; rechecked."

### 7.3 Re-reading and confirming the text, step by step

All of this runs on the capture thread (it owns every UIA object), triggered by a message from the island. No step reuses a range or an offset from the check without reading it back.

1. **Gate.** Glim not paused; the check id is current; the foreground window and its process are the stored ones; `GetFocusedElement` is the stored element (`CompareElements`). If the user is in another app, nothing happens: the row says "Click in the field first" (§7.6).
2. **PR #8 gate, again** (§7.5) on the focused element.
3. **Find the paragraph by its content, not its offsets.** From the caret range (TextPattern2 `GetCaretRange`, else the selection), widen to a window around it (the paragraph unit where the app supports it, else ±2,000 characters, capped like today's `MAX_FIELD_CHARS`), and `FindText` the checked paragraph, case-sensitive. It must match **exactly once**. Zero → the text changed: refuse. Two or more (the same paragraph pasted twice) → pick the one whose position matches the stored offset; if none does, refuse.
   - Why not offsets: our offsets count Rust characters of `GetText`'s string; UIA's "character" unit is provider-defined (Word counts hidden field codes, Chrome counts embedded objects, emoji are two UTF-16 units). Offsets are only a tie-breaker.
   - Edits elsewhere in the field (an earlier paragraph, the subject) don't block a fix; any edit inside the paragraph does.
   - > ⚠️ **Amended while building (2026-10-11):** `FindText` is not used. Word's search is limited to short strings (a paragraph can be 700 characters), and the text has to be read back anyway. Instead: the caret's range is widened to its paragraph (`ExpandToEnclosingUnit(Paragraph)`), its text is read, and the checked paragraph is located in that text in Rust (exactly once; else the whole document's text is searched the same way, with the stored offset as the tie-breaker). The ranges are then built from there and **read back** as in step 4, so a unit mismatch still refuses.
4. **Narrow to the problem.** Inside the found paragraph range, move the start and end by the problem's position, then **read the range back**: `GetText` must equal the problem word exactly, and the characters just before and after must match the stored context. Any difference (a unit mismatch in that app included) → refuse. A miscounted range can never be typed over, because it is read back first.
5. **Select it** with `IUIAutomationTextRange::Select()`, then read the app's selection back (`GetSelection`): exactly one range, text equal to the problem word. Apps whose `Select()` doesn't move the real selection fail here and are refused (and logged by app name, so they go on the "unsupported" list).
6. **Last checks, immediately before typing:** focus and foreground unchanged; no modifier key held (Ctrl, Alt, Shift, Win via `GetAsyncKeyState`: a held Ctrl would turn typed letters into shortcuts); no mouse button held. If a key is held, wait up to 500 ms for release, else refuse.
   - > ⚠️ **Amended while building (2026-10-11):** the IME check is not possible from another process: `ImmGetContext` returns nothing for another app's window. An open IME composition (Japanese, Chinese input) is not detected; the read-back in step 8 catches a fix that the IME swallowed. The owner uses en-US only.
7. **Type the replacement** in **one** `SendInput` call (`KEYEVENTF_UNICODE` down/up per UTF-16 unit). One call means Windows doesn't interleave the user's own keys with ours. A removal (Harper's "remove" suggestion) is a single Backspace over the verified selection.
   - The replacement must be printable text only: no newline, tab, or control characters. In chat apps Enter **sends the message**, so a replacement containing one is dropped at check time and never offered.
8. **Read back the result.** Re-read the paragraph: it must equal the stored paragraph with exactly that one span replaced. Then put the caret back where the user had it (a degenerate range `Select()`, shifted by the length change if it was after the fix), so they keep typing where they were. Then recheck at once (no 600 ms wait): the remaining suggestions' positions moved.
9. **If the read-back doesn't match**, Glim does **not** try to repair it or send Ctrl+Z (that could undo something else). The island says: "That didn't apply cleanly. Press Ctrl+Z in [app] to undo it." The outcome is logged as a code with the app name, never the text.

Steps 1-7 take a few milliseconds; the gap between the last check (6) and the keys landing (7) is the only window where the user could change focus, and step 8 catches it if they do.

### 7.4 Undo in each app

Typing over a selection is what the user would do by hand, so undo is the app's own:

| App | Editor under UIA | Expected Ctrl+Z | To verify in 1b |
|---|---|---|---|
| **Word** (`WINWORD.EXE`) | Word's own provider | One Ctrl+Z restores the original word ("Typing" is one undo step). The Undo list shows "Typing". | AutoCorrect: we type no trailing space or punctuation, so it shouldn't fire; confirm. Track Changes on: the fix shows as a tracked edit, as a hand edit would. |
| **Outlook classic** (`OUTLOOK.EXE`) | The compose body is Word's editor | Same as Word, inside the message. | Same as Word; also the subject line (a plain edit box, its own undo). |
| **New Outlook** (`olk.exe`) | WebView2 (Chromium) | Like Gmail. | Same checks as Gmail. |
| **Gmail in Chrome / Edge** | Chromium `contenteditable` | Chromium groups the typed characters with the replaced selection: one Ctrl+Z puts the original word back. Gmail uses the browser's undo for typing. | Confirm one press is enough (not one per character); confirm Gmail's own "Undo" banner isn't involved (it's for send/delete only). |
| **Notepad** (Win11) | RichEdit-based | Multi-level undo; one Ctrl+Z restores the word. | Confirm. |
| **Chat apps** (Slack, Teams, Discord…) | Electron / WebView2 | Usually Chromium's undo; some editors (Slack's) keep their own. | Each is tested before it's turned on for fixes. Until then, chat apps show suggestions but no apply button (§7.6). |

Glim shows no undo button of its own in 1b: an undo from Glim would have to send Ctrl+Z, which could undo the user's own next edit. After a fix, the island's one-line note says "Fixed. Ctrl+Z in [app] undoes it."

If an app turns out to need two Ctrl+Z presses (one for the typing, one to restore the selection), the note says so for that app, and the plan gets a warning note here.

### 7.5 Never touching sensitive or paused fields

Suggestions only ever exist for fields that passed PR #8's filters, but the apply step doesn't rely on that. At apply time, on the focused element **as it is now**:

- Glim's **Pause** (tray) and, from 1d, the **pause hotkey**: Rust keeps its own copy of the paused flag (today Pause lives in the island's state only) and refuses while paused. Pausing also drops the kept paragraph (§7.2).
- The **app** is on (§5), not on the **pause list** (password managers).
- **Password field** (`IsPassword`): refuse.
- **Payment and ID labels** (`is_sensitive_field`): refuse.
- **Paused sites** in a browser (`page_urls` against the pause list, frames included): refuse.
- **Read-only** content (`is_editable`): refuse.
- The re-read paragraph **and the replacement** go through the card / SSN / IBAN / 12+ digit checks (`finish_reading`); either one hitting → refuse.
- **Elevated apps** (running as administrator): Windows blocks a normal app's `SendInput` into them (UIPI). Detected up front where possible, and in any case caught by the read-back (nothing changed → "Glim can't type into apps running as administrator").

Every refusal is silent in the target app (nothing typed, nothing selected — selection only happens after these gates pass) and gives one line in the island.

### 7.6 What the user sees

- Each suggestion row gets its replacements as buttons (up to three). Clicking one applies it; the row shows "Fixing…" for the few milliseconds it takes, then the list refreshes from the recheck.
- **While the source field isn't focused** (the user is in another app; suggestions persist there since 1a), the buttons are disabled with "Click in the field first". Glim never switches apps for them.
- **Apps not yet verified for fixes** show suggestions read-only, as in 1a (owner, 2026-10-11). Fixes are on in the 1b test matrix only: **Notepad, Word, Outlook classic, Chrome (Gmail)**. View-only until each is tested: Edge, new Outlook (`olk.exe`), and the chat apps (Slack, Teams, Discord, WhatsApp, Claude, ChatGPT, Grok). The list is in PROJECT_STATUS.
- **Dismiss** on the list clears the suggestions (and the kept paragraph) until the text changes.
- Refusals: "The text changed; rechecked." / "That field is paused." / "Glim can't type into apps running as administrator." Every new string gets its 9 translations.

### 7.7 Fields without TextPattern

Some fields give only ValuePattern (plain inputs in some apps): Glim can't select or type there safely. **Decided (owner, 2026-10-11): view-only, with a Copy button.** The suggestions show as usual; each replacement is a **Copy** button that puts that word on the clipboard **only when the user clicks it** (never automatically). Nothing is typed, and no paragraph is kept for these fields. The copied word goes into Windows' clipboard history like any copy.

### 7.8 Tests and verification for 1b

- **Unit tests** (pure Rust, a fake field behind a trait, no UIA): paragraph found once / zero / twice; an edit inside the paragraph refuses; an edit elsewhere doesn't; unit mismatch caught by the read-back; a stale check id refuses; a replacement with a newline or digits that look like a card is never offered; modifier held → wait then refuse; paused → refuse; the island can only send ids (a message with text is rejected); kept paragraph never in logs (source scan + sentinel).
- **Read-only self-test in Notepad** (no keys typed): the list shows fix buttons for a Notepad field; with focus elsewhere they say "Click in the field first"; Dismiss clears the list.
- **Typed self-test, Notepad only** (fresh file per run, as in 1a): apply "recieved" → "received", read back, Ctrl+Z restores it. **This sends real keystrokes**, so per CLAUDE.md it runs **only after the owner confirms in chat they're away from the keyboard**, with the screenshot check first.
- **Owner test matrix:** Word, Outlook classic (body + subject), Gmail in Chrome, Notepad; each: apply, undo with one Ctrl+Z, apply after editing the paragraph (must refuse), apply from another app (button disabled), a password field and a paused site (no suggestions, no apply).
- **Network:** unchanged (no new network code); re-checked with netstat over the process tree, as in 1a.

### 7.9 Decisions (owner, 2026-10-11)

The design was approved as written, with these answers:

1. **Split:** 1b-apply first (this section), then the floating bubble (§7a) as its own PR.
2. **No undo button in Glim.** The app's own Ctrl+Z is the undo.
3. **ValuePattern-only fields:** suggestions view-only. Never copy automatically; a **Copy** button the user clicks is fine (§7.7).
4. **Chat apps:** view-only until each one is tested; the list is kept in PROJECT_STATUS (§7.6).
5. **The caret goes back** to where the user was typing (§7.3 step 8).
6. **Privacy (added by the owner):** the kept paragraph is dropped when the suggestions are dismissed, when the user moves to different text, when Glim is paused, and after 10 minutes at most; the sensitive-data check runs before it is kept; a test for each (§7.2).
7. **Testing:** unit tests and read-only checks first. The typed self-test runs in **Notepad only**, only after the owner says in chat that they're away from the keyboard. The owner tests Outlook, Word and Gmail by hand.

## 7a. The floating bubble at the field (1b, owner 2026-10-10)

A Grammarly-style marker where the user is typing, so suggestions don't need a trip to the top of the screen:

- **A mini version of the Glim lantern with a count badge** (owner, 2026-10-10: not a plain dot), anchored to the edge of the focused text field (bottom right, inside the field's bounds). It is the real lantern, ported from `docs/brand/glim-mascot.html` like the others (never redrawn), at a small size, in `s-suggest`, with the same amber badge as the pill. It is placed from the field's and the caret's bounding rectangles (UIA `BoundingRectangle`, TextPattern `GetBoundingRectangles` on the caret range). It shows the count only, no text.
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

- **Captured text stays in memory** for one check and is dropped (from 1b, the checked paragraph is kept in memory while its suggestions show, so a fix can find it again: §7.2). It is never logged, never stored, and has no history or cache. **Suggestions contain text too:** they get the same rule (in memory, panel and island only; never in glim.log or the results log).
- **Network:** Harper runs in-process. Model calls (1c) go only through `windows/src-tauri/src/net/mod.rs`.
  > ⚠️ **Amended 2026-10-11 (§12):** the allowlist becomes localhost + `api.anthropic.com` (https, 443). The Ollama `keep_alive` note applies only to the optional offline fallback.
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
7. **Harper's misses** are left to the gemma layer (1c) (§3). *(The AI layer is now the Claude API; §12.)*

## 12. Direction change: the Claude API (owner, 2026-10-11)

The owner is no longer prioritizing local-only AI. Full statement: `docs/vision.md`. What it means for Phase 1:

1. **1c rewrite uses the Claude API.** A fast, cheap **Haiku-class** model by default; a stronger one can be picked per task in Settings. Model IDs and prices come from Anthropic's docs at build time.
2. **API key:** entered in Settings and stored in **Windows Credential Manager** (no new crate; the `windows` crate's credential calls). Only Rust reads it; it never goes back to a webview, a log or `settings.json`. The PR calls out that credential storage is back (Phase 0a removed it).
3. **Network:** `net::ALLOWED_HOSTS` goes from localhost only to **localhost + `api.anthropic.com`** (https on 443 only), still through the single checkpoint in `net/mod.rs`. The webviews stay localhost-only; API calls come from Rust. Verified empirically: a request to another host fails, and netstat over the process tree shows nothing else.
4. **Kept:** the card/SSN/IBAN/payment filtering before anything leaves capture, and again on every request body just before it is sent; **send only on the user's click**; no logging of captured text (the log gets outcome, model, tokens and cost only). New powers start as "suggest and wait for my click"; the rewrite's result is a suggestion and is typed only through 1b's apply path, on a click.
5. **Spend:** Settings shows this month's API spend (Glim's own estimate from the token counts) with an **optional monthly cap**. At the cap nothing is sent and the island says why.
6. **The "brain" setting:** 1c's Settings picks the brain, **Claude API (default)** or **local model** (gemma3 through Ollama), switchable at run time without rebuilding. Both go through `net::request` and the same pre-send scan. The local path is the optional offline fallback; nothing new is built for it before the Claude path works.
   - **Local model benchmarking is deferred** until the owner's next laptop (32 GB+ RAM). This one has 16 GB and is already memory-tight, so the API stays the default and the gemma3 timings in §4 are not re-measured here.
7. **1d** gains the AI settings above (brain, key, model per task, spend and cap).

**Decided (owner, 2026-10-11):**
1. **The rewrite hotkey counts as the owner's request** to send that paragraph to the API. The result still shows as a suggestion and is typed in only when the owner clicks it (1b's apply path).
2. **`s-delegate` shows while text is going to the API.** The rule changed in CLAUDE.md, the design file's header and PROJECT_STATUS. The code side changes in the 1c PR: `RESERVED_LANTERN_STATES.delegate`'s owner (now `"phase3-delegation"`) and the allowed callers in `tests/lantern.test.mjs` (`SHOWS_RESERVED`).
3. **CLAUDE.md's network rule** now matches `docs/vision.md`: localhost + `api.anthropic.com`, through `net/mod.rs`.

## 13. 1c design: the Claude API rewrite (2026-10-12)

Follows `docs/vision.md` §2–3 and §12 above. Branch `phase-1c-rewrite`, stacked on `post-1b-fixes` (PR #15). Where this section and an older one disagree (§1 item 6 says `s-think`, §2's 1c row lists "clearer"), this section wins.

### 13.1 What the user sees

1. The caret is in a paragraph of an app the checker is on (Outlook, Word, Gmail, Notepad…).
2. **Ctrl+Alt+Shift+R**, or the **Rewrite** button on the island's suggestion card, opens the **Rewrite card** in the island with four tones: **Friendlier**, **More formal**, **Shorter**, **Fix grammar**. Nothing has been sent yet. The island never takes focus, so the caret stays in the field.
3. **Clicking a tone is the request to send** (§12 decision 1: the user's own action). That one paragraph goes to the Claude API, the same paragraph the checker uses (signature and quoted text left out). The lantern is in **`s-delegate`** from the moment the request leaves until the answer is in (vision §3). The card says "Rewriting with Claude…".
4. The answer is shown **as a suggestion**: the rewritten paragraph, with **Use this** (fields where fixes are typed), **Copy** (fields Glim can't type in) or plain text (view-only apps), and **Dismiss**. Under it: the model and this request's estimated cost.
5. **Use this** types it in through 1b's apply path (§7.3), unchanged. Only the rewrite's id goes from the island to Rust. Rust re-verifies that the field still holds the paragraph as it was sent, replaces the whole paragraph, and reads it back. Ctrl+Z in the app undoes it. If the paragraph changed in the meantime, nothing is typed ("The text changed…").
6. If nothing can be sent, the card says why instead: no API key (Settings → AI), the monthly cap is reached, Glim is paused, there is no paragraph here, the paragraph is over 2,000 characters, it looks like a card, SSN or IBAN, or the brain is set to the local model (not built yet, see 13.8).

**Hotkey decision:** the hotkey opens the card and the tone click sends. That is one click more than §12 decision 1 allows (the hotkey alone could send with a default tone). A "default tone, send on the hotkey" option belongs with 1d's default-tone setting. **Owner: say if you want the one-key version now.**

### 13.2 The API request (checked against Anthropic's docs 2026-10-12)

Sources: platform.claude.com `about-claude/models/overview`, `about-claude/pricing` and `api/errors`, read 2026-10-12. None of these values come from memory.

| Setting | Model ID | Input $/MTok | Output $/MTok | Note |
|---|---|---|---|---|
| **Haiku 5.5 (default)** | `claude-haiku-5-5` | 0.10 | 0.50 | For prompts up to 100,000 tokens; above that, 0.50 / 2.50. A rewrite is far below. |
| Sonnet 5.5 | `claude-sonnet-5-5` | 2.00 | 10.00 | |
| Opus 5.5 | `claude-opus-5-5` | 4.00 | 20.00 | |

- **Endpoint:** `POST https://api.anthropic.com/v1/messages`, with headers `x-api-key`, `anthropic-version: 2023-06-01` and `content-type: application/json`. Rust calls it directly (there is no official Rust SDK), only through `net::request`.
- **Body:**
  - `model`, `max_tokens: 2048` and `output_config: {effort: "low"}` (fast and cheap; all three models think adaptively and accept `low`).
  - A fixed `system` prompt: rewrite only the text in the tags; keep names, numbers, dates, addresses and facts exactly; return one paragraph of plain text with no line breaks, no quotes and no preamble; if nothing needs changing, return the text as it is.
  - One user message holding the tone instruction and the paragraph inside `<text>` tags.
  - Left out: `thinking` (the models' default is used), streaming (a paragraph comes back in one piece), tools, and prompt caching (the prompt is under the minimum cacheable size).
- **Answer:**
  - The text is the `text` blocks of `content`, joined and trimmed.
  - `stop_reason` `end_turn` is a result, `max_tokens` means "too long to rewrite", and `refusal` means "Claude declined to rewrite this".
  - An answer with line breaks or control characters, or one the card/SSN/IBAN scan flags, is shown with Copy only and never typed (`writing::offerable`).
- **Cost:** `usage.input_tokens × input price + usage.output_tokens × output price`, at the prices of the model the response names, in micro-dollars. Thinking tokens are billed as output and are already in `output_tokens`.
- **Errors**, by status and `error.type`:
  - 401 `authentication_error`: Anthropic refused the API key.
  - 402 `billing_error`: a billing problem on the Anthropic account.
  - 403 `permission_error`.
  - 413: the request is too large.
  - 429 `rate_limit_error`: shows the wait from `retry-after` when there is one. A spend-cap 429 has none.
  - 500 `api_error`, 504 `timeout_error`.
  - 529 `overloaded_error`: "Claude is busy, try again".
  - Network and TLS failures.

  Only the status and `error.type` are logged. **There are no automatic retries:** each try costs money, and the user can click again.
- **Not used yet: server-side refusal fallbacks** (`fallbacks`). Haiku has none. On Sonnet and Opus a fallback re-runs the request on another model, so the cost estimate would have to use that model's prices. **Owner: on or off for Sonnet/Opus?**
- **Timeout:** 20 s for the whole request, 10 s to connect. Plan §2's target is about 4.5 s; it gets measured with the owner's key, not in tests.

### 13.3 The brain setting

- `settings.json` gains `aiBrain`: `"claude"` (default) or `"local"`. It can be switched at any time, with no rebuild.
- `"local"` is the offline fallback: gemma3 through Ollama, on localhost through `net::request`. **Nothing is built for it in this PR** (§12.6: not before the Claude path works). With it selected, the Rewrite card says "Rewrite with the local model isn't built yet. Choose Claude API in Settings."
- `aiModel` (default `claude-haiku-5-5`) picks the model for 1c. Later tasks get their own settings.

### 13.4 The API key: Windows Credential Manager

- **Storage:** a **generic credential**, target `Glim/AnthropicApiKey`, persistence `LOCAL_MACHINE` (this user on this PC, never roamed). It goes through the `windows` crate's `CredWriteW` / `CredReadW` / `CredDeleteW` (feature `Win32_Security_Credentials`), with no new crate. **This adds credential storage back on purpose** (Phase 0a removed the `keyring` crate), and the PR says so.
- **Entering it:** Settings sends the key to Rust once, when the user saves it (`ai_key_set`). That is the only time it passes through a webview. From then on Settings gets only `{set, hint}`, where the hint is `sk-ant-…` plus the last 4 characters. **Remove** deletes the credential.
- **Reading it:** only the network call reads it, just before sending. It is held in a type whose `Debug` prints nothing. It is never logged, and never written to `settings.json`, the log or any event. A source test checks that nothing logs it.
- **Tests:** the store is a small trait with an in-memory fake, so tests never touch the real Credential Manager. One `#[ignore]` test does a real write, read and delete for a manual run.

### 13.5 Monthly spend and the cap

- **What is stored:** `spend.json` in Glim's config folder holds `{month: "2026-10", micros, requests, inputTokens, outputTokens}`, by local time. Only numbers, never text. A new month starts again at zero.
- **The cap:** `aiMonthlyCapUsd` in `settings.json`, where 0 means no cap.
  - Before a request is sent, if this month's estimate is already at or over the cap, nothing is sent. The card says "Monthly cap of $X reached; nothing more is sent this month."
  - The check runs before each request, so the cap can be passed by at most one rewrite's cost (about $0.0005 on Haiku).
- **Settings shows this month's spend**, with the note "Glim's own estimate from token counts. Your Anthropic Console bill is the real number." The Console has its own hard spend limit, and setting one there is recommended too.

### 13.6 The network checkpoint change (`net/mod.rs`)

- **Hosts:** `ALLOWED_HOSTS` stays loopback only, with any scheme and port as today. The new `ANTHROPIC_HOST = "api.anthropic.com"` is allowed **only with `https` on port 443** (no explicit port, or 443). There is no other Anthropic host and no wildcard, so `x.api.anthropic.com`, `api.anthropic.com.evil.com`, `http://` and `:8443` are all refused.
- **Redirects:** every hop passes `check` as before. In addition, a hop from a non-loopback origin may not change host, so Anthropic can't redirect anywhere else, not even to localhost.
- **TLS:** rustls with its bundled roots, with certificate checks on. A source test forbids `danger_accept_invalid_certs` and `danger_accept_invalid_hostnames`.
- **Unchanged:** proxies are never used, and the guard test (no sockets or `reqwest` outside `src/net/`) stays as it is.
- **Tests use a fake local server.** The client takes the base URL as a parameter: `https://api.anthropic.com` in the app, `http://127.0.0.1:<port>` in tests. **No test calls the real API.**
- **Empirical check (owner's live test):** a request to another host is refused, and during a rewrite netstat over `glim.exe` and its `msedgewebview2.exe` children shows only loopback and Anthropic's addresses.

### 13.7 Before anything is sent, and while it is

- **Where the paragraph comes from:** a field that already passed the payment-field label check and PR #8's text scan.
- **The scan just before sending:** the serialized request body is scanned again with `capture::sensitive::contains_sensitive_number`, immediately before `net::request`. A hit sends nothing ("Not sent: this looks like it has a card, SSN or bank number in it.").
- **Only on a click:** a request can only start from the tone click, an island command that carries the tone id and nothing else. Rust takes the paragraph from its own capture state, never text from a webview.
- **Logs:** lines such as `rewrite: ok claude-haiku-5-5 in=212 out=187 $0.000115 1.9s`, `rewrite: refused (cap)` or `rewrite: error 529 overloaded_error`. Never request or response text, never the key.
- **`s-delegate`:** the rewrite view shows it through `showReserved("delegate", "api-request")`. `RESERVED_LANTERN_STATES.delegate`'s owner becomes `"api-request"`, and `SHOWS_RESERVED` in `tests/lantern.test.mjs` gains that one file.

### 13.8 Build steps (one commit each), and what is not in 1c

1. `net/mod.rs`: the Anthropic host (https/443), the redirect rule, the TLS source test.
2. Credential storage (`ai/key.rs`): the trait, Credential Manager, the fake.
3. The API client (`ai/claude.rs`): request building, answer and error parsing, cost, and the pre-send scan, tested against a fake local server.
4. Spend tracking and the cap (`ai/spend.rs`).
5. The rewrite flow:
   - the capture thread keeps the current paragraph;
   - `Request::Rewrite{tone}` and `RewriteApply{id}`;
   - the island's Rewrite card and `s-delegate`;
   - the hotkey registered (`rewrite` becomes ported).
6. Settings → AI: brain, key (masked, with Remove), model, cap, this month's spend.

**Not in 1c (later, or on the owner's word):**
- rewriting a **selection**: paragraph only for now, since reading the selection needs UIA `GetSelection`, a separate change;
- the **local brain**;
- a default tone, or one-key send;
- streaming;
- refusal fallbacks;
- the measured ≤4.5 s target, which needs the owner's key.
