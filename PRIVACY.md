# Privacy

Glim is local-only. In Phases 0–2 nothing it does leaves this computer. This file says what is enforced, and where in the code, so each claim can be checked.

## What is enforced, and where

| Rule | Where | How it is checked |
|---|---|---|
| Every outbound HTTP request goes through one module. | `windows/src-tauri/src/net/mod.rs` | `net::tests::no_other_module_can_reach_the_network` scans every Rust source file outside `src/net/` and fails if one names `reqwest`, `TcpStream`, `TcpListener`, `UdpSocket`, `tokio::net::Tcp*/Udp*`, `ToSocketAddrs`, `WinHttp`, `WinInet` or `URLDownloadToFile`. |
| Only `127.0.0.1` and `localhost` (any port, http or https, no credentials in the URL) are allowed. Anything else — another host, a LAN address, `::1`, `0.0.0.0`, other `127.x` addresses, `*.localhost` — is refused before a socket opens. | `net::ALLOWED_HOSTS`, `net::check`, `net::request` | `only_127_0_0_1_and_localhost_are_allowed`; `a_request_to_an_external_host_is_refused_before_anything_is_sent` tries `https://example.com` through the real request path and expects the refusal. |
| A redirect may only lead to another allowed address. | `net::request` (custom redirect policy) | `a_redirect_off_this_machine_is_refused` serves a real `302 → https://example.com` on loopback and expects the request to fail. |
| No proxy is ever used (a system or `HTTP_PROXY` proxy would carry the request off the machine). | `net::request` (`no_proxy()`) | `a_system_proxy_is_never_used` sets `HTTP_PROXY`/`ALL_PROXY` to a fake proxy and checks the answer came straight from the local server. |
| `localhost` resolves to `127.0.0.1` inside the app, whatever the hosts file says. | `net::request` (`resolve("localhost", 127.0.0.1)`) | — |
| A model server address typed in Settings is refused unless it is on this machine. | `net::normalise_server_url` (called by `local_chat.rs`) | `a_pasted_server_address_is_cleaned_up`, `connect_cleans_a_pasted_address_and_reports_whether_it_is_this_machine` |
| The webviews can only fetch from the app itself, the IPC bridge, and loopback. | `windows/src-tauri/tauri.conf.json` → `app.security.csp`: `connect-src ipc: http://ipc.localhost http://127.0.0.1:* http://localhost:*` (plus `object-src 'none'`, `form-action 'none'`, `frame-src 'none'`) | — |
| The WebView2 runtime makes no background connections: no Edge experiment config, component updater, network-error reports, pings or proxy auto-discovery, and no host name but `localhost` resolves inside the webview. | `BROWSER_ARGS` in `windows/src-tauri/src/lib.rs` and `additionalBrowserArgs` in `tauri.conf.json`: `--disable-background-networking --disable-component-update --disable-domain-reliability --no-pings --no-proxy-server "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE *.localhost"` | `every_window_asks_webview2_for_the_same_locked_down_arguments` keeps both copies identical and the switches present. Observed with netstat and a WebView2 net-log (2026-10-09): without them the runtime called `config.edge.skype.com`, `edge.microsoft.com` and `ecs.nel.measure.office.net`; with them, nothing. |
| "Open in browser" only opens loopback pages. | `open_url` in `windows/src-tauri/src/lib.rs` → `net::check_str` | — |
| No updater, no telemetry, no crash reporting. | `windows/src-tauri/Cargo.toml` has no updater or analytics plugin; `tauri.conf.json` has no `plugins.updater` | — |
| No API keys, no Credential Manager. | The `keyring` dependency and `secrets.rs` were removed. | — |

The tests run with `cargo test --workspace` from `windows/`. The empirical check of a running build (connections observed with `netstat` filtered to the app's processes) is recorded in `PROJECT_STATUS.md`.

## What was removed in Phase 0a

- Cloud chat providers: Anthropic, OpenAI, Google AI, OpenRouter, and the "any OpenAI-compatible server" option (which could point anywhere), with their API-key storage in the Windows Credential Manager.
- Web-service pollers: GitHub, Stripe, Vercel, Notion, Resend, Cal.com, n8n.
- The Codex plan pill, which ran `codex app-server` (a process that asks OpenAI's service for the plan limits).
- The domain-directory lookup of the user's display name (`GetUserNameExW`), which can query a domain controller. Only the local account database is read now.

## What still talks to other processes on this machine

- **Agents' hooks** (Claude Code, Codex, Cursor, Gemini CLI, Copilot CLI, Muse Code, OpenCode, Amp, Hermes): the agent runs the relay `coucou-hook.exe` on each event, and the relay forwards the event to Glim over a named pipe `\\.\pipe\coucou-<user SID>`. Nothing in this path is networked. Details in `PROJECT_STATUS.md` → "Claude Code integration".
- **Ollama / LM Studio** on `127.0.0.1`. If you point `OLLAMA_HOST` at another machine, the allowlist refuses it.
- **Launching other apps on request**: "Open terminal" brings the session's terminal forward or opens VS Code (`code`), the Claude Desktop pill opens the Claude app, and "Show folder" opens Explorer. Those apps are separate programs with their own network behaviour; Glim only starts or focuses them when you click.

## Not covered by the choke point

- The **build** downloads Rust crates, npm packages, and the NSIS/WiX installers (Tauri bundler) — build time only, never at runtime.
- The **WebView2 runtime** is Microsoft's component and is updated by Microsoft's own updater service, outside Glim's processes. Inside Glim's processes its background traffic is switched off (table above) — checked empirically, not assumed (see `PROJECT_STATUS.md` → "Phase 0a verification"). A future runtime version could add a service those switches don't cover; the host-resolver rule is the backstop for anything that uses a host name.
- **Your own status line command**: if you install the plan-usage relay and you already had a Claude Code status line, the relay runs that command for you, as Claude Code would. What it does is up to it.
