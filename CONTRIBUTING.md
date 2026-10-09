# Contributing to Glim

Glim is a local-only Windows companion. Read [CLAUDE.md](CLAUDE.md) for the house rules and [PROJECT_STATUS.md](PROJECT_STATUS.md) for where things stand.

## Getting started

```powershell
cd windows
npm ci
npm run pack
```

`npm run pack` builds `target\release\glim.exe` and the installers in `windows\release\`.

## Tests

From `windows/`:

```powershell
npm test
cargo test --workspace
```

## Rules of the house

- Every network request goes through `windows/src-tauri/src/net/mod.rs`, and only to localhost. A test fails the build otherwise.
- No telemetry, no cloud services, no updater.
- Never write `~/.claude/settings.json` without a backup and the user's confirmation.
- Never block Claude Code: if the app doesn't answer, the hook must exit right away.

## Pull requests

- One topic per PR, with a cropped screenshot for anything visual.
- Update `PROJECT_STATUS.md` in the same PR as any behavior change.
