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

### Phase 2 — SOP recorder
- User-initiated only.
- Windows Graphics Capture + UIA events.
- The user states the goal at record time.
- A local LLM writes a real SOP from the recording.
- Output is editable Markdown with screenshots, PDF export, and redaction.

### Phase 3 — delegation layer (opt-in)
- A local user model: Markdown, user-stated facts only, user-editable, bootstrappable from the user's notes.
- The user model is injected into tasks delegated to Claude Code (headless) and Codex CLI.
- Loop: formulate the task, run it, read the result, follow up, report back.
- "It processes, I decide": sends, purchases, publishes and deletes wait for explicit approval. This is enforced through each CLI's tool allow/deny permissions, not prompt wording.
- Each hop is verified against the artifact (diff/build/test), never the agent's own report.

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
- **Phase 0a** — rebrand to Glim, strip upstream assets, network choke point, local-only lockdown. *PR open (this branch).*
- **Phase 0b** — the real Glim mascot (from `docs/brand/glim-mascot.html`) replaces the placeholder orb.
- **Phase 1** — writing assistant.
- **Phase 2** — SOP recorder.
- **Phase 3** — delegation layer.
- **Code signing before distribution** (e.g. Azure Trusted Signing). Users with Smart App Control on can't run unsigned builds, and a privacy product that asks users to disable a security feature is a non-starter.
- **Rename the integration identifiers** (DECIDED 2026-10-09: one PR after Phase 0a merges, incl. migrating the existing data folder and an end-to-end hook check; deferred, see "Claude Code integration"): `coucou-hook.exe`, the `coucou-hook` marker in `~/.claude/settings.json`, the `\\.\pipe\coucou-<SID>` pipe, the `%APPDATA%\Coucou` / `%LOCALAPPDATA%\Coucou` folders, the other agents' `coucou.*` plugin/config names and the `coucou_agent` payload field. They must change together, with a migration for existing installs.
- **Delete the non-Windows trees** (DECIDED 2026-10-09: separate small PR after Phase 0a merges): `NotchBuddy/` (macOS/iOS app — still contains Coucou-branded Swift code that draws Mochi), `relay/` (upstream's Cloudflare Worker for the iPhone link — a cloud service), `linux/`, `scripts/*.sh`, `tests/*.swift`, `docs/*.md` (upstream's docs), and the `build.yml`/`release.yml`/`linux.yml` workflows. None of it is built or shipped by Glim today.

## Decided 2026-10-09
- Bundle identifier is `com.anthonygarcia.glim` (was `com.glim.app`, which made the bundler warn about the macOS `.app` extension).
- `docs/brand/` artwork is all rights reserved; the code stays MIT (see `NOTICE.md`).

## Open questions
- At Phase 3 build time, verify whether Claude Code and Codex CLI can run on a subscription login rather than API keys, and check their current headless flags and permission syntax.

## Current status
Phase 0a implemented on branch `phase-0a-lockdown` (PR open, not merged).

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

## Removed in Phase 0a
- **Network paths:** Anthropic Messages API (`api.anthropic.com`, with web search), OpenAI / Google AI / OpenRouter (`openai_compat.rs`), the "any OpenAI-compatible server" option that could point anywhere; GitHub, Stripe, Vercel, Notion, Resend, Cal.com and n8n pollers (`integrations.rs`, `github.rs`); the Codex plan pill, which spawned `codex app-server` (it asks OpenAI for the limits); the domain-directory display-name lookup (`GetUserNameExW`); "open in browser" for arbitrary URLs (now loopback only). No updater existed upstream; none was added.
- **Secrets:** `secrets.rs` and the `keyring` crate (Windows Credential Manager / Secret Service). Glim stores no keys.
- **Upstream assets:** all 29 WAVs, the Xcode asset catalogs (Coucou icons), `docs/media/`, `design/`, `windows/screenshots/`, upstream's website pages; the Mochi drawing code in the Windows app (engine face/hands/outfits, the greeting's Mochi, the drop animation's mailbox Mochi, the recap image's Mochi), the wardrobe (outfits), sounds and their settings, the mute and wardrobe shortcuts (now reserved, never registered).

## Claude Code integration
Documented as found; **not modified** in Phase 0a (the identifiers below still say `coucou`).

- **What Glim writes to `~/.claude/settings.json`** — only after an explicit click in Settings, after showing the exact diff, taking a dated backup, and checking the file hasn't changed since the preview (`hooks.rs` → `config_file.rs`):
  - `hooks.<Event>[]` gets one entry per event: `{"hooks":[{"type":"command","command":"\"C:/Users/<you>/AppData/Local/Coucou/bin/coucou-hook.exe\" <Event>","timeout":N}]}` for `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop`, `StopFailure`, `SubagentStart`, `SubagentStop` (timeout 10 s) and `PermissionRequest` (120 s). Entries are recognised by the `coucou-hook` substring; uninstall removes only those.
  - Optionally (separate switch, "Plan usage"): `statusLine.command` becomes `"…/coucou-hook.exe" --statusline`. A status line the user already had is saved to `%LOCALAPPDATA%\Coucou\bin\statusline-previous.json` and the relay keeps running it.
  - On this machine `~/.claude/settings.json` has no Glim/Coucou entries (checked 2026-10-09, before and after the verification runs).
- **The relay:** `coucou-hook.exe` (crate `windows/hook/`), staged by the app into `%LOCALAPPDATA%\Coucou\bin\` at launch. Claude Code runs it per event with the hook JSON on stdin.
- **What the relay sends:** the hook JSON minus `tool_response`, `tool_output` and `transcript_path`; every string cut to 2,000 chars (except `Edit`/`MultiEdit`/`Write` edit strings: 256 KB each, 512 KB per event, for the live diff); plus `cwd`, `term_program`, `wt_session`, `term_session_id`, `vscode_pid`, `session_pid` (from `CLAUDE_CODE_SSE_PORT`), `term_editor`, and `coucou_agent` (`claude-desktop` when `CLAUDE_CODE_ENTRYPOINT=claude-desktop`). The status line mode sends only `session_id` and `rate_limits`.
- **Channel:** a named pipe, `\\.\pipe\coucou-<user SID>` (`pipe.rs` server, `hook/src/win.rs` client). The relay checks the pipe server's process belongs to the same user before writing. Not networked. If Glim isn't running the relay exits 0 at once and Claude Code is unaffected.
- **Answers:** only `PermissionRequest` waits — for the island's `allow`/`deny` or question answers, written back on the same pipe and turned into Claude Code's `hookSpecificOutput` by the relay. 300 ms to connect, 800 ms for the island to acknowledge, 108 s for a human; otherwise Claude Code asks in the terminal.
- **Other agents** (Codex, Cursor, Gemini CLI, Antigravity, Copilot CLI, Muse Code, OpenCode, Amp, Hermes) use the same relay with `--agent <name>`, installed into their own config files by `agents.rs` with the same diff/backup/fingerprint flow.

## docs/brand/
Committed as provided by the owner, untouched: `glim-mascot.html` (mascot spec, "Glim mascot (approved)"), `app-icon.svg` (1024×1024 app icon, dark tile with the lantern), `app-icon-small.svg` (transparent-background lantern for small sizes). `app-icon.png` (1024×1024) and `app-icon-small.png` (256×256) were added 2026-10-09 and are the icon sources: `npm run icons` runs the Tauri CLI on `app-icon.png` for the large sizes, and on `app-icon-small.png` for `32x32.png` and the 16/24/32 px layers of `icon.ico`. The SVGs are kept as reference.

## Lessons learned
- **Smart App Control blocks the Rust toolchain (2026-10-08).** Windows Smart App Control in enforce mode blocks the unsigned `rustc_driver` DLL (`0xC0E90002`). Tauri then panics with an unhelpful `Option::unwrap()` error that points nowhere near the cause. Fix: turn SAC off on the dev machine. This is why code signing is on the roadmap.
- **`gh` defaults a fork's PRs to upstream (2026-10-08).** In a fork, `gh pr create` and similar commands target the parent repo (Louis-CFM/coucou) unless told otherwise. Fix: `gh repo set-default anthonygarcia866/local-companion` (done on this machine); CLAUDE.md forbids any gh activity on upstream.
- **The webview runtime phones home even when the app doesn't (2026-10-09).** A choke point in Rust and a strict CSP still left WebView2 contacting Microsoft (experiment config, component updater, NEL reports, WPAD). Only watching the whole process tree showed it; `glim.exe` alone looked clean. Fix: Chromium switches plus a resolver rule on the WebView2 command line, verified with netstat and a net-log. Any new window must use `BROWSER_ARGS`.
- **UI automation hit the owner's browser (2026-10-09).** While re-verifying Phase 0a, simulated clicks and scrolls aimed at Glim landed in the owner's browser, because the owner was using the machine and the browser was in front. Two clicks went to the browser's tab strip and 12 wheel scrolls to the page underneath. Nothing checked the foreground window first. Fix: the CLAUDE.md rule — no real mouse or keyboard input unless the owner confirms they're away, and one screenshot first to check that only Glim is in front.
- **A rebuilt exe kept the old icon (2026-10-09).** After `npm run icons` replaced `icon.ico`, `npm run pack` succeeded but `glim.exe` still embedded every old icon layer: Cargo didn't rerun the build script that compiles the icon resource. Caught by byte-searching the exe for each `.ico` layer. Fix: `npm run icons` now touches `src-tauri/build.rs`.
- **Screenshots of a transparent window show what's behind it (2026-10-09).** The island's window is mostly transparent, so capturing its rectangle captured other apps. Crop to the drawn panel, not the window rect.
