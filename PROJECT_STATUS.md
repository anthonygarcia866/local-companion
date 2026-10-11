# Project status — Glim

Repo: `anthonygarcia866/local-companion`. Dev machine location: `C:\Users\antho\local-companion`.

## Name
The app and its mascot are both named **Glim** (decided 2026-10-08). Product name `Glim`, bundle identifier `com.anthonygarcia.glim`, executable `glim.exe`, installers `Glim-Windows-<version>-setup.exe` / `.msi`. The brand artwork is in `docs/brand/` (see below).

## Vision
A local-first Windows desktop companion.

### Phase 1 — writing assistant
- UI Automation text capture of the focused field (password fields are skipped).
- Suggestions from a local model via Ollama.
- Island/pill UI that never steals focus.
- Settings for tone, aggressiveness, and a per-app on/off toggle.

### Phase 1.5 — Voice + Actions (priority right after 1b; owner, 2026-10-10)
- **Push-to-talk hotkey first:** the microphone is open only while the key is held.
- **Later, optional:** a local wake word ("Hey Glim"), **off by default**, detected on-device only.
- **whisper.cpp** speech-to-text; the model is picked by measured speed on the user's CPU (as gemma3:4b was).
- Then **gemma3 turns the request into a structured action, or into dictated text** (filler words, punctuation, tone matched to the active app, using the writing layer's capture context).
- **First actions:**
  - **Dictate** into the focused field, through 1b's insertion path.
  - **Open apps:** Claude Code, Codex, Outlook (no confirmation).
  - **"Pick up from yesterday":** read HANDOFF.md across the owner's repos, summarize where each one stopped, then open Claude Code in the chosen repo, told to read HANDOFF.md and continue.
  - **Draft emails** in classic Outlook via COM. The draft is shown to the user before sending, and **sending always needs the user's explicit OK**.
- **Rule:** consequential actions (send, delete, purchase, publish) wait for approval. Low-risk ones (open, read, summarize) don't.
- Audio is never stored.
- Its own mascot state for "listening to dictation" — not `s-record`: red stays reserved for screen recording.

### Phase 2 — SOP recorder
- User-initiated only.
- Windows Graphics Capture + UIA events.
- The user states the goal at record time.
- A local LLM writes a real SOP from the recording.
- Output is editable Markdown with screenshots, PDF export, and redaction.
- **Two capture layers with different rules:**
  - **Writing layer** (Phase 1): editable fields only, never page content (`capture::is_editable`; see `docs/capture-results.md`).
  - **Observation layer:** sees screens, page content and actions, in four modes:
    1. Explicit recording with a goal stated at the start.
    2. "Shadow mode": a toggle that observes everything until the user stops it, with a clear, visible mascot indicator the whole time.
    3. A rolling buffer that keeps only the last ~15–30 minutes in memory, continuously discarded, so the user can say "make an SOP from what I just did".
    4. Per-app exclusions (banking, password managers) that are never observed in any mode.
  - **To revisit at Phase 2 design:** shadow mode and the rolling buffer — CPU cost, the indicator, and how long the buffer keeps.

### Phase 3 — delegation layer (opt-in)
- A local user model: Markdown, user-stated facts only, user-editable, bootstrappable from the user's notes.
- The user model is injected into tasks delegated to Claude Code (headless) and Codex CLI.
- Loop: formulate the task, run it, read the result, follow up, report back.
- "It processes, I decide": sends, purchases, publishes and deletes wait for explicit approval. This is enforced through each CLI's tool allow/deny permissions, not prompt wording.
- Each hop is verified against the artifact (diff/build/test), never the agent's own report.
- **Memory policy:** memory stores learned summaries, never raw captured text.
  - **Keep:** big-picture goals and projects, how the user works, team members and contacts, what is being discussed, and how topics connect. Sources can include everything Glim observes, including the user's email inbox.
  - **Never store**, enforced by a hard filter before any memory write, even inside summaries: SSNs; bank, card and account numbers; dates of birth; phone numbers; personal street addresses; financial figures about individuals; passwords and credentials.
  - **Tenants** are referred to by role and property/unit ("a tenant at 1408 Jefferson"), never by name. Coworkers and business contacts may be named.
  - Memory is plain, user-editable files the user can view, edit and delete.
  - The filter must be testable: a fixture set of fake sensitive data, and a test asserting none of it reaches the memory files.
- **Outlook inbox learning:** Glim reads the user's classic Outlook mailbox locally through the Outlook COM object model (the local OST cache: no network, so the localhost-only rule holds; no Microsoft Graph or cloud API).
  - Purpose: learn the user's workflow — who they work with, recurring topics and processes, how threads connect to their projects.
  - All of it goes through the memory policy above: summaries only, the sensitive-data filter, tenants by role, not name.
  - The initial import is user-triggered, with progress shown; after that, optional incremental updates.
  - Outlook may show a programmatic-access security prompt; handle it at design time.
  - New Outlook has no COM API: out of scope unless the user switches.

## Privacy model
- **Phases 0–2: strictly local, no exceptions.** All outbound network access goes through a single choke-point module in Rust, which allows localhost only.
- **Phase 3: opt-in.** Data leaves the machine only on per-task approval, after the user sees the exact outgoing payload. Exceptions are added only through the choke-point module.
- Phase 1 text and Phase 2 recordings never flow to Phase 3 automatically. Only user-model entries marked shareable may.
- Enforcement is verified empirically (attempt a blocked request, confirm it fails), not trusted from config.
- What is enforced and where: [PRIVACY.md](PRIVACY.md).

### Network choke point
- **Location:** `windows/src-tauri/src/net/mod.rs`. `net::request(method, url, timeout)` is the only way to build an outbound request; it checks `net::check` first.
- **Allowlist:** `net::ALLOWED_HOSTS = ["127.0.0.1", "localhost"]`, any port, http/https, no credentials in the URL. Redirects are re-checked hop by hop, proxies are never used, and `localhost` is resolved to 127.0.0.1 by the client itself.
- **Guard test:** `net::tests::no_other_module_can_reach_the_network` fails if any Rust file outside `src/net/` names `reqwest` or a socket type. Mutation-checked: adding a `reqwest::get` line to `tray.rs` makes it fail.
- **Webview side:** CSP `connect-src ipc: http://ipc.localhost http://127.0.0.1:* http://localhost:*`, and WebView2 launched with background networking off and a resolver rule that fails every host name except `localhost` (`BROWSER_ARGS` in `lib.rs`, `additionalBrowserArgs` in `tauri.conf.json`; a test keeps them identical).

## Roadmap
- **Phase 0a** — rebrand to Glim, strip upstream assets, network choke point, local-only lockdown. *Merged 2026-10-09 (PR #2).*
- **Phase 0b** — the real Glim mascot (from `docs/brand/glim-mascot.html`) replaces the placeholder orb. In the notch: done (PR #5, see "Mascot"); the remaining placeholders moved to Phase 4. Also a text-capture spike (UI Automation) to decide how Phase 1 reads the focused field: see "Text-capture spike".
- **Phase 1** — writing assistant. Plan: `docs/phase1-plan.md`; capture plan and spike results: `docs/capture-results.md`. **1a** (instant checker, read-only list) on branch `phase-1a-checker`, PR open: see "Writing checker (Phase 1a)".
- **Phase 1.5** — Voice + Actions (see Vision): push-to-talk dictation and spoken actions. Priority right after 1b.
- **Phase 2** — SOP recorder, with two capture layers under different rules (see Vision). The recording start/stop sound cue ships with the recorder, not with Phase 4's sounds.
- **Phase 3** — delegation layer, with a memory policy: learned summaries only, never raw captured text, behind a testable sensitive-data filter (see Vision).
- **Phase 4** — Polish: sounds (synthesized in code via Web Audio, no audio files; suggestion sounds default off), more mascot animations/emotes, and replacing the remaining Coucou placeholders outside the notch (greeting, file-drop, recap image, per-session characters).
- **Priority order** (owner, 2026-10-10): 1a → 1b → Voice + Actions (1.5) → Phase 3 memory (including learning from the Outlook inbox) → 1c / 1d → Phase 3 delegation → Phase 2 SOP recorder → Phase 4 polish.
- **Code signing before distribution** (e.g. Azure Trusted Signing). Users with Smart App Control on can't run unsigned builds, and a privacy product that asks users to disable a security feature is a non-starter.
- **Rest of the Coucou names** (the relay, pipe and data folders were renamed 2026-10-09, see "Claude Code integration"): the other agents' plugin/config names (`~/.copilot/hooks/coucou.json`, OpenCode/Amp `coucou.js`/`coucou.ts`, the Hermes `coucou` plugin, Antigravity's `coucou` group, "generated by Coucou"), the `coucou_agent` / `coucou_diff_truncated` payload fields, and the Linux data/socket names (`~/.local/share/coucou`, `coucou.sock`; Linux is unbuilt). Each needs its own old-name handling like the relay's.
- **Upstream docs** in `docs/upstream/` (`AGENTS.md`, `INTEGRATIONS.md`, `SPEC.md`, `UPSTREAM_CLAUDE.md`) describe the Mac app and are reference only. Write Glim's own docs when the agent integration is reworked.

## Decided 2026-10-09
- Bundle identifier is `com.anthonygarcia.glim` (was `com.glim.app`, which made the bundler warn about the macOS `.app` extension).
- `docs/brand/` artwork is all rights reserved; the code stays MIT (see `NOTICE.md`).

## Open issues
- **Dev switches stop working / island unreachable (seen 2026-10-09, not yet fixed).** Two separate observations, same session:
  1. After a `--dev-chat` reply finished, the island folded to its hidden 300×8 strip at the top of the primary display, and further forwarded switches (`--dev-chat`, `--mascot-state think`, both known to work) had no visible effect: the island didn't reveal and no request reached Ollama. Glim stayed responsive. A restart fixed it. Cause unknown; the single-instance forwarding or the island's reveal path are the suspects.
  2. Later the owner couldn't open the island at all. Glim was responsive, but its island window was at x −1272, y −58: top centre of DISPLAY1 (the 1536×960 screen left of the main one), although `screen` is `"primary"` and the primary display is DISPLAY3 at (0,0). At the 21:28 launch it had been on the primary display, so something moved it later; the `screen-changed` → `Bridge.reposition()` path picking the wrong monitor is the main suspect. Restarting put it back on the primary display.
  - Workarounds: the tray icon's **Settings…** opens Settings without the island; in a dev session `glim.exe --open-settings` (GLIM_DEV=1) does the same from the command line.
  - To do: reproduce (watch window position across display changes and after a chat), log the chosen monitor on every reposition, and make reveal-after-hidden robust.

- **Capture reads payment details: fixed on branch `capture-sensitive-fields` (2026-10-10, PR open, not merged); some gaps remain.** See "Payment and ID data" under the text-capture spike. Still open:
  - **Long numbers (owner's decision, 2026-10-10, refined the same day):** a card typed into an unlabelled field passes through 12–15 digit states that fail Luhn, so a reading is dropped when it holds 12+ digits that may be a card being typed. That means an unbroken run of 12+ digits, or digits split by spaces/dashes and grouped like a card (4-4-4, 4-4-4-4…, Amex 4-6-x). International phone numbers are exempt (a leading "+", at most 15 digits).
    - **Still readable:** unit lists ("Units 101 102 103 104"), order numbers (3-7-7) and `00`-prefixed phone numbers.
    - **Still dropped:** unbroken tracking and account numbers, and 4-digit groups that look like a card ("Units 1001 1002 1003").
    - Up to 11 typed digits of a card can still reach the panel.
  - **Browsers don't expose `autocomplete` (cc-*) or `name=`** through UI Automation (checked in Chrome and Edge). IAccessible2 attributes were not checked.
  - **Labels are matched in English only.**
  - **No one-key pause yet**, and the pause list is built in: both belong to Phase 1d's settings.
  - A dropped reading shows as "skipped (sensitive)", the same as a field skipped by its labels (kept as is, owner 2026-10-10). Neither the panel nor the results log can tell which, but they do show that a field was skipped.

## Open questions
- At Phase 3 build time, verify whether Claude Code and Codex CLI can run on a subscription login rather than API keys, and check their current headless flags and permission syntax.

## Current status
Mascot merged 2026-10-10 (PR #5, `c21debf`). Text-capture spike on branch `phase-0b-capture` (PR open, not merged): see "Text-capture spike".
Phase 0a merged 2026-10-09 (PR #2, `8f07e66`). Integration rename (relay `glim-hook.exe`, pipe `\\.\pipe\glim-<SID>`, `Glim` data folders with migration) on branch `feat/rename-integration-ids` (PR open, not merged). Non-Windows trees removed 2026-10-09 (PR #3, `82b3fb3`): `NotchBuddy/` (macOS + iPhone app), `relay/` (iPhone relay Worker), `linux/` (Arch recipe for upstream Coucou), the macOS-only `build.yml`/`release.yml` workflows, `scripts/` and `tests/*.swift` (Swift tests and tools that compiled `NotchBuddy/` sources), and `docs/IPHONE.md`; also `linux.yml` (Glim is Windows-only, decided 2026-10-09). Upstream's Mac-app docs moved to `docs/upstream/` with a reference-only note. The string catalog moved to `windows/src/i18n/Localizable.xcstrings`; `scripts/gen-strings.mjs` and its test read it there.

- **Upstream:** [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou), forked at `5cb2a27` (2026-10-08). Remotes: `origin` = anthonygarcia866/local-companion, `upstream` = Louis-CFM/coucou. Credited in `NOTICE.md`.
- **Toolchain** (2026-10-08): git 2.54.0, gh 2.94.0, Node 24.16.0, npm 11.13.0, rustup 1.29.1, rustc/cargo 1.99.0, VS Build Tools 2022 17.14.41 (VC tools), WebView2 runtime 154.0.4258.62.
- **Phase 0a build** (`cd windows`, `npm run pack`, 2026-10-09 00:49): exit 0, unsigned.
  - `windows\release\Glim-Windows-0.2.0-setup.exe` — 2,296,552 bytes, sha256 `BCFAF19F2A8A6BCB…`
  - `windows\release\Glim-Windows-0.2.0.msi` — 3,203,072 bytes, sha256 `995265911477C6A6…`
  - `windows\target\release\glim.exe` — 5,687,808 bytes, sha256 `1CDA2F49DD6B2A55…`
  - Down from 4.95 MB / 5.77 MB at baseline: the WAVs and the cloud/integration code are gone.
- **Tests:** `cargo test --workspace` 157 + 30 passed; `npm test` 285 passed; `tsc --noEmit` clean.
- **Launch** (2026-10-09, `glim.exe` run directly): window titles "Glim" (island, desktop-character window) and "Settings — Glim"; the island shows a plain grey orb (placeholder) and no service pills; Settings header reads "Glim 0.2.0" with the new icon. Cropped captures in `docs/verification/phase-0a/`.
- **Network, observed:** see "Phase 0a verification" below.

### Phase 0a verification (2026-10-09)
- **netstat on the app's whole process tree** (`glim.exe` + its 8 `msedgewebview2.exe` children), polled every 2 s:
  - *Before the WebView2 lockdown* (same code otherwise), 5 min: `glim.exe` itself opened nothing external, but the WebView2 network process did — QUIC to `3.101.126.163:443` (AWS us-west-1) and `[2603:1063:1e:143::365:7ea3]:443`, TCP to `[2620:1ec:33:1::11]:443` (×2). A WebView2 net-log of a 75 s run named them: `config.edge.skype.com/config/v1/Edge/154.0.4258.62` (Edge experiment config), `edge.microsoft.com/componentupdater/api/v1/update`, `ecs.nel.measure.office.net/api/report`, plus a `wpad` lookup. The pages themselves only ever loaded `tauri.localhost` / `ipc.localhost`.
  - *After* (the shipped build), 4 min 20 s, 51 polls, with normal use (island opened, chat to a local server, Settings opened and scrolled, a blocked connect attempted): **0 endpoints, local or remote**. A 75 s net-log with the same WebView2 switches contained no host name other than `tauri.localhost` / `ipc.localhost`. Polling every 2 s can miss a connection that opens and closes in between; the before-run shows it does catch the runtime's traffic.
- **Blocked request in the running app:** Settings → Local models → LM Studio → `https://example.com:1234` → Connect: "Blocked: Glim only connects to this computer (127.0.0.1 or localhost), not example.com:1234." (`docs/verification/phase-0a/settings-blocked-external.png`). Also covered by `net::tests::a_request_to_an_external_host_is_refused_before_anything_is_sent`.
- **Ollama:** not installed on this machine (nothing on `:11434` or `:1234`). The local chat path was exercised instead against `tests/fake_local_llm.py` (an OpenAI-compatible stand-in) on `127.0.0.1:60771`: the island streamed its answer (`island-chat-local.png`). Real Ollama is still untested.

## Mascot — Glim the lantern
Done in the island's notch on branch `mascot-lantern` (2026-10-09). Source of truth: `docs/brand/glim-mascot.html` (approved); ported, not redrawn.

- **Where it lives:**
  - `windows/scripts/port-lantern.mjs` generates `windows/src/mascot/lantern.css` and `windows/src/mascot/lantern-svg.ts` from the design file. Never edit those two by hand; `tests/lantern.test.mjs` fails if they drift from the design.
  - `windows/src/mascot/lantern.ts` is the component (`new Lantern({ detail, state })`): state = one class `s-<state>` on the `<svg>`; `compact` below 48 px of rendered height; detail `"notch"` drops the `aura` and `twinkles` groups, `"full"` keeps them (large contexts: settings, onboarding, about); gradient/filter/clipPath ids get a per-instance suffix; the design's `prefers-reduced-motion` rule is kept.
  - What the port changes, and only this: page-level CSS dropped; selectors the design left unscoped are scoped to `.lantern` (same relative order and specificity inside the lantern); keyframes prefixed `lantern-` (the app already had a global `@keyframes pulse`).
  - Wired in `windows/src/island/island.ts` (`drawBot`): the lantern replaces the placeholder orb, drawn 1.2× the old orb's diameter tall with its base on the orb's bottom edge (24 px in the 26 px notch pill → compact; 53–79 px in the expanded cards → full detail minus aura/twinkles). The engine (`mochi/engine.ts`) still runs for its timers but no longer draws in the island.
- **Nine states** (design updated 2026-10-09): `s-idle`, `s-listen`, `s-think`, `s-suggest`, `s-record`, `s-paused`, `s-delegate`, `s-done` (happy eyes, a hop), `s-error` (dim, droopy, worried brows, a sweat drop — never red). Compact uses a wider listen glance (`glance-c`).
- **Reserved states — never reuse them for anything else** (design file header, CLAUDE.md): red `s-record` = screen recording only; `s-delegate` = Phase 3 data leaving the machine only. Neither feature exists yet, so no app state maps to them. Enforced three ways: `lanternStateFor` returns `AppLanternState` (reserved states excluded, so `tsc` rejects a mapping to them); the only way to show one is `Lantern.showReserved(state, owner)` with a typed owner (`"screen-recording"` / `"phase3-delegation"`, or `"dev-preview"` for the dev switch); `tests/lantern.test.mjs` fails if any island state (read from the `BotStateName` union) maps to one, if a reserved state name appears in `src/` outside the mascot module, or if anything but the allowed callers calls `showReserved`. When the recording or delegation feature lands, add its file to `SHOWS_RESERVED` in that test.
- **State mapping** (`lanternStateFor`, island state → lantern):

  | Island state | Lantern |
  |---|---|
  | `idle` (notch) | `s-idle` |
  | `idle` (island open) | `s-listen` |
  | `working`, `thinking`, `searching` | `s-think` |
  | `approval`, `question` (waiting for your answer) | `s-suggest` |
  | `finished` | `s-done` |
  | `error` | `s-error` |
  | `ratelimit` | `s-paused` — the agent hit its usage limit and can't work until it resets: "not working right now" |
  | `sleeping` | `s-idle` — only the desktop character sets it, after a stretch with no agent activity; nothing is blocked, Glim is resting, which is s-idle's dozing look |
  | `dizzy` (shake gag) | `s-think` |

  Not mapped: the emotes (love, surprised, proud, wink, yawn, happy, annoyed), the integration/plan body tint (`engine.bodyColor`), and the drop sequence's morph — the lantern ignores them.
- **Still the placeholder** (not the notch, not in this PR): the desktop character window (`desktop/`), the per-session mini bots (`mochi/minibots.ts`), the greeting (`mochi/greeting.ts`), the drop sequence (`upload/canvas.ts`) and the recap image (`recap/share.ts`).
- **Dev-only switches** (start Glim with `GLIM_DEV=1`; a second `glim.exe` forwards its arguments to the running instance; ignored without `GLIM_DEV=1`; no hotkeys): `--mascot-state <idle|listen|think|suggest|record|paused|delegate|done|error|auto>` shows that lantern state (`auto` follows the app again); `--dev-chat "<prompt>"` connects Ollama as Settings → Connect does (if needed) and sends the prompt through the island's chat view (pass it as one quoted argument — `Start-Process -ArgumentList` with an array splits it into words); `--open-settings` opens the Settings window.
- **Checks:** `npm run check:lantern` (`windows/dev/lantern-check.mjs`, headless Edge over the DevTools protocol, also in `windows-ci.yml`): 90 renders (9 states × full/compact × 5 frozen instants) compared with the design page pixel by pixel (≤ 2/255 rounding allowed, every exception printed; a visible change fails); `prefers-reduced-motion: reduce` emulated through the protocol stops every animation in every state; notch-size frames of the blinks and glances. Mutation-tested: a changed colour and a deleted reduced-motion rule both fail. A render that differs is re-rendered from fresh pages up to twice before it counts: on GPU-less CI runners the aura's SVG blur (`feGaussianBlur`) isn't bit-stable (seen 2026-10-10: 1,307 pixels up to 32/255 along the ribbon edges, same markup); a real change persists through the re-renders and still fails (checked).

### Mascot verification (2026-10-09)
- In the running app, each state forced with `--mascot-state`, cropped to Glim's pill: `docs/verification/mascot/notch-<state>@4x.png` (all nine). Cap and handle ring stay visible on the black pill (the ring, `#6b4530` with no outline, is the dimmest part but reads). `s-error`'s sweat drop shows in the app (`app-error-sweat-lantern@8x.png`).
- Compact listen glance (after the design's `glance-c`): measured in the running app from a burst of notch captures, the eyes' darkness-weighted centre moves −0.75 px then +0.76 px (~1.5 px swing) and blinks; `app-glance-{rest,left,right}-lantern@8x.png`. Before `glance-c` the swing was ~0.6 px and barely read.
- Unforced, with a real `claude -p` session hooked to Glim: `s-think` in the notch mid-session (`auto-working@4x.png`), then the expanded "Session finished" card with the lantern in `s-done` (`auto-finished-expanded.png`).
- Browser check: 90/90 renders byte-identical to the design; reduced motion stops every animation in all 18 state/size pairs; notch-size frames in `frame-*@8x.png`.

## Glim presence (branch `glim-presence`)
How Glim sits on screen. Nine items, one commit each (a few split in two); PR open, not merged.

1. **Pill size:** Settings → General → Pill size, small / medium / large (default large); `core/layout.ts` `pillGeometry`.
2. **Attention pop:** a short bounce on each lantern state change, except idle ↔ listen (the island opening and closing); `s-record` gets its own stronger, repeating cue (`pop-record`). Additive `pop` / `pop-record` / `ignite` classes in the design file, ported by `port-lantern.mjs`. Reduced motion: glow change only.
3. **Startup ignite:** the launch shows the compact pill with the lantern dark (`s-paused`), the flame lights over 800 ms, then it settles in `s-idle`. App states asked for meanwhile wait; recording never does. Replaces upstream's greeting. Every leftover placeholder character (minis, file drop, recap image, desktop window) is now the lantern.
4. **Palette:** upstream's purple/indigo accents replaced with Glim's amber (`#FFB347`).
5. **Monitor:** a once-a-second display watch, running even while the island is hidden, re-places the island when its target display changes or Windows moves the window (the wrong-monitor open issue). New display option "Display of the active window". Every display and placement is logged.
6. **Verify:** see "Presence verification" below.
7. **Visibility modes** (`src-tauri/src/presence.rs`, `src/island/presence.ts`): **Normal** (the pill), **Ember** (a 28 px glowing dot at the dock's edge; click it for the pill), **Hidden** (the window is hidden; back via the tray icon's Open or the hotkey). Global hotkey **Ctrl+Alt+Shift+Space** cycles Normal → Ember → Hidden (was Ctrl+Alt+H, which Word and OneNote use for highlight); it is handled in Rust, so it reaches a hidden window. Rebindable in Settings → Shortcuts; see "Hotkeys" below. Settings → General → "Show Glim as" picks the mode. A **fullscreen app in front** on the island's display (Windows' D3D-fullscreen or presentation-mode state, or a foreground window covering the whole display; a maximised window does not count) hides Glim in every mode, and it comes back when the app leaves.
   - **Recording rule:** while the screen is being recorded (Phase 2), something stays on screen in every mode, Hidden and fullscreen included: the recording indicator (the lantern in `s-record`, in the ember's place). `presence::presence()` is the rule (`recording_is_never_hidden` test); the page shows the indicator only through `showReserved(RECORDING_STATE, "screen-recording")` (`tests/presence.test.mjs`, `SHOWS_RESERVED` in `tests/lantern.test.mjs`). Nothing sets recording yet: `presence::set_recording` is for the Phase 2 recorder.
   - Rust owns `settings.visibility`, like `docks`: a webview's save never overwrites it.
8. **Docking** (`placement.rs`): top-center (default), top-left, top-right, left edge, right edge (upright). Drag the closed pill: the window follows the cursor without being activated; on release it snaps to the nearest dock of the display under the cursor and is remembered for that display (`settings.docks`). Upright docks: the pill stands along the edge and the island opens sideways, away from it. Settings → General → Position.
9. **Self-capture skip:** the capture layer skips Glim's whole process tree (`glim.exe` and its `msedgewebview2.exe` children) before any read. It fails closed: an unknown pid is rechecked at once, a failed process snapshot skips the reading, and only `glim.exe` / `msedgewebview2.exe` can join the tree.

- **Dev switches added** (GLIM_DEV=1, forwarded by a second `glim.exe`): `--dock <top-center|top-left|top-right|left-vertical|right-vertical>`, `--visibility <normal|ember|hidden>`, `--fullscreen <on|off|auto>` (pretends a fullscreen app is or isn't in front; `auto` = detect again).

### Presence re-verification (2026-10-10, last run 17:07 on `glim.exe` built 17:06, 5,796,352 bytes; first run 15:41 on the 15:38 build)
Reusable: `windows/scripts/verify-presence.ps1` (dev switches only, no mouse or keyboard; PrintWindow of Glim's island window only; backs up and restores `settings.json`; ends by starting Glim normally). Log and captures in `docs/verification/presence/` (`verify-log.txt`). **All checks passed.**
- **Pill sizes** (`size-small/medium/large.png`, 2×): lantern 25 / 36 / 49 px tall; small and medium in the compact drawing, large with the full face.
- **Ignite** (`ignite-*ms.png`): dark grey lantern at 0 ms, flame lit by ~260 ms after the window appears, glow, settled in `s-idle`.
- **Pop** (`pop-before/peak/after.png`, 3×, a 25 ms capture burst around `--mascot-state think`): lantern 49 px at rest, 54 px at the captured peak, 49 px after.
- **Docks**: top-left (0,0), top-right (1200,0), top-center (600,0) at 720×320; left / right upright at (0,340) / (1200,340), 720×400; lantern where each dock puts it.
- **Visibility**: Ember = 28×28 at (946,0), an amber dot (`vis-ember.png`, 8×); Hidden = window not visible; Normal brings the pill back; `--fullscreen on` hides it, `off` brings it back.
- **Palette**: the chat card's glow is amber (`palette-after-chat.png`); before: indigo (`docs/verification/phase-0a/island-chat-local.png`).
- **ba8d079 / focus**, read from the real window after every step: `WS_EX_NOACTIVATE` held throughout and the foreground was never Glim's (except the chat step, where the chat field takes focus on purpose); after the chat, Ember and Normal both had `WS_EX_NOACTIVATE` back (focus released). The ember was **not** click-through (it takes the mouse over its whole window). Back from Hidden with the cursor elsewhere and not moving, the panel **was** click-through (no swallowed clicks).
- **Hotkeys** (17:07 run): the startup log line read `shortcuts: registered openChat=Ctrl+Alt+Shift+C, goToAlert=Ctrl+Alt+A, jumpToTerminal=Ctrl+Alt+T, nextPill=Ctrl+Alt+Right, prevPill=Ctrl+Alt+Left, cycleVisibility=Ctrl+Alt+Shift+Space; not registered: none`.
- Hands-on by the owner (2026-10-10): sizes, dragging + snapping, ember, hidden + the Ctrl+Alt+Shift+Space cycle, tray restore, fullscreen auto-hide, right-click menu, upright dock left/right — all working.
- Not covered live: the hotkey and clicks (real input), the recording indicator (nothing records yet), real fullscreen apps (dev override only).

### Presence verification (2026-10-10, `glim.exe` built 13:13, 5,789,184 bytes)
Driven only by dev switches (no mouse or keyboard); PrintWindow of Glim's own island window (transparent parts and the black pill both come out black, so the lantern is what shows). Captures in `docs/verification/presence/`.
- **Docks** on a 1920×1080 display: top-left window at (0,0), top-right (1200,0), top-center (600,0), all 720×320; left/right upright at (0,340) / (1200,340), 720×400, the lantern at the top of the upright pill.
- **Ember:** a 28×28 window at the dock's edge: (946,0) for top-center, (1892,526) for the right edge; a soft amber dot (`vis-ember@8x.png`).
- **Hidden:** the window is hidden; Normal brings the pill back. **Fullscreen** (`--fullscreen on`): hidden in Normal and in Ember; `off` brings the ember back.
- **Focus:** after every step the foreground window belonged to another app (Chrome / Windows Terminal), never Glim; the foreground at the end was the one at the start.
- **Bug found and fixed in this run:** Hidden → Normal left the window hidden. Glim hid it through tao (`win.hide()`) but showed it with a raw `ShowWindow`, and tao re-applies its cached visibility on every later window-flag change (tao 0.37.1 `window_state.rs`, `apply_diff`), so the next click-through toggle hid it again. Now shown through tao too (`win.show()`; the island is WS_EX_NOACTIVATE, so it doesn't take focus — checked above).
- **Not verified live:** the recording indicator (nothing records yet; covered by the Rust rule test and `tests/presence.test.mjs`), the real fullscreen detection against a real fullscreen app (only the dev override was driven), the visibility hotkey and a click on the ember (both need real input).

### Hotkeys (checked 2026-10-10 on the owner's machine)
Glim's own hotkeys follow one pattern, **Ctrl+Alt+Shift + a key**: Space = show/hide, C = chat, R = rewrite. All three are in Settings → Shortcuts and rebindable; if any enabled Glim shortcut fails to register, Settings → Shortcuts shows a warning naming it, and the log has one line at startup listing what registered and what didn't (`shortcuts: registered …; not registered: …`). The three are chat, visibility, and the Phase 1 rewrite (listed as "Coming in Phase 1": shown and rebindable now, registered only once Phase 1 lands, so it takes no key from other apps before then).

| Hotkey | Use | Word (and Outlook's editor) | Held by another app | Result |
|---|---|---|---|---|
| Ctrl+Alt+Space | Open the chat (until 2026-10-10) | **Read Aloud** | **yes** | Never registered on this machine (log: "openChat not registered: HotKey already registered"). Replaced. |
| Ctrl+Alt+Shift+C | Open the chat (new default) | unassigned | free | Adopted. AltGr+C types ₢ on Brazilian ABNT2, but that is the unshifted layer; Shift+AltGr is checked at run time against the installed layouts. |
| Ctrl+Alt+Shift+Space | Visibility cycle | unassigned | free | Adopted. |
| Ctrl+Alt+G | Planned Phase 1 rewrite | **"next Editor suggestion"** | **yes** | Conflicts: not used. |
| Ctrl+Alt+Shift+R | Phase 1 rewrite (default instead of G) | unassigned | free | Adopted as the planned default (R for rewrite). |

How it was checked: Word's own key table (`Application.FindKey`, a hidden read-only Word instance, closed after); "held by another app" by registering and at once releasing each combination (`RegisterHotKey`), which fails when any running program already holds it. Windows itself assigns none of these. Chrome's and Outlook's main-window shortcut lists have no Ctrl+Alt+Shift+Space or Ctrl+Alt+Shift+R. VS Code is not installed on this machine, so its keybindings could not be read; neither combination is among its documented Windows defaults. Which running app holds Ctrl+Alt+Space and Ctrl+Alt+G is unknown (Windows doesn't say); candidates running at the time: Claude desktop, ChatGPT, Grok Bot, Aqua Voice, AMD Radeon Software, Logi Options+, Teams, Superhuman. Only en-US is installed, so AltGr isn't a factor here (Glim still checks installed layouts at run time).

## Writing checker (Phase 1a, branch `phase-1a-checker`)
- **What it does:** in the apps it is on for (`src-tauri/src/writing/apps.rs`: Outlook classic + new, Word, Chrome, Edge, Notepad, chat apps), the capture thread now runs in normal sessions too. About 1 s after typing stops, it checks the paragraph around the caret, without the signature or quoted reply, with Harper (`harper-core` 2.11.0, en-US, Apache-2.0, no network crates). The island gets the suggestions (`writing-suggestions` event); the text never leaves the capture thread otherwise and is never logged (tests enforce both sides).
- **Island:** a count on the pill at the lantern's top right (`role="status"`, read out as "N writing suggestions"), `s-suggest` unless an agent state is showing (agent wins), a pencil tab, and a read-only "Suggestions" view (problem in context, Harper's message, replacements). **Nearest the caret first, three a page, Previous / Next and "1–3 of 12"** (the view is 290 px; at 212 the third row and the pager were cut off, and the window can't scroll). Opening the island shows it after a waiting approval. Applying fixes is 1b.
- **Ember:** with suggestions waiting the ember brightens (no bounce) and shows the count ("9+" past nine). Clicking it **peeks**: the island opens on the list without changing the saved mode (`presence::set_peek`), and the ember comes back when the island closes. Without suggestions the click is unchanged (back to Normal). Hidden shows nothing. (Was an open issue after the first 1a build; fixed in PR #9.)
- **Timing:** a check runs 600 ms after the text stops changing (was 1 s). Nothing is sent while typing, so the count doesn't flicker.
- **Chat apps** (Slack, Teams, Discord, WhatsApp, Claude, ChatGPT, Grok) use relaxed rules: no capitalization lints, nothing on common chat words (lol, idk, tmrw, ok, i'm …); real misspellings are still found. Everything else keeps the full rules.
- **Performance** (release build, this machine): setup ~0.54 s once, on the first supported field, and it includes a warm-up check. Without the warm-up, the user's first check took 25 ms (61-70 ms live). Now: first check 3 ms, later ones 0.2-1 ms for 46-155 chars; live log `writing: 2 checks, median 5.13 ms`. `cargo test --release -p glim timing_probe -- --ignored --nocapture` prints the numbers.
- **Verification (2026-10-10, `glim.exe` built 20:58, 11,776,512 bytes, sha256 `27bc2404a5573969…`):** `scripts/verify-writing-notepad.ps1`, no mouse or keyboard: count 2 for "We recieved the payment and will deposit it tomorow.", gone for a clean sentence in a second tab, numbers-only log line, no test text in `glim.log`; PASS 3 times (`docs/verification/writing-notepad/`). Network: Glim's 9 processes (glim.exe + WebView2) had 0 TCP and 0 UDP endpoints. Tests (round 2): `npm test` 341, `cargo test --workspace` 244 + 30 (1 ignored), `tsc` clean; clippy is clean on every file the branch touches (8 older `-D warnings` lints remain in `agents.rs`, `config_file.rs`, `island.rs`, `net/mod.rs`, `recap.rs`).
- **Size:** `glim.exe` 5.8 MB → 11.8 MB (Harper's dictionary).
- **Owner's hands-on (2026-10-10, first build):** works in Outlook, Word and Gmail; counts and suggestions correct ("neeeed" → needed/need). Their feedback (the stuck list, the debounce, Ember, chat rules) is in the second round of PR #9.
- **Round 2 verification (2026-10-10, `glim.exe` built 21:41, 11,788,800 bytes):** `verify-writing-notepad.ps1` PASS, no mouse or keyboard:
  - count 2, then gone for a clean sentence;
  - in Ember, a twelve-mistake paragraph → label "Glim: 12 writing suggestions"; a UI Automation Invoke on the ember opened the list on "1–3 of 12" with the saved mode unchanged; Next → "4–6 of 12"; Minimize → back to the ember;
  - a text box compiled as `slack.exe`: one suggestion ("recieved"), with u / tmrw / lol / idk / i / btw / ok / ty let through;
  - checks: median 3.5-12.5 ms, max 17.7 ms for 154 characters.
- **Still owed:** the owner's retest: many suggestions, the ember, a real chat app.

## Text-capture spike (Phase 0b)
Branch `phase-0b-capture`. Results and the Phase 1 plan: `docs/capture-results.md`.

- **Code:** `windows/src-tauri/src/capture/` — a dedicated MTA thread owns every UI Automation object; it subscribes to focus-changed events (the handler only wakes the thread) and re-reads the focused field every 300 ms. Password fields are skipped before any pattern is asked for; then TextPattern2 (text + caret), TextPattern, ValuePattern. Read-only: nothing is written back, no keyboard hooks. Dev only (`GLIM_DEV=1`).
- **Privacy:** the full field (capped at 20,000 characters) stays inside the capture thread. Only the window around the caret (current paragraph, ≤500 chars before, ≤200 after) and the field length go to the dev-only debug panel (`capture.html`), live, never stored. The results log (`%LOCALAPPDATA%\Glim\capture-results.jsonl`) is written from `CaptureMeta` (app, control type, pattern, readable, password, char count, caret position) — no field for text or window titles. Tests: the log and `Debug` output never contain them; no log/write call in the module mentions text, excerpt or title (mutation-tested).
- **Owner's round 4 (2026-10-10), all pass:** Chrome page with no text box focused skipped (read-only content, 0 chars); Gmail compose, an AppFolio notes field, VS Code, Word and Outlook compose readable.
- **Editable fields only:** before any text is read, `capture::is_editable` decides from ValuePattern.IsReadOnly, the text range's IsReadOnly attribute, the legacy read-only state and the control type; read-only content (Chrome exposes whole pages as read-only Documents — an AppFolio dashboard was captured in full before this) is skipped like a password field, panel "skipped (read-only content)". Self-tested with a WinForms window: read-only box and read-only document skipped, editable box read.
- **Payment and ID data** (`capture/sensitive.rs`, branch `capture-sensitive-fields`), all inside the capture thread:
  1. Before anything about the field is read: apps on the pause list (password managers, by executable) are skipped.
  2. After the password check, and before any text: fields whose UIA Name, LabeledBy name, HelpText, FullDescription or AutomationId read like card number, CVC/CVV, expiry, cardholder, bank account/routing number, IBAN or SSN are skipped (word-boundary matching: "Cc", "Scorecard notes", "Card notes" stay readable). So are browser fields whose page or any enclosing frame URL (each Document's UIA Value) is on a paused site: checkout/bank/payment hosts, plus any `/checkout`, `/checkouts`, `/payment(s)` path.
  3. After the text is read, before anything else sees it (`finish_reading`), the whole reading is dropped if it holds a Luhn-valid 13–19 digit number grouped like a card (unbroken, 4-4-4-4…, 4-6-5, 4-6-4; spaces or dashes), a 3-2-4 SSN-shaped number, a mod-97-valid IBAN, or 12+ digits that may be a card being typed (unbroken, or grouped 4-4-4… / 4-6-x) unless written as an international phone number ("+" and ≤15 digits, exempt from all number checks).
  - Skipped and dropped readings send no text, length, caret or window title. The panel shows "skipped (sensitive)" or "skipped (paused app or site)".
  - **Tests:** published test cards, never-issued SSNs and example IBANs never appear in the panel event's JSON, its Debug output or the results log line, in any formatting (not even a 4-digit piece). Phone numbers (including "+44 20 7946 0958", "+4915112345678"), addresses, invoice totals and dates get through. Every state of a card being typed (with spaces, dashes or none) is dropped from its 12th digit on; unit lists and order numbers stay readable. A source-order test requires the pause, label and site checks before `read_text`.
  - **Live** (`windows/scripts/verify-capture-sensitive.ps1`, 2026-10-10 19:39, release build 19:38): a dev Glim, fixtures on 127.0.0.1 in a throwaway Edge profile, and a compiled stand-in `KeePass.exe`. Focus moved only with UIA SetFocus, and only after 4 s with no user input. All 12 checks pass: ordinary notes (90 chars) and a unit list "Units 101 102 103 104" are read; a test card, a card being typed (12 digits), an SSN, an IBAN, a field labelled "Card number" and a field with id `cc-exp` are skipped as sensitive; a `/checkout/` frame, a `/checkout/` page and KeePass.exe are skipped as paused. None of the fake numbers appeared in the panel's text, the new results-log lines or glim.log (`docs/verification/capture-sensitive/`).
- **Focus:** the island and every window Glim builds never take focus by appearing (test + runtime check). The hidden Settings window used to (fixed, see lessons).
- **Ollama:** 0.40.2 installed via winget, listening on 127.0.0.1:11434 only; model `gemma3:4b` (3.4 GB; non-thinking, recent; `llama3.2:3b` 2.0 GB is the faster fallback). Glim's chat streams a real reply through `net::request`; netstat on Glim's whole process tree during the chat: one connection, `glim.exe` → 127.0.0.1:11434, nothing else. Latency vs. context: ~200 chars 2.3 s, ~1,000 chars 3.5 s, ~5,000 chars 10.4 s (details in the results doc).
- **Dev switches added:** `--dev-chat "<prompt>"`, `--open-settings` (see Mascot → dev switches).

## Removed in Phase 0a
- **Network paths:** Anthropic Messages API (`api.anthropic.com`, with web search), OpenAI / Google AI / OpenRouter (`openai_compat.rs`), the "any OpenAI-compatible server" option that could point anywhere; GitHub, Stripe, Vercel, Notion, Resend, Cal.com and n8n pollers (`integrations.rs`, `github.rs`); the Codex plan pill, which spawned `codex app-server` (it asks OpenAI for the limits); the domain-directory display-name lookup (`GetUserNameExW`); "open in browser" for arbitrary URLs (now loopback only). No updater existed upstream; none was added.
- **Secrets:** `secrets.rs` and the `keyring` crate (Windows Credential Manager / Secret Service). Glim stores no keys.
- **Upstream assets:** all 29 WAVs, the Xcode asset catalogs (Coucou icons), `docs/media/`, `design/`, `windows/screenshots/`, upstream's website pages; the Mochi drawing code in the Windows app (engine face/hands/outfits, the greeting's Mochi, the drop animation's mailbox Mochi, the recap image's Mochi), the wardrobe (outfits), sounds and their settings, the mute and wardrobe shortcuts (now reserved, never registered).

## Claude Code integration
Renamed from upstream's `coucou` names on 2026-10-09 (relay, pipe and data folders together, with a migration). The user-visible strings followed: "Denied from Glim" (relay deny message), "Waiting for your answer in the island (Glim)" (Codex status), "Glim weekly recap" (recap image file name).

- **What Glim writes to `~/.claude/settings.json`** — only after an explicit click in Settings, after showing the exact diff, taking a dated backup, and checking the file hasn't changed since the preview (`hooks.rs` → `config_file.rs`):
  - `hooks.<Event>[]` gets one entry per event: `{"hooks":[{"type":"command","command":"\"C:/Users/<you>/AppData/Local/Glim/bin/glim-hook.exe\" <Event>","timeout":N}]}` for `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop`, `StopFailure`, `SubagentStart`, `SubagentStop` (timeout 10 s) and `PermissionRequest` (120 s).
  - **Which entries are Glim's:** commands naming `glim-hook` (`agents::MARKER`) or the old `coucou-hook` (`agents::LEGACY_MARKER`). Installing removes both and adds one fresh entry per event, so re-running it never duplicates and replaces old entries; uninstalling removes both and nothing else. Only `glim-hook` entries count as **installed**: an old entry's relay path went with the old folder, so Settings offers "Install hooks…" to replace it. The same rule applies to the status line and to the other agents' configs (`agents.rs`), and to the Claude Code pill's connected dot (`agent_hooks.rs`, which still counts the Mac app's `~/.claude/coucou/nb-hook`).
  - Optionally (separate switch, "Plan usage"): `statusLine.command` becomes `"…/glim-hook.exe" --statusline`. A status line the user already had is saved to `%LOCALAPPDATA%\Glim\bin\statusline-previous.json` and the relay keeps running it.
  - On this machine the hooks were installed by the owner on 2026-10-09 (Settings → Install hooks…): the diff against the backup `~/.claude/settings.json.bak-20261009-213348` is exactly Glim's 12 entries added (one per event); the owner's own `PreToolUse` hook is untouched and still first, nothing else changed, and Glim's own dated backup is byte-identical. With Glim closed, `glim-hook.exe` exits 0 with no output in ~12 ms median (49 ms max, 10 runs × 12 events); through Git Bash, as Claude Code runs it, ~340 ms, nearly all of it bash's own start-up (`bash -c true` alone: ~345 ms).
- **The relay:** `glim-hook.exe` (crate `glim-hook`, `windows/hook/`), staged by the app into `%LOCALAPPDATA%\Glim\bin\` at launch. Claude Code runs it per event with the hook JSON on stdin.
- **Data folders:** `%APPDATA%\Glim` (preferences) and `%LOCALAPPDATA%\Glim` (relay, inbox, recap, `glim.log`). On every launch, before anything else, `platform::migrate_data_dirs` moves an older build's `%APPDATA%\Coucou` / `%LOCALAPPDATA%\Coucou` over: a whole-folder rename when the new folder doesn't exist, otherwise entry by entry (folders on both sides are merged), never overwriting; the old folder goes once empty. `coucou.log` becomes `glim.log`; the old `coucou-hook.exe` is deleted. What it did goes to `glim.log` ("data folder migration: …").
- **What the relay sends:** the hook JSON minus `tool_response`, `tool_output` and `transcript_path`; every string cut to 2,000 chars (except `Edit`/`MultiEdit`/`Write` edit strings: 256 KB each, 512 KB per event, for the live diff); plus `cwd`, `term_program`, `wt_session`, `term_session_id`, `vscode_pid`, `session_pid` (from `CLAUDE_CODE_SSE_PORT`), `term_editor`, and `coucou_agent` (`claude-desktop` when `CLAUDE_CODE_ENTRYPOINT=claude-desktop`). The status line mode sends only `session_id` and `rate_limits`.
- **Channel:** a named pipe, `\\.\pipe\glim-<user SID>` (`pipe.rs` server, `hook/src/win.rs` client). The relay checks the pipe server's process belongs to the same user before writing. Not networked. If Glim isn't running the relay exits 0 at once and Claude Code is unaffected.
- **Answers:** only `PermissionRequest` waits — for the island's `allow`/`deny` or question answers, written back on the same pipe and turned into Claude Code's `hookSpecificOutput` by the relay. 300 ms to connect, 800 ms for the island to acknowledge, 108 s for a human; otherwise Claude Code asks in the terminal.
- **Other agents** (Codex, Cursor, Gemini CLI, Antigravity, Copilot CLI, Muse Code, OpenCode, Amp, Hermes) use the same relay with `--agent <name>`, installed into their own config files by `agents.rs` with the same diff/backup/fingerprint flow.

### Rename verification (2026-10-09, `glim.exe` built 16:48)
- **Migration on this machine:** before, `%LOCALAPPDATA%\Coucou` held `bin\coucou-hook.exe`, `coucou.log` (2,105 B) and an empty `inbox\`; `%APPDATA%\Coucou` was empty; `%LOCALAPPDATA%\Glim\inbox` already existed (created by `cargo test`, see lessons). After one launch: both `Coucou` folders gone; `%LOCALAPPDATA%\Glim` holds `bin\glim-hook.exe`, `inbox\` and `glim.log` (3,188 B, the old history plus the migration lines); `%APPDATA%\Glim` exists.
- **Pipe:** while Glim ran, the only matching pipe was `\\.\pipe\glim-S-1-5-21-…-1001`; no `coucou-` pipe. Both binaries contain `pipe\glim-` and not `pipe\coucou-`.
- **End to end:** with Glim running, `claude -p "reply with ok" --model haiku --settings <temp file with Glim's hook entries> --setting-sources project --no-session-persistence` in an empty temp folder replied "ok", exit 0, 4 s; `glim.log` gained `hook SessionStart`, `hook UserPromptSubmit`, `hook Stop`, `hook SessionEnd`. With Glim closed: no pipe, the relay exits 0 in 17 ms, and the same `claude -p` run still exits 0 in 4 s.

## docs/brand/
Committed as provided by the owner, untouched: `glim-mascot.html` (mascot spec, "Glim mascot (approved)"), `app-icon.svg` (1024×1024 app icon, dark tile with the lantern), `app-icon-small.svg` (transparent-background lantern for small sizes). `app-icon.png` (1024×1024) and `app-icon-small.png` (256×256) were added 2026-10-09 and are the icon sources: `npm run icons` runs the Tauri CLI on `app-icon.png` for the large sizes, and on `app-icon-small.png` for `32x32.png` and the 16/24/32 px layers of `icon.ico`. The SVGs are kept as reference.

## Lessons learned
- **Win11 Notepad keeps unsaved edits per file path across launches:** a test that edits a file in Notepad poisons every later run on that path. Never edit; use fresh file names.
- **Chromium leaves role-less divs out of the accessibility tree:** an element a screen reader or UI Automation must find needs a role or label.
- **Smart App Control blocks the Rust toolchain (2026-10-08).** Windows Smart App Control in enforce mode blocks the unsigned `rustc_driver` DLL (`0xC0E90002`). Tauri then panics with an unhelpful `Option::unwrap()` error that points nowhere near the cause. Fix: turn SAC off on the dev machine. This is why code signing is on the roadmap.
- **`gh` defaults a fork's PRs to upstream (2026-10-08).** In a fork, `gh pr create` and similar commands target the parent repo (Louis-CFM/coucou) unless told otherwise. Fix: `gh repo set-default anthonygarcia866/local-companion` (done on this machine); CLAUDE.md forbids any gh activity on upstream.
- **The webview runtime phones home even when the app doesn't (2026-10-09).** A choke point in Rust and a strict CSP still left WebView2 contacting Microsoft (experiment config, component updater, NEL reports, WPAD). Only watching the whole process tree showed it; `glim.exe` alone looked clean. Fix: Chromium switches plus a resolver rule on the WebView2 command line, verified with netstat and a net-log. Any new window must use `BROWSER_ARGS`.
- **UI automation hit the owner's browser (2026-10-09).** While re-verifying Phase 0a, simulated clicks and scrolls aimed at Glim landed in the owner's browser, because the owner was using the machine and the browser was in front. Two clicks went to the browser's tab strip and 12 wheel scrolls to the page underneath. Nothing checked the foreground window first. Fix: the CLAUDE.md rule — no real mouse or keyboard input unless the owner confirms they're away, and one screenshot first to check that only Glim is in front.
- **A rebuilt exe kept the old icon (2026-10-09).** After `npm run icons` replaced `icon.ico`, `npm run pack` succeeded but `glim.exe` still embedded every old icon layer: Cargo didn't rerun the build script that compiles the icon resource. Caught by byte-searching the exe for each `.ico` layer. Fix: `npm run icons` now touches `src-tauri/build.rs`.
- **`cargo test` writes to the real data folder (2026-10-09).** Some tests create `%LOCALAPPDATA%\<data folder>\inbox` under the real profile. Before the rename they touched `Coucou\inbox`, after it `Glim\inbox`, so on a dev machine the migration's "both folders exist" path is the normal case, not an edge case. The first version of the merge left same-named subfolders behind; it now merges them recursively. Fixed at the root in the same PR: under `cargo test`, `platform::data_base` points both data folders at a scratch folder per test thread, `tests_never_use_the_real_data_folders` asserts it, and `windows-ci.yml` fails if any `Glim`/`Coucou` data folder exists after the tests. Checked locally by hashing every file in the real folders before and after `cargo test` + `npm test`: identical. The test-created empty `inbox\` was removed.
- **A crate rename breaks CI silently until it runs (2026-10-09).** `windows-ci.yml` runs `cargo build --release -p coucou-hook`; renaming the crate would have failed CI on the first push. Search `.github/` as well as the source for an identifier you rename.
- **`gen-strings --check` reports "out of date" on a fresh Windows checkout (2026-10-09).** With `core.autocrlf=true`, Git checks `strings.json` out with CRLF and the generator writes LF, so the check fails although the content is identical (`git diff` is empty). Not caused by any change; CI doesn't run that check.
- **A new Tauri window needs a capability, or its listeners fail silently (2026-10-09).** The capture debug panel stayed on "Waiting for a focused field…" while the backend was reading Word fine (the results log had it). Tauri v2 only lets a window use the event API if a file in `src-tauri/capabilities/` lists it; `default.json` named island/settings/character, so `listen()` was refused with no visible error. Fix: `capabilities/capture-debug.json` (listen/unlisten and set-title, that window only). To tell "backend not reading" from "panel not hearing", the panel now mirrors metadata (never text) into its window title, and the capture thread logs focus-event/poll counts and app names every 30 s.
- **Launching Glim stole keyboard focus (2026-10-09, fixed).** The hidden Settings window, created at launch without `.focused(false)`, took the foreground: the app being typed in lost focus to an invisible window. All window builders are now unfocused (test-enforced); verified by reading the foreground window before and after launch and after revealing the island.
- **A generator's output must not depend on line endings (2026-10-09).** The mascot port passed locally and failed on Windows CI: the owner's updated design file is LF in the working copy, CI checks it out as CRLF (`core.autocrlf`), and the port carried the `\r`s into the generated files. `port-lantern.mjs` now normalises its input; a test ports a CRLF copy and an LF copy and requires the same output. Same trap for tests that scan source with `include_str!` (PR #6: a search for a newline-brace-newline failed only on CI): normalise to LF first.
- **A screen-region crop can catch other windows (2026-10-09).** Cropping the island from a screen capture by finding its black pixels broke when the window behind the island was dark too: the "pill" ran the full capture width, so the crop could have held another app's pixels. Those captures were deleted unsaved. Fix: capture Glim's window with `PrintWindow(PW_RENDERFULLCONTENT)`, which renders only Glim's own pixels (transparent parts come out black), and size-check any crop before keeping it.
- **Screenshots of a transparent window show what's behind it (2026-10-09).** The island's window is mostly transparent, so capturing its rectangle captured other apps. Crop to the drawn panel, not the window rect.
