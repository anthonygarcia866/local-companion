# Text-capture spike — results (Phase 0b)

Spike: `windows/src-tauri/src/capture/` (UI Automation, read-only, dev only with `GLIM_DEV=1`). For the focused field in another app it reads the process name, control type, which pattern worked, readable yes/no, field length and caret position. Password fields (`IsPassword`) are skipped before any pattern is asked for, and so is anything that isn't editable (see "Writing layer: editable fields only").

**Privacy:** captured text is never written anywhere. The results log (`%LOCALAPPDATA%\Glim\capture-results.jsonl`) holds only the columns below, written from a type with no field for text or window titles; tests enforce it. The debug panel shows only a window around the caret, live, and keeps nothing.

## Results

Tested 2026-10-09/10 on the owner's machine (Windows 11, Ryzen AI 7 350, 16 GB). "Caret" means the pattern reported where the caret is. Round 1 was read correctly by the backend while the panel couldn't show it yet (a missing Tauri capability, since fixed); its numbers come from the results log.

| App | Control type | Pattern | Readable | Chars seen | Caret | Source |
|---|---|---|---|---|---|---|
| Word (`WINWORD.EXE`) | Document | TextPattern2 | yes | 54–70 | yes | owner, round 1 |
| Word (`WINWORD.EXE`) | ListItem | none | no | 0 | — | owner, round 1 (a ribbon/list item, not the page) |
| Notepad (`Notepad.exe`) | Document | TextPattern2 | yes | 1,650 | yes | self-test: launched via `Start-Process`, focus moved with UIA `SetFocus` (no input) |
| Outlook classic — new email body (`OUTLOOK.EXE`) | Document | TextPattern2 | yes | — | yes, correct | owner, round 3, via poll. The field length included the email signature |
| Outlook (`OUTLOOK.EXE`) | DataItem | ValuePattern | yes | 12 | no | owner, round 1 (the message list, not a compose body) |
| Outlook (`OUTLOOK.EXE`) | Window / Pane | none | no | 0 | — | owner, round 1 |
| Grok (`Grok Bot.exe`) | Edit | TextPattern | yes | 1–59 | yes | owner, round 2: "Grok Bot.exe — Edit — TextPattern — readable — via poll" |
| Claude desktop (`claude.exe`) | Edit | TextPattern | yes | 1–2,121 | yes | owner, both rounds |
| Claude desktop (`claude.exe`) | Button | none | no | 0 | — | owner |
| Windows Terminal (`WindowsTerminal.exe`) | Text | TextPattern | yes | 5,301 | yes | owner, both rounds |
| A WebView2 app (`msedgewebview2.exe`) | Document | TextPattern | yes | 28–3,800 | no | owner, round 1 |
| Explorer / Snipping Tool | Pane, Window, Button, Group | none | no | 0 | — | owner (not text fields) |
| AppFolio login page in Chrome — password field (`chrome.exe`) | — | skipped (password) | not read | 0 | — | owner, round 3: correctly skipped, nothing read |
| AppFolio dashboard in Chrome, no text box focused (`chrome.exe`) | Document | TextPattern | **yes — bug** | 6,316 | — | owner, round 3: the whole page was captured. Fixed: now `skipped (read-only content)` (see below) |
| Self-test window — read-only text box (`powershell.exe`, WinForms) | Edit | skipped (read-only content) | not read | 0 | — | self-test after the fix (form focused it on launch, no input) |
| Self-test window — read-only rich-text document | Document | skipped (read-only content) | not read | 0 | — | self-test after the fix |
| Self-test window — editable text box | Edit | ValuePattern | yes | 39 | — | self-test after the fix |
| **Not reached yet** | | | | | | |
| Chrome — a page with no text box focused, after the fix | | | | | | owner to test (should skip) |
| Chrome — Gmail compose | | | | | | owner to test |
| AppFolio in Chrome — notes field | | | | | | owner to test |
| VS Code — editor | | | | | | owner to test |
| Word / Outlook compose, after the fix | | | | | | owner to re-test (should stay readable) |

## Writing layer: editable fields only

Chrome exposes a whole web page as a focusable **read-only** Document, so with no text box focused the spike captured an AppFolio dashboard's entire page content (6,316 characters). The writing layer must only ever read fields the user can type in. Before any text is read, `capture::is_editable` decides from signals that carry no text:

1. **ValuePattern.IsReadOnly**, when the element has a ValuePattern.
2. Otherwise the **`IsReadOnly` text attribute** of the element's whole text range: an attribute query, no characters are read. This is how a contenteditable box such as Gmail compose shows as editable inside a read-only page.
3. Otherwise the legacy (MSAA/IA2-style) **STATE_SYSTEM_READONLY** state skips it.
4. Otherwise only an **Edit** control counts as editable; anything unclear is skipped.

Read-only fields are skipped like password fields: nothing is read, and the panel shows "skipped (read-only content)". Unit tests cover the decision table (page content, contenteditable, read-only boxes, unknown documents), and a source-order test requires the password and editability checks to come before the only function that reads text.

**Delivery:** UIA focus-changed events arrive (3–44 per 30 s while the owner switched apps); the 300 ms poll fills in between them and is what follows typing inside a field (`via poll` in the Grok note). Both paths are reported per reading.

## Model latency vs. context (gemma3:4b, this machine, CPU)

Ollama 0.40.2, `gemma3:4b` (3.4 GB), a grammar fix of one sentence with N characters of surrounding context (synthetic text), reply ~20 tokens. Model loaded, median of 3, each run with a unique prompt so nothing is served from cache.

| Context sent | Prompt tokens | Total time | Of which reading the prompt |
|---|---|---|---|
| ~200 chars | 107 | **2.3 s** | 0.9 s |
| ~1,000 chars | 262 | **3.5 s** | 2.1 s |
| ~5,000 chars | 1,036 | **10.4 s** | 8.6 s |

- Reading the prompt runs at ~124 tokens/s; writing the answer at ~12–15 tokens/s. So every extra 1,000 characters of context costs ~2 s, and a longer answer costs ~1 s per 14 tokens (a 100-word rewrite, ~130 tokens, adds ~9 s on top of its context — an estimate from the measured rate, not a run).
- The first request after the model is unloaded takes ~12.6 s (10 s of it loading). Ollama unloads after 5 idle minutes by default; Phase 1 should keep the model loaded while the assistant is on (`keep_alive`), or warm it up when a supported field gets focus.
- Quality note: the grammar fix was correct in every run but worded differently ("I and the vendor…" vs "The vendor and I…").
- Not tried yet: the Radeon 860M iGPU (Ollama's Vulkan backend), which may cut these times.

## Phase 1 plan — how capture should work

1. **Read on demand, not continuously.** Capture the focused field when the user pauses typing (debounced, ~700 ms) or asks for a suggestion — not every 300 ms. Keep focus events for "which field is active"; read the text only when it will be used.
2. **Patterns:** TextPattern2 first (text + caret: Word, Notepad), then TextPattern (Electron/Chromium edits: Claude, Grok, WebView2 apps), then ValuePattern (simple fields, no caret). These covered every text field the spike reached. Where no pattern answers, the field is unsupported and Glim stays quiet.
3. **Tiered context** — the full field is read (safety cap 20,000 characters) and held in memory only; what goes to the model depends on the task:
   - **Grammar / spelling fix:** the current paragraph only (the caret window: ≤500 characters before the caret, ≤200 after). ~2–3.5 s on this machine.
   - **Rewrite / tone:** the paragraph plus surrounding context that changes the meaning — an email's subject line and the quoted message it replies to — capped around 1,000–1,500 characters (~3.5–4.5 s). 5,000 characters (~10 s) is too slow for an inline suggestion on this CPU.
4. **Never persisted.** Captured text lives in memory for the duration of one suggestion and is dropped. No logs, no history, no cache. Long-term memory of the user's writing is Phase 3 and opt-in only.
5. **Only what the user is writing.** Grammar and rewrite must exclude the email signature block and quoted reply text. In Outlook the captured field included the signature, so Phase 1 needs to cut the field down to the user's own new text (signature delimiters, Outlook's signature/reply separators, `>`-quoted lines and "On … wrote:" headers) before choosing the context tier. Quoted text may still be *context* for a rewrite, never something Glim corrects.
6. **Windows' own text suggestions.** Windows shows its own suggestion popup while typing (Settings → Time & language → Typing → "Show text suggestions when typing on the physical keyboard"). It competes with Glim's suggestion UI for the same moment and screen space. Phase 1 should detect when it's on and recommend turning it off during onboarding, and position Glim's suggestion so the two don't overlap if the user keeps both.
7. **Skip by default:** password fields (never read), terminals (Windows Terminal exposed 5,301 characters of scrollback — commands and output can hold secrets) and Glim's own windows. Everything else follows the per-app on/off toggle from the Phase 1 vision.
8. **Open before Phase 1 ships:** test the rows still marked "owner to test" above; decide how a suggestion is applied (TextPattern is read-only; ValuePattern.SetValue works only for simple fields; anything else needs a user-confirmed paste — no synthetic typing without the user's say-so).
