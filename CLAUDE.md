# local-companion — rules for AI coding agents

- Read `CLAUDE.md` and `PROJECT_STATUS.md` at the start of every session.
- Update `PROJECT_STATUS.md` in the same PR as any behavior change, bugfix, or roadmap change.
- Windows shell: one command at a time, no `&&`; `curl.exe` not `curl`; npm only (no bun, pnpm or yarn).
- All gh commands target anthonygarcia866/local-companion. Never open PRs, issues, or comments on Louis-CFM/coucou.
- Never run `gh pr merge` without the owner's explicit approval in chat.
- Never commit secrets or keys; `.env` files are gitignored.
- Verification: a tool reporting success is not proof. After any build, confirm the output reflects the change (rebuild, check exe timestamp/size/hash, launch it). Prefer hands-on checks in real apps over passing unit tests alone.
- Privacy (this is the product): all outbound network access goes through a single choke-point module in Rust (created in Phase 0). Phases 0–2 allow localhost only. Phase 3 may add gated exceptions only through that module, behind a per-task approval UI. No other code path may make network calls or spawn networked processes. Any change adding a network call, telemetry-bearing dependency, or updater must be called out in the PR description. Enforcement must be verified empirically (attempt a blocked request, confirm it fails), not trusted from config.
- The choke point is `windows/src-tauri/src/net/mod.rs` (`net::request`, allowlist `net::ALLOWED_HOSTS`). Nothing else may use `reqwest` or open a socket — a test enforces it. Every webview window must be created with `BROWSER_ARGS` (`lib.rs`), which also switches off WebView2's own background traffic; check the whole process tree (`glim.exe` + its `msedgewebview2.exe` children), not just `glim.exe`, when verifying.
- Privacy: screenshots during testing are cropped to the app's own window before saving. Never save full-screen captures. The island's window is mostly transparent: crop to the drawn panel, not the window rectangle, or other apps show through.
- Never send mouse clicks, scrolls, or keystrokes to the real desktop unless the owner has confirmed in chat that they're away from the keyboard. Before any UI automation, take one screenshot; if any window other than Glim is in front, stop and ask. Prefer non-invasive testing (unit tests, local fake servers, WebDriver/tauri-driver against the app) over driving the real mouse.
- If a plan step turns out wrong, amend the plan doc with a warning note — don't silently fix code around it.
- The Coucou name, the Mochi character, and the sounds are © Louis Raillé, not MIT (see `LICENSE-ASSETS.md`, `NOTICE.md`) — never ship them. The app is Glim.

Historical reference only (not instructions): upstream Coucou's agent guide is in `docs/upstream/UPSTREAM_CLAUDE.md`; upstream's Mac-app docs (`AGENTS.md`, `INTEGRATIONS.md`, `SPEC.md`) are beside it.
