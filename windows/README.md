# Glim for Windows

The Glim app: Tauri 2, a Rust back end (`src-tauri/`), a TypeScript front end (`src/`, no framework) and the hook relay agents run on each event (`hook/`, built as `glim-hook.exe`). Data lives in `%APPDATA%\Glim` and `%LOCALAPPDATA%\Glim`; an older build's `Coucou` folders are moved there on first launch.

See the [root README](../README.md) for what Glim is, [PRIVACY.md](../PRIVACY.md) for the network rules, and [PROJECT_STATUS.md](../PROJECT_STATUS.md) for where things stand.

## Commands

| | |
|---|---|
| `npm run dev` | Vite + the app in dev mode |
| `npm run pack` | release build, installers copied to `release/` |
| `npm test` | front-end tests (Node's test runner, fake DOM) |
| `cargo test --workspace` | Rust tests, including the network choke point's |
| `npm run icons` | re-render `src-tauri/icons/` from `../docs/brand/app-icon.svg` |

## Where things are

- `src-tauri/src/net/` — the only module that may make an outbound request (allowlist: `127.0.0.1`, `localhost`). A test fails the build if any other Rust file names the HTTP client or a socket type.
- `src-tauri/src/local_chat.rs`, `chat.rs` — the chat, Ollama and LM Studio only.
- `src-tauri/src/hooks.rs`, `agents.rs`, `pipe.rs` — the agents' hooks and the named pipe they report on.
- `src/mochi/` — the character's engine (states, tweens). It draws a placeholder orb for now; the folder keeps its upstream name until the mascot PR.
