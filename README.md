# Glim

A local-first Windows desktop companion that lives at the top of your screen. It shows your coding agents' sessions (Claude Code, Codex, Cursor, Gemini CLI and others), lets you approve and answer them from the island, and chats with a model running on your own computer (Ollama or LM Studio).

**Glim only ever connects to this computer.** Every outbound request goes through one Rust module that allows `127.0.0.1` and `localhost` and nothing else — see [PRIVACY.md](PRIVACY.md).

> Status: Phase 0a (rebrand + local-only lockdown). The mascot is a placeholder orb until the next PR. See [PROJECT_STATUS.md](PROJECT_STATUS.md) for the roadmap.

## Build

Windows 10/11 with Node, npm, Rust (stable) and the MSVC build tools:

```
cd windows
npm install
npm run pack
```

The installers land in `windows/release/` (`Glim-Windows-<version>-setup.exe` and `.msi`); the app itself is `windows/target/release/glim.exe`. Builds are unsigned for now (code signing is on the roadmap).

Tests: `npm test` (front end) and `cargo test --workspace` (Rust), both from `windows/`.

## Layout

- `windows/` — the app (Tauri 2: Rust in `src-tauri/`, TypeScript in `src/`, the hook relay in `hook/`).
- `windows/src-tauri/src/net/` — the network choke point.
- `docs/brand/` — Glim's name, mascot and icon artwork.
- `tests/fake_local_llm.py` — an OpenAI-compatible stand-in server for testing the local chat path.

Upstream's macOS/iOS app, its iPhone relay and its Linux packaging were removed from this fork; they remain in [upstream's repo](https://github.com/Louis-CFM/coucou).

## License

Code: MIT (see [LICENSE](LICENSE)). Glim is a fork of [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé — see [NOTICE.md](NOTICE.md) for what was and wasn't carried over.
