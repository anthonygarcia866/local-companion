# Remote runbooks: design (DRAFT for the owner's review, 2026-11-19)

Roadmap TOP item 5. **Design only: nothing here is built.** Code follows only after the owner approves this doc (and its open decisions, §7) in chat.

## 1. What is built today

- The courier's `runbook` command (`remote/exec.rs`, `{"name": "<runbook>"}`) runs a runbook exactly as `glim.exe --run <name>` does (`runbooks::run_now`, `Trigger::Remote`). It must be approved and unchanged since (SHA-256 + HMAC), and it must have no `ask = true` step (refused as `runbook-asks`).
- **Any** approved, non-asking runbook can be started remotely. Nothing in the runbook says "this one may run from the cloud".
- Its result is `"<name>: OK (n/m steps)"`, or `runbook-failed-N-of-M`, or `runbook-<refusal code>` (`not-found`, `not-approved`, `changed`, `asks` …). It carries no step output.
- The usual courier rules already apply: a signed, sealed task, the kill switch, per-client and PC-wide rate limits, one outcome per id, results signed (and sealed) by the PC. The result format from 2026-11-19 adds `status`, the times, `client` and `capped`.

## 2. The owner's rules (2026-11-19)

1. **No secret value ever travels through the relay**: not in a task, not in a result, not in an error.
2. **Only runbooks marked remote-allowed** can be triggered remotely.
3. **`ask = true` steps need the owner's click on the PC**, or the run is refused remotely.
4. **Results report outcomes, never secrets.**

## 3. Design

### 3.1 Remote-allowed is part of the runbook, and of its approval (rule 2)

- A new optional table in the TOML:

  ```toml
  [remote]
  allowed = true
  ```

  Absent or `false` means the runbook is not remote-allowed.
- Because it sits in the file, it is **inside the approved hash**. Turning it on or off means approving the runbook again, on the usual card. The card then shows one more line: **"Cloud Glim may start this runbook (Settings → Remote)."**
- `run_now` with `Trigger::Remote` refuses a runbook without it, as `runbook-not-remote`. This check comes before any step runs, and before any secret is read.
- **Behavior change:** approved non-asking runbooks that are remote-startable today stop being so until the owner opts each one in. That is the point of rule 2, and the PR will call it out.
- Settings → Runbooks marks remote-allowed runbooks with a small "remote" tag. Settings → Remote → Remote tasks lists their names (read only).
- Not proposed: a list of allowed names in Settings, separate from the file. It would be a second source of truth outside the approved hash, and a runbook could change under a name without anyone noticing.

### 3.2 What the task may carry (rule 1)

- The task carries **only `name`**, as today: no parameters, no environment, no step choice. `command::parse` stays strict (`only(args, &["name"])`, name syntax as now).
- Secrets stay where they already are. `env_secret` and `secret_stdin` read from Credential Manager on the PC, at run time, exactly as in a local run. The relay and cloud Glim only ever see the runbook's name.
- **Proposed, for a later PR:** parameters from a fixed enum declared in the TOML (e.g. `[remote] choices = { env = ["test", "dev"] }`). They would be in the approved hash and could never be free text. Out of scope for the first PR (§7, D3).

### 3.3 `ask = true` steps (rule 3)

The default is the safe one: **refused remotely** (`runbook-asks`, as today). An optional mode, proposed for the owner to choose (§7, D1), lets the PC ask:

- **"Ask on this PC"**:
  - The task arrives and the run starts. At the first `ask = true` step, Glim shows the existing ask card on the island: the step's one-liner, "**Requested from cloud Glim (account-2)**", and buttons **Run this step** / **Stop**.
  - It waits at most **10 minutes**, and never past the task's own expiry.
  - Run → the step runs, and the run goes on to the next step (another ask asks again). Stop → the run ends with `runbook-declined-at-N`. No answer → `runbook-ask-timeout-at-N`. Steps already done stay done; nothing is undone. This is the same as a local run stopped at an ask, and the result says which step.
  - **Never auto-approved**: not in away mode, not by voice ("run it" from the mic can't answer a remote ask), not by a second task. Only a real click on the card counts.
  - **Phone ping** (Settings → Phone on, quiet hours respected): one fixed text, "Glim: a remote runbook is waiting for your click on the PC". The text carries no runbook name and no step.
  - While a remote run waits, other remote runbook tasks are refused as `runbook-busy`, so only one remote run at a time.
- Refusing up front (before step 1) when the runbook has any ask step is also possible, and simpler: the cloud learns at once instead of after the earlier steps ran. That is D2.

### 3.4 What the result says (rules 1 and 4)

- The result is the 2026-11-19 format, with `kind: "runbook"` and a fixed-shape `detail` built only from codes and counts:
  - `"<name>: OK (3/3 steps)"`
  - `"<name>: failed at step 2 of 3 (run)"`: the step's **kind**, never its text (paths, commands and secret names stay on the PC).
  - `"<name>: stopped at step 2 of 3 (asked, declined | timed out)"`
- **Never in a result:** step output (it stays in `%LOCALAPPDATA%\Glim\runbooks-output\`, already redacted, for Settings), secret names or values, file contents, command lines, paths.
- `redact_detail` (2026-11-19) still runs over the detail as a second net, though the detail is built from codes only.
- Error codes stay fixed strings (`runbook-not-remote`, `runbook-not-approved`, `runbook-changed`, `runbook-asks`, `runbook-busy`, `runbook-failed-N-of-M`, `runbook-declined-at-N`, `runbook-ask-timeout-at-N`). A refusal message from `Refusal::message()` (English text that can name steps) never goes into a result, only `code()`.

### 3.5 Limits and logs

- Rate: at most **5 remote runbook tasks an hour** per client and **10 on the PC** (inside the existing 30/50 command caps). At most **one remote run at a time**: a second gets `runbook-busy`. glim-update's build lock still applies to its own runbook.
- `runbooks.log` gets `<name> remote(<client>) …` with the outcome code, as now (`Trigger::Remote`), plus the client. The Remote activity list shows `runbook`, the outcome code and (D4) maybe the name.
- The kill switch stops a remote run between steps: the run ends `killed` after the current step, and a waiting ask card closes.

### 3.6 Runbooks that need care

- **glim-update** (`launch_detached` restarts Glim): the run's last step starts the updater and Glim exits, so the result must be sent **before** the detached step fires. The detail would read `"glim-update: launched (n/n steps)"`, `status: done`, meaning launched, not installed. Cloud Glim checks `update-result.json` on the PC afterwards with a later `session-status`-like command. That command doesn't exist yet: it is a follow-up (§7, D5). Recommendation: don't mark glim-update remote-allowed in the first PR.
- **relay-tokens** and anything that changes credentials or the relay itself: **not recommended** for remote. A remote task that rotates the relay's own tokens could cut the line that carries its result. The owner keeps these local-only.
- **Recommended first remote runbooks:** read-only or idempotent ones (a `check`-only health runbook, a test run with no secrets).

## 4. Threats and how the design answers them

| Threat | Answer |
|---|---|
| Relay or a stolen cloud token triggers a runbook | Tasks must verify with that client's signing key (as today); a token alone can't sign. Only remote-allowed runbooks; rate limits; kill switch. |
| A runbook is changed after approval so a remote run does something new | Approval hash (SHA-256 + HMAC, scripts included) is checked just before running (as today); `[remote]` is inside it. |
| A secret leaks through the result | The detail is built only from codes and counts; output stays local; `redact_detail` as a second net; refusal messages never sent. |
| A secret leaks through the task | The task carries only a name (no parameters in v1). |
| A remote run abuses an ask step | Refused by default; with "Ask on this PC", only a real click on the card, never voice, away mode or another task. |
| The cloud floods the owner with ask cards | One remote run at a time, 5/hour per client, a 10-minute wait per ask. |
| A remote run cuts its own line (relay-tokens, glim-update) | Not recommended for remote; glim-update would answer before it launches (D5). |

## 5. Changes (when approved), one PR

- `runbooks/mod.rs`: parse `[remote] allowed`, show it on the approval card (one string × 9 languages), refuse `Trigger::Remote` without it (`Refusal::NotRemote`, code `not-remote`).
- `remote/exec.rs`: build the detail from codes and counts (step kind on failure), one remote run at a time (`runbook-busy`), and the remote runbook rate limit (in `Gate`).
- If D1 = "Ask on this PC": the ask card with "Requested from cloud Glim (<client>)", a 10-minute wait bounded by the task's expiry, the phone ping, and kill-switch close. Otherwise asks stay refused.
- Docs: CLAUDE.md (runbook and courier paragraphs), PRIVACY.md, docs/remote-courier.md, PROJECT_STATUS.md, `examples/runbook.example.toml` (the `[remote]` table, commented out).
- Tests:
  - A non-remote approved runbook is refused remotely and still runs locally.
  - Toggling `[remote]` needs re-approval.
  - The detail carries no path, command or secret name; a failing step reports only its kind.
  - Asks are refused (and, with D1, the card's Run / Stop / timeout outcomes).
  - busy and the rate limit work, and the kill switch works between steps.
  - Rust: a scan that no `Refusal::message()` text reaches a result.

## 6. Out of scope

- Parameters (D3).
- A remote "update status" command (D5).
- Running a runbook on a schedule from the cloud.
- Remote approval of a runbook. **Approval stays a click on this PC, always.**

## 7. Open decisions for the owner

- **D1:** `ask = true` steps remotely: (a) always refused (simplest, today's behavior), or (b) "Ask on this PC" (§3.3). Recommendation: **(a) first**, (b) as its own PR if needed.
- **D2:** with (a), refuse a runbook that has any ask step **before step 1** (recommended: nothing half-done), or run up to the ask and stop there.
- **D3:** fixed-choice parameters later: yes or no.
- **D4:** Remote activity list shows the runbook's name, or only "runbook" and the outcome. Recommendation: **the name** (it's the owner's own list, on the PC).
- **D5:** glim-update from the cloud: not in the first PR (recommended), or answer-before-launch plus a later "update status" command.
- **D6:** the rate limits in §3.5 (5/hour per client, 10 on the PC, one at a time): OK, or other numbers.
