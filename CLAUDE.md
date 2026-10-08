# local-companion — guide for AI coding agents

local-companion is a fork of [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou), being turned into a local-first Windows desktop companion. The vision, phases, privacy model and current status live in `PROJECT_STATUS.md`.

## Standing rules
- Read `CLAUDE.md` and `PROJECT_STATUS.md` at the start of every session.
- Update `PROJECT_STATUS.md` in the same PR as any behavior change, bugfix, or roadmap change.
- Windows shell: one command at a time, no `&&`; `curl.exe` not `curl`; npm only (no bun, pnpm or yarn).
- Never run `gh pr merge` without the owner's explicit approval in chat.
- Never commit secrets or keys; `.env` files are gitignored.
- Verification: a tool reporting success is not proof. After any build, confirm the output reflects the change (rebuild, check exe timestamp/size/hash, launch it). Prefer hands-on checks in real apps over passing unit tests alone.
- Privacy (this is the product): all outbound network access goes through a single choke-point module in Rust (created in Phase 0). Phases 0–2 allow localhost only. Phase 3 may add gated exceptions only through that module, behind a per-task approval UI. No other code path may make network calls or spawn networked processes. Any change adding a network call, telemetry-bearing dependency, or updater must be called out in the PR description. Enforcement must be verified empirically (attempt a blocked request, confirm it fails), not trusted from config.
- If a plan step turns out wrong, amend the plan doc with a warning note — don't silently fix code around it.
- The Coucou name, the Mochi character, and the sounds are © Louis Raillé, not MIT (see `LICENSE-ASSETS.md`) — never ship them.

## Build (Windows)
```
cd windows
npm install
npm run pack
```
Installers land in `windows/release/`. `npm run tauri dev` runs the app without packaging.

## Upstream guide (reference only)
The section below is upstream Coucou's agent guide, kept for orientation in the inherited code. Where it conflicts with the standing rules above, the standing rules win. Its App Store, bundle-identifier and "never restyle" rules protect upstream's shipping build and do not bind this fork.

### Where things are
- `NotchBuddy/Sources/App/` — Mac-only Swift code. `NotchBuddy/Sources/CoucouKit/` — code shared with the iPhone app (Mochi's BotEngine and outfits, pills, diff, models). `NotchBuddy/Sources/Phone/` — iPhone app (`CoucouPhone` target), with its widgets in `Sources/Widgets/` and the expanded approval notification in `Sources/NotificationContent/`. `NotchBuddy/Resources/sounds/` — the 28 WAV sounds. `NotchBuddy/project.yml` — XcodeGen project (never edit the `.xcodeproj` by hand).
- `NotchBuddy/Sources/CoucouKit/PillCatalog.swift` — single source of truth for all declared pills (workspace tools, agents, AI providers, services). Every pill ID, color, category and subtitle lives here.
- `docs/SPEC.md`, `docs/INTEGRATIONS.md` — behaviour, views, states, integrations (in French).
- `design/prototype/notch-buddy.html` — original prototype, the visual source of truth. `design/captures/` — target screenshots.
- `windows/` — the Tauri app for Windows and Linux: Rust in `src-tauri/`, TypeScript in `src/`, the `coucou-hook` relay in `hook/`. `windows/README.md` lists what differs from the Mac.
- `docs/*.html` — the GitHub Pages site (privacy, terms, support, legal notice).
- `relay/` — the Coucou relay, a stateless Cloudflare Worker that forwards the iPhone Live Activity pushes (it holds the APNs key, which must never ship in an app). See `relay/README.md`.

### Upstream rules
- Swift 6, SwiftUI + AppKit. No third-party dependencies unless truly unavoidable. The character is drawn in code (`Canvas` + `TimelineView`), no Rive/Lottie/images.
- Secrets live in the Keychain, never on disk or in git.
- No telemetry. Network calls only to services the user configured.
- Never block Claude Code: if the app doesn't answer, the hook exits immediately.
- Never overwrite `~/.claude/settings.json`: dated backup, merge, show the diff, write only after the user confirms.
- Never send an email or approve a Claude Code or Codex permission without an explicit click.
- Performance: 0 % CPU when the island is hidden.
- Keep the bundle identifier `fr.louisraille.NotchBuddy` (Keychain items, preferences and permissions depend on it).
- Never restyle what already ships (pills, cards, Settings, chat…): existing views stay exactly as they are in `main`, which is the App Store build. Change the look of an existing view only when explicitly asked.
- Pill IDs are stable contract values (Keychain, UserDefaults, hook routing): never rename an existing pill ID.
- New views follow the existing app style. `design/prototype/notch-buddy.html` and `design/captures/` are references for new work, not a reason to change existing views.
- Every release adds its CHANGELOG.md section, a row in the README Versions table, and commits the regenerated Info.plist with the new version.
