# Project status — local-companion

Repo location (dev machine): `C:\Users\antho\local-companion`

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
- **Phases 0–2: strictly local, no exceptions.** All outbound network access goes through a single choke-point module in Rust (created in Phase 0), which allows localhost only.
- **Phase 3: opt-in.** Data leaves the machine only on per-task approval, after the user sees the exact outgoing payload. Exceptions are added only through the choke-point module.
- Phase 1 text and Phase 2 recordings never flow to Phase 3 automatically. Only user-model entries marked shareable may.
- Enforcement is verified empirically (attempt a blocked request, confirm it fails), not trusted from config.

## Roadmap
- **Phase 0** — network choke-point module in Rust (localhost only), with an empirical blocked-request test.
- **Phase 1** — writing assistant.
- **Phase 2** — SOP recorder.
- **Phase 3** — delegation layer.
- **Code signing before distribution** (e.g. Azure Trusted Signing). Users with Smart App Control on can't run unsigned builds, and a privacy product that asks users to disable a security feature is a non-starter.
- **Strip upstream assets before any distribution.** The Coucou name, the Mochi character and the sounds are © Louis Raillé, not MIT.

## Open questions
- At Phase 3 build time, verify whether Claude Code and Codex CLI can run on a subscription login rather than API keys, and check their current headless flags and permission syntax.

## Current status
Forked, baseline builds, no app-code changes yet.

- **Upstream:** [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou), forked at `5cb2a27` (2026-10-08, "Merge pull request #355 from Louis-CFM/release/0.2.3"). Remotes: `origin` = anthonygarcia866/local-companion, `upstream` = Louis-CFM/coucou.
- **Toolchain** (2026-10-08): git 2.54.0, gh 2.94.0, Node 24.16.0, npm 11.13.0, rustup 1.29.1, rustc/cargo 1.99.0, VS Build Tools 2022 17.14.41 (VC tools), WebView2 runtime 154.0.4258.62.
- **Baseline build** (`cd windows`, `npm run pack`, 2026-10-08): exit 0, unsigned.
  - `windows\release\Coucou-Windows-0.2.0-setup.exe` — 5,194,241 bytes (4.95 MB), sha256 `D496FC7998F4D66F…`
  - `windows\release\Coucou-Windows-0.2.0.msi` — 6,045,696 bytes (5.77 MB), sha256 `7F0A7E6537DC4FE7…`
  - `windows\target\release\coucou.exe` — 8,786,432 bytes, sha256 `0F96598BF83DB9C4…`
  - The build itself fetches NSIS and WiX binaries from GitHub on first run (Tauri bundler). That is build-time only, not app runtime, but note it for offline builds.
- **Baseline launch** (2026-10-08): ran `coucou.exe` directly (not installed). Process responding, window title "Mochi"; the black notch island appeared at the top-center of the screen with Mochi and four session pills. `~/.claude/settings.json` was not modified by the launch.

## Lessons learned
- **Smart App Control blocks the Rust toolchain (2026-10-08).** Windows Smart App Control in enforce mode blocks the unsigned `rustc_driver` DLL (`0xC0E90002`). Tauri then panics with an unhelpful `Option::unwrap()` error that points nowhere near the cause. Fix: turn SAC off on the dev machine. This is why code signing is on the roadmap.
