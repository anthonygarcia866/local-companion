# Glim — vision

Status: **direction set by the owner 2026-10-11.** Local-only AI is no longer a priority. Glim keeps its privacy rules, and its AI runs on the Claude API by default. The phase plans (`docs/phase1-plan.md`, PROJECT_STATUS "Roadmap") follow this file. Where they disagree, this file wins until they are amended.

## 1. Roles

| Who | Job |
|---|---|
| **Claude Code** | **Builds Glim.** Writes the code, tests it, opens PRs, and stops for the owner's review. It never ships a change without that review. |
| **Glim** | **Lives on the owner's PC.** It sees context (the focused field, the active app, and later the screen and the inbox, each behind its own rules), types and clicks for the owner, and runs 24/7 jobs. |
| **The Claude API** | **Glim's brain.** Rewrites, drafts, turning a spoken request into an action, and agent jobs all go to Claude. Glim decides what may be sent and what may be done with the answer. Claude only writes the answer. |

## 2. AI: the Claude API by default

- **Every AI feature uses the Claude API by default:** 1c rewrite first, then email drafts, spoken actions (Phase 1.5), SOP writing (Phase 2) and agent jobs (Phase 3).
- **Model per task:** a **fast, cheap model (Haiku-class) by default.** A stronger model (Sonnet- or Opus-class) can be chosen per task in Settings. Model IDs and prices are checked against Anthropic's docs when each feature is built, never hardcoded from memory.
- **API key:**
  - The owner enters it in **Settings** and it is stored in **Windows Credential Manager** (the `windows` crate's `CredWriteW` / `CredReadW`, no new crate). Phase 0a removed the `keyring` crate, so this adds credential storage back on purpose, and the PR says so.
  - Only Rust reads it. It never reaches a webview after it is entered, and it is never logged or written to `settings.json`.
  - Settings shows it masked, with **Remove**.
- **The "brain" setting:** Settings picks the brain for 1c, **Claude API (default)** or **local model** (gemma3 through Ollama, `net::request` to localhost as today). It can be switched at any time without rebuilding. The local model is an **optional offline fallback, not the default**, and nothing new is built for it before the Claude path works.
- **Local model benchmarking is deferred** until the owner's next laptop (32 GB+ RAM). This one has 16 GB and is already memory-tight, so the API stays the default.
- **Spend:**
  - Settings shows **this month's API spend**, worked out from the token counts in each response times the model's price.
  - An **optional monthly cap** can be set. Once it is reached, Glim sends nothing more that month and the island says why.
  - This is Glim's own estimate. Anthropic's console bill is the real number, and Settings says so.

## 3. Privacy rules (kept, and how they change)

The product is still "Glim decides what leaves the machine." What changes is the destination list, not the discipline.

- **Network:** the rule changes from "localhost only" to **"localhost + the Anthropic API"**, still through the **single checkpoint in `windows/src-tauri/src/net/mod.rs`**.
  - `ALLOWED_HOSTS` gains exactly `api.anthropic.com`, and only for **https on 443** with certificate checks on. No other Anthropic host is added, and no wildcard.
  - Redirects are still re-checked hop by hop, and proxies are still never used.
  - The guard test that forbids sockets and `reqwest` outside `src/net/` stays as it is.
  - The webviews stay locked to localhost (CSP and `BROWSER_ARGS` unchanged). API calls are made from Rust only, so the key and the text never pass through a webview's network stack.
  - Verified empirically, as before: a request to another host is refused, and netstat over the whole process tree shows only loopback and Anthropic's addresses.
- **Kept, unchanged:**
  - **Card, SSN, IBAN and payment filtering** happens before anything leaves capture. Every request body is scanned again just before it is sent (`finish_reading`'s checks). A hit sends nothing.
  - **Send only on the user's request:** a click, or the rewrite hotkey (owner, 2026-10-11), which sends that one paragraph. Nothing goes to the API in the background. What comes back is a suggestion, typed in only on a click. §3 of the phase 1 plan floated a slower background check; that idea is dropped.
  - **No logging of captured text.** Request and response bodies are never logged or stored. The log gets an outcome code, the model, the token counts and the cost.
  - **New powers start as "suggest and wait for my click".** Anything that types, sends, deletes, buys, publishes or runs a job is first shown as a suggestion. It only moves to "do it without asking" if the owner later says so for that one power.
- **The mascot shows it:** while text is going to the API, the lantern is in `s-delegate`, the state reserved for data leaving the machine (owner, 2026-10-11).
- **Not changed:** the Phase 3 memory policy (summaries only, a hard sensitive-data filter, tenants by role), per-app exclusions, the paused sites and apps, and the reserved mascot states.

## 4. Phase 1.5 voice: measure before choosing

Speech-to-text is **not decided.** Before anything is built, compare **local whisper** (whisper.cpp, the model picked by speed on this CPU) with **cloud speech-to-text** on:

- **cost:** per hour of dictation, at the owner's real use;
- **speed:** time from releasing the key to text appearing;
- **accuracy:** on the owner's own speech, including property-management words, unit numbers and names.

Same clips for every option. The results go in `docs/` with a recommendation, and the owner chooses. A cloud option would need its host added to the checkpoint like Anthropic's, and the PR says so. Audio is never stored, whichever wins. Turning the transcript into an action or into clean dictated text uses the Claude API (§2).

## 5. Long term: the hub

- **When the owner upgrades their laptop, the current laptop becomes the always-on hub.**
- **The hub holds:**
  - the **rules** (what Glim may do, where and when);
  - **one approval queue** for everything waiting on the owner;
  - the **open items** (tasks, follow-ups, threads in progress);
  - the **scheduler** for 24/7 jobs.
- **Exposed via MCP**, so Glim on the owner's daily PC, Claude Code and other agents all read and write the same state instead of each keeping its own.
- **It replaces Muse/Jarvis job by job:** each job moves only once its hub version works, has been checked against the old one, and the owner says to switch. Nothing is cut over in bulk.
- Not designed yet. Before any code: how the PCs reach each other (LAN only, or a tunnel), how the MCP endpoint is authenticated, and what the network rule becomes on each machine.
