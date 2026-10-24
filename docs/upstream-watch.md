# Upstream watch: Coucou since our fork

Reviewed 2026-10-11. Upstream is [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou), forked at `5cb2a27` (2026-10-08). This review covers `upstream/main` at `0dffbaf`: 151 commits since the fork.

**Coucou is a reference, not a merge source.** Nothing here is merged or cherry-picked. Anything we take is rewritten in Glim's code, under Glim's rules (CLAUDE.md, `docs/vision.md`): one network checkpoint, filtering before anything leaves capture, send only on the owner's click, and no captured text in logs.

## How to look again

- The remote is set up to fetch only: `upstream` fetches from `https://github.com/Louis-CFM/coucou.git`, and its push URL is set to `DISABLED-read-only`, so a push fails.
- To refresh: `git fetch upstream`, then `git log --no-merges 5cb2a27..upstream/main` (or start from the commit this file was last reviewed at).
- Only `windows/` and `docs/` matter to us. `NotchBuddy/` is the Mac app (Swift), `android/` is the new Android app.
- **Licences:**
  - the phone-link code (`windows/src-tauri/src/phone_link/`) and `android/` are © Rhytam Biswas, MIT;
  - everything else is © Louis Raillé, MIT for code;
  - the name, Mochi, the icon, the sounds and the media are reserved (`LICENSE-ASSETS.md`, §3 below).

  Adapting MIT code means keeping its copyright notice: add a line to `NOTICE.md` in the PR that does it.

## 1. Ranked: what's worth taking

| # | Upstream change | Verdict | Why |
|---|---|---|---|
| 1 | **Approval queue** (`e391a14`, Mac): permission requests queue up instead of pushing each other away | **Adapt** | Our Windows island turns a second request away. The first card stays, and the second goes back to the terminal (`island/hooks.ts`, "One card, one request"). It is safe, but with two agents running, one of them waits in a terminal you aren't looking at. A queue is also the first piece of the hub's "one approval queue" (`docs/vision.md` §5). Swift, so rewrite it in TS. Keep our rule that a card is never replaced silently. |
| 2 | **Several sessions on one pill** (`5652ea8`, Mac, `PillSessionRegistry`): concurrent Claude Code sessions on one pill no longer clobber each other | **Adapt** | Same bug class here. `upsert()` keys the pill by project and overwrites `t.sessionId` with the latest session's id. Two sessions in one repo then share one pill and one session id. A stop or error from one can reset the other, and an approval check that compares session ids can match the wrong one. Worth a test first, to confirm it on Windows. |
| 3 | **Android phone link** (`0d0a543` + follow-ups; `docs/ANDROID_LINK.md`, `docs/RELAY_LINK.md`, `phone_link/`) | **Adapt, later** (design reference for the phone companion; no code now) | A careful design, worth following for our phone companion. Its rules match ours: the link is off by default and every capability has its own switch; Allow needs the phone's biometric or screen lock (Deny doesn't); a long command can only be denied from the phone; the decision is bound to a fingerprint of the pending request; the computer's API key never leaves the computer; nothing is logged. **Why not port now:** it opens a TLS listener on `0.0.0.0:47821` and announces itself over mDNS, and the relay mode opens an outbound WebSocket to the user's Cloudflare Worker. Our rule is localhost + `api.anthropic.com` through `net/mod.rs`. A listener and mDNS need an owner decision and a checkpoint for inbound traffic, which `net/mod.rs` doesn't have. It also adds 6+ crates (rustls, tokio-rustls, ring, rcgen, qrcode, mdns-sd, tokio-tungstenite, webpki-roots). The hub (`vision.md` §5) may be the better home: the phone would talk to the always-on hub, not the daily PC. Decide that before designing. |
| 4 | **Voice with Claude as the brain** (`aacd077`, `da2017c`, `323d89e`, `bc2c733`, `96b68e3`; `docs/VOICE.md`), Mac | **Adapt the design** for Phase 1.5; the code is Swift | How it works: the user's Anthropic key, Haiku by default; Claude acts only through a fixed set of tools; it proposes and the app decides. The conversation is kept in memory only, forgotten after 90 s. Web search is opt-in. A failed API call falls back to the local phrase parser. **Rules that match ours:** voice can never approve a permission (it can deny); it can never send an email; the email card opens filled in and stays open until a click (`prepare_email` → the mail card → Send). That is our "suggest and wait for my click". **Differences:** speech-to-text there is on-device Apple Speech. Ours is the whisper-vs-cloud comparison still to run (`vision.md` §4). Their mail goes through Apple Mail via AppleScript; ours is Outlook COM drafts (Phase 1.5). Skip ElevenLabs text-to-speech and the wake word for now (ours is push-to-talk first). |
| 5 | **"Open terminal" picks the session's own window** (`574deec`) | **Skip** (Linux-only) | The Linux port of what Windows already does. The Windows side only got a refactor (`window_owners` / `focus_session_window`, same behaviour). Nothing here fixes our "Open in n8n" bug (§2). |
| 6 | **Island keys: Ctrl+O / Ctrl+E, Ctrl+↑/↓ over lists** (`322e2ab`) | **Skip** | Those keys work only while the island has keyboard focus. Ours never takes focus, so they would go to the app in front. That is the same reason our approval key hints fail (Post-1b issue 4). |
| 7 | **Your own sounds** (`2261334`, `sounds.rs`): the user picks sound files | **Adapt, Phase 4** | Our sounds are planned as synthesized Web Audio, with no files and suggestion sounds off by default. User-supplied files are a fine option on top. Coucou's own sounds stay out (§3). |
| 8 | **CI action bumps** (`a9e5dee`): checkout v7, setup-node v7, cache v6, upload-artifact v7 | **Port** (trivial, in any CI PR) | Keeps our `windows-ci.yml` off deprecated runner versions. No behaviour change. |
| 9 | **Spotify pill** (`spotify.rs`, 1.5k lines), **crypto and mail pills** (Mac), Mochi frame-rate work, wardrobe, Linux Wayland shortcuts, Android app, site pages | **Skip** | They call outside services (Spotify, Coinbase, OKX), are Mac or Linux only, or are Mochi art we don't ship. |

## 2. The "Open in n8n" bug (Post-1b issue 1)

**Upstream has the same bug and hasn't fixed it.** Our error card (`windows/src/views/views.ts`, `buildError`) builds its second button as `btn(tl("Open in n8n"), "secondary", () => actions.openUrl(""))` for every error, whatever the source. Upstream's `buildError` at `0dffbaf` is identical. So it wasn't app or terminal detection: the button is hard-coded. In Glim it is also dead, since `openUrl("")` opens nothing and the n8n integration was removed in Phase 0a.

**Fix ourselves** in the post-1b PR:
- the error card offers **Open terminal** (or **Open Claude** for desktop sessions), like the finished card does;
- "Open in n8n" goes, since n8n is the label of the Ollama/LM Studio chat pills' `source`, not an integration.

**Usage limits (Post-1b issue 2).** `StopFailure` maps every failure to `error` (`island/hooks.ts`), and upstream doesn't tell a usage limit apart either. The `ratelimit` state and the lantern's `s-paused` already exist. What's missing is reading the failure's reason, and the reset time when it's given. Check what Claude Code's `StopFailure` payload carries before designing it.

## 3. Muse Code

**Glim already handles `--agent muse`, and upstream hasn't changed it since the fork.**
- **Install:** Settings writes Glim's hooks to `~/.config/muse/settings.json`. These are Claude-style groups with a `*` matcher, timeouts in milliseconds and `schema_version: 1` for a new file (`agents.rs`, `MUSE_EVENTS`). The events are SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop and SessionEnd.
- **Each hook runs** `glim-hook.exe --agent muse <Event>`.
- **Approvals:** Muse is one of the three external agents whose approvals the island answers (with Codex and Copilot). The relay replies with a bare `permissionDecision` (`hook/src/reply.rs`).
- **Tests:** a round-trip install and uninstall, and the approvals list.

**But on this PC no Muse install or config was found.** There is no `~/.config/muse/`, no `muse` command, and no process or Start-menu entry; the only "muse" folder is a leftover from `cargo test`. So it has never been tried for real here. To do, when the owner next uses Muse on this machine:
- confirm where Muse keeps its settings on Windows: if it isn't `~/.config/muse/settings.json`, Glim installs into the wrong file;
- install the hooks from Settings and run one session;
- check the pill and one approval.

## 4. Coucou assets in Glim: what ships

Upstream's `LICENSE-ASSETS.md` now also reserves "the copies of these assets anywhere else in this repository, such as the icon and the sounds in `android/app/src/main/res/` and `windows/`". Checked against our `main` (`013fe2d`) and the release `glim.exe` (16:55 build):

| Reserved | In Glim? | Detail |
|---|---|---|
| Sounds | **No** | No audio files are tracked; all 29 WAVs were removed in Phase 0a. |
| App and menu-bar icon | **No** | `windows/src-tauri/icons/` (5 files) is built from the owner's Glim PNGs (`6c84a9d`), and none of them matches any file in the fork-point tree. |
| Images, GIFs, videos (`docs/media/`, `design/`, screenshots) | **No** | Those folders don't exist here. The only tracked images are `docs/brand/` (Glim's own) and `docs/verification/` (our test captures). |
| The Mochi character (art) | **No** | The drawing code went in Phase 0a; every leftover placeholder became the lantern (Presence item 3). |
| The name **"Mochi"** | **In code and one log line** | Internal names: `DesktopMochi` (`desktop/main.ts`), the `desktopMochi` settings key, the `desktop_mochi_*` commands, the `src/mochi/` folder. The log line `desktop Mochi: poll` is in `glim.log`. None of these is in the UI, but the log can be seen. **Replace:** rename to "desktop lantern" / `desktop_lantern_*`, migrating the settings key. |
| The name **"Coucou"** | **Yes, in user-visible text and files Glim writes** | Already on the roadmap ("Rest of the Coucou names"). **Replace:** |

The "Coucou" items to replace, in order:
- **Files Glim writes into other agents' configs, and the text that names them:**
  - `~/.copilot/hooks/coucou.json`;
  - OpenCode `~/.config/opencode/plugins/coucou.js`;
  - Amp `~/.config/amp/plugins/coucou.ts` (its header reads "Coucou plugin for Amp");
  - the Hermes plugin `~/.hermes/plugins/coucou/` and the hint "`hermes plugins enable coucou`";
  - Antigravity's `"coucou"` hook group, with its error "A hook group named "coucou"…";
  - the marker comment "generated by Coucou".

  Each needs old-name handling like the relay rename got: recognize and remove the old file, then write the new one.
- **The catalog string** "Writes: • ~/.claude/coucou/nb-hook…" (all 10 languages) is the Mac app's text. No Windows source file uses it (checked), but it is compiled into `glim.exe`. Drop it from the Windows catalog.
- **The payload fields** `coucou_agent` and `coucou_diff_truncated` (relay ↔ island). They aren't visible, but rename them together with the relay.
- **Temp files** named `<name>.coucou-<pid>` next to the user's config files during an atomic write (`config_file.rs`, `recap.rs`). They're briefly visible in those folders. Rename them to `.glim-<pid>`.

Not an asset, but worth fixing at the same time: the installer's `shortDescription` / `longDescription` (`tauri.conf.json`) say "local-only" and "only ever connects to this computer". That stays true until 1c lands; the 1c PR must change it.

## 5. Cherry-picked from upstream (owner's queue, 2026-10-19)

The owner asked for these few upstream fixes as **cherry-picks** (`git cherry-pick -x -m 1 <upstream merge commit>`), each in its own draft PR. That's an exception to "nothing here is merged or cherry-picked" above, made for small, self-contained bug fixes in code Glim still shares with upstream. Never a wholesale merge of upstream.

| Upstream PR | What | Glim PR / status |
|---|---|---|
| [#522](https://github.com/Louis-CFM/coucou/pull/522) (`9300d91`) | Pill labels reserve left padding so the mascot doesn't cover them (`style.css`, 1 line) | `cherry/pill-label-padding`, draft PR |
