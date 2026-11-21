# Remote runbooks: design (owner decisions 2026-11-20)

Roadmap TOP item 5. The owner decided D1-D6 in chat on 2026-11-20 (§7). They replace the draft answers of 2026-11-19. Build order, one small PR each, merged per the rule:

1. **Levels 1 and 3, plus editable limits** (§3.1, §3.5).
2. **Checkpoint/resume, plus the `run-status` courier command** (§3.3). Built 2026-11-20: `remote/runs.rs`. Until PR 3, nothing can approve a paused run: it only expires (24 h) or ends `killed`. A run still running when Glim stops ends `failed … (interrupted)` at the next start (its step may or may not have finished, so it is never resumed).
3. **Phone approval (level 2)**: owner answered §8 on 2026-11-21; built as PR 3 (the page in a PR of its own, owner-merge only).

No runbook is remote-allowed after any of these PRs. The owner picks levels afterwards, one runbook at a time, by editing its file and approving it again.

## 1. What is built today (before PR 1)

- The courier's `runbook` command (`remote/exec.rs`, `{"name": "<runbook>"}`) runs a runbook exactly as `glim.exe --run <name>` does (`runbooks::run_now`, `Trigger::Remote`). The runbook must be approved and unchanged since (SHA-256 + HMAC). It must have no `ask = true` step, or it is refused as `runbook-asks`.
- **Any** approved, non-asking runbook can be started remotely. Nothing in the runbook says "this one may run from the cloud". PR 1 ends that.
- Its result is `"<name>: OK (n/m steps)"`, `runbook-failed-N-of-M`, or `runbook-<refusal code>`. It carries no step output.
- The usual courier rules apply: a signed task (sealed, with Require sealed tasks on since 2026-11-20), the kill switch, rate limits, one outcome per id, and results signed and sealed by the PC.

## 2. The owner's rules (2026-11-19)

1. **No secret value ever travels through the relay**: not in a task, a result or an error.
2. **Only runbooks marked remote-allowed** can be triggered remotely.
3. **A step that needs the owner's OK** gets it from the owner (level 2: their phone, §8), never from the cloud.
4. **Results report outcomes, never secrets.**

## 3. Design

### 3.1 Remote level: in the runbook's own file (D1)

- A new optional table and step field in the TOML:

  ```toml
  [remote]
  level = "cloud"   # "cloud" | "phone" | "never"; absent = "never"

  [[step]]
  type = "run"
  # ...
  remote = "phone"  # optional, per step: raises this step's level (never lowers it)
  ```

- The levels:
  1. **`cloud`, "cloud may run"**: runs from a signed cloud task with no ask.
  2. **`phone`, "ask on my phone"**: the run pauses there and waits for the owner's approval, signed by a key that lives only on their phone (§8). **Until PR 3 is built, a level-2 step is refused remotely** (`runbook-needs-phone`), before step 1 runs.
  3. **`never`, "never remotely"**: refused from the cloud as `runbook-not-remote`. **This is the default** for any runbook without `[remote]`.
- A step's level is the stricter of the runbook's level and its own `remote`, with `never` > `phone` > `cloud`. A step can't be less strict than its runbook. An `ask = true` step is at least `phone` remotely, since the phone replaces the click on the PC.
- If any step is `never`, the whole runbook is refused remotely **before step 1**. Nothing ends up half-done.
- Because the level sits in the file, it is **inside the approved hash**. Changing it means approving the runbook again on the usual card. The card shows one more line: **"Remote: cloud may run"**, **"Remote: ask on my phone"**, or nothing for `never`. Steps marked `phone` show "(asks on your phone when run remotely)".
- `run_now` with `Trigger::Remote` checks the levels before any step runs and before any secret is read.
- **Behavior change (PR 1):** approved non-asking runbooks that are remote-startable today stop being so until the owner sets a level. That is rule 2.
- Settings → Runbooks shows a small "remote: cloud" or "remote: phone" tag. Settings → Remote → Remote tasks lists remote-allowed runbooks by name, read only.

### 3.2 What the task may carry (rule 1, D3)

- PR 1 and PR 2: **only `name`**, as today. `command::parse` stays strict.
- Secrets stay where they are. `env_secret` and `secret_stdin` read from Credential Manager on the PC at run time. The relay and cloud Glim only ever see the runbook's name.
- **D3, yes, later PR:** fixed-choice parameters declared in the runbook file (`[remote.choices] env = ["test", "dev"]`). They are inside the approved hash and are never free text. A task names a choice and the PC checks it against the list.

### 3.3 Resumable runs (D2), PR 2

- A remote run that reaches a level-2 step **saves a checkpoint and pauses**: run id, runbook name, the approved hash it started under, the steps done, the requesting client, the task id, and the times. It lives in `%LOCALAPPDATA%\Glim\remote-runs.json`, with no output and no secret values.
- **On the owner's approval** (§8), the run **resumes at that step** without redoing earlier ones. Another level-2 step pauses again.
- **Refused to resume if the runbook's hash changed** since the run started (`resume-changed`). The run ends; nothing more runs.
- **No answer in 24 h:** it stops cleanly as `expired`. Nothing is undone, and steps already done stay done.
- The kill switch ends a paused run (`killed`). So does Stop on the phone (`declined`).
- **Status strings** Muse/cloud Glim can read: `running (step 2 of 5)`, `paused at step 3 of 5`, `resumed`, `done`, `failed at step 4 of 5 (run)`, `declined at step 3 of 5`, `expired at step 3 of 5`, `killed`, `resume-changed`.
- **New courier command `run-status`** (the seventh fixed command), `{"run": "<run id>"}`, or `{}` for the client's own recent runs. A client sees **only its own runs**. The answer carries name, status string and times, never output. The `runbook` command's result names the run id.
- A paused run counts as the one remote run at a time (§3.5). A second remote runbook task gets `runbook-busy` until the paused run ends.

### 3.4 What the result says (rules 1 and 4)

- The detail has a fixed shape, built only from codes and counts: `"<name>: OK (3/3 steps)"`, `"<name>: failed at step 2 of 3 (run)"` (the step's **kind**, never its text), and `"<name>: paused at step 3 of 5 (run <id>)"`.
- **Never in a result:** step output (it stays in `runbooks-output\`, redacted, for Settings), secret names or values, file contents, command lines, paths.
- `redact_detail` still runs over the detail as a second net.
- Only fixed codes go out (`runbook-not-remote`, `runbook-needs-phone`, `runbook-not-approved`, `runbook-changed`, `runbook-busy`, `runbook-rate-limited`, `runbook-failed-N-of-M` …). A `Refusal::message()` (English that can name steps) never goes into a result.

### 3.5 Limits, activity, logs (D4, D6)

- **D6:** defaults are **5 remote runbook tasks an hour per client**, **10 on the PC**, and **one remote run at a time** (a paused run counts). They are inside the existing 30/50 command caps.
- The limits can be edited in **Settings → Remote → Remote tasks** (per client 1-20, PC 1-50; one-at-a-time stays fixed). They are stored in settings.json under `remote`, so **only the PC can change them**. No courier command reads or writes them, and the command parser has no field for them (a test checks).
- **D4:** the Remote activity list shows **the runbook's name** with the outcome ("runbook glim-check: done"). It is the owner's own list, on the PC. glim.log keeps kinds and codes only, as today.
- `runbooks.log` gets `<name> remote(<client>) …` with the outcome code.
- The kill switch stops a remote run between steps (`killed` after the current step) and ends a paused run.

### 3.6 Runbooks that need care

- **D5: glim-update is never remote for now.** Its file must stay without `[remote]`. Also, whatever its file says, PR 1 refuses a runbook with a `launch_detached` step remotely (`runbook-not-remote`), because its result could not be delivered before Glim restarts.
- **relay-tokens** and anything that changes credentials or the relay itself: not recommended for remote. A remote run that rotates the relay's tokens could cut the line that carries its result.
- Recommended first remote runbooks: read-only or idempotent ones (a `check`-only health runbook, a test run with no secrets).

## 4. Threats and how the design answers them

| Threat | Answer |
|---|---|
| Relay or a stolen cloud token triggers a runbook | Tasks must verify with that client's signing key; a token alone can't sign. Only runbooks with a level; rate limits; kill switch. |
| A runbook is changed after approval so a remote run does something new | The approval hash (SHA-256 + HMAC, scripts included) is checked before running and again before every resume. `[remote]` and per-step `remote` are inside it. |
| A cloud task raises its own limits or level | Levels live in the approved file; limits only in Settings on the PC; no command carries either. |
| A cloud client approves its own level-2 step | Approval must verify under the phone's key, which only the PC holds the public half of. Cloud clients have Ed25519 task keys only, and no task path is accepted as an approval (§8). |
| A secret leaks through the result | Codes and counts only; output stays local; `redact_detail` as a second net. |
| A secret leaks through the task | The task carries only a name (later: a fixed choice). |
| A half-done run | Refused up front if any step is `never`; paused runs resume only under the same hash; expiry stops them cleanly. |
| A remote run cuts its own line (relay-tokens, glim-update) | glim-update never remote (D5, plus the `launch_detached` rule); relay-tokens not recommended. |

## 5. PR 1: levels 1 and 3, plus editable limits

- `runbooks/mod.rs`: parse `[remote] level` and per-step `remote`; work out the effective level; refuse `Trigger::Remote` for `never` (and any `launch_detached`) as `not-remote`, and for `phone` as `needs-phone` (until PR 3), before step 1. Card lines in 9 languages.
- `remote/exec.rs` and `Gate`: runbook rate limits from settings; `runbook-busy` for one at a time; detail with codes and counts; Remote activity carries the runbook name.
- Settings → Remote: two number fields for the limits. Settings → Runbooks: the remote tag.
- Docs: CLAUDE.md, PRIVACY.md (if needed), docs/remote-courier.md, PROJECT_STATUS.md, `examples/runbook.example.toml` (`[remote]` commented out).
- Tests: unmarked → `not-remote` remotely and still runs locally; `cloud` runs; a `never` or `phone` step refuses up front; toggling `[remote]` needs re-approval; `launch_detached` never remote; the limits come from settings and no command can set them; busy; the name is in activity but not in glim.log.

## 6. Out of scope

- Running a runbook on a schedule from the cloud.
- **Approving a runbook file remotely.** Approval stays a click on this PC, always. Level-2 phone approval OKs **one paused step of an already-approved runbook**, never a file.

## 7. Owner decisions (2026-11-20; replace the draft answers)

- **D1:** a per-runbook (or per-step) remote level in the runbook's own file: 1 cloud may run, 2 ask on my phone, 3 never remotely (default). Investigate the Phone channel and propose the phone-approval design first (§8), flagging anything that weakens "only my phone can approve".
- **D2:** resumable runs: checkpoint at a level-2 step, resume on approval without redoing steps, a 24 h expiry with a readable status, refuse resume on a hash change, and a courier command for run status.
- **D3:** yes to fixed-choice parameters, declared in the file and inside the hash, never free text. Later PR.
- **D4:** show the runbook name in Remote activity.
- **D5:** no glim-update from the cloud for now.
- **D6:** limits 5/h per client, 10/h on the PC, one at a time, editable in Settings → Remote, only on the PC.

## 8. Phone approval (level 2): owner answers 2026-11-21, BUILT (PR 3)

**Owner's answers (2026-11-21), which settle §8.3:**

1. **(a) A synced Face ID passkey.** Built so a hardware security key can be added later as another approver with no rebuild: up to 3 approvers, each its own credential and key (`kind` = `passkey` / `security-key`).
2. **The page is on GitHub Pages from this repo** (`approve/`, `.github/workflows/approve-pages.yml`, `https://anthonygarcia866.github.io/local-companion/`).
   - Owner-merge only: `.github/CODEOWNERS`, plus the rule in CLAUDE.md "Merges". It covers the page and the PC's verification code (`remote/approve.rs`, `relay/src/approvals.js`).
   - Match code: Pushover's text and the page both show the runbook, step, client and a random 6-character code. The page says not to approve if the code doesn't match. The passkey signs SHA-256 of the request id, run id, runbook, its approved hash, step, step kind, step count, client, expiry and code, and the PC rebuilds and checks every one of them.
   - **Other Pages sites on the same origin, checked 2026-11-21:** none. Of the account's 6 repos (5 private, 1 public), none has Pages enabled, and there is no `anthonygarcia866.github.io` user site. So nothing else can use the passkey today. CLAUDE.md now forbids enabling Pages on another repo of this account. If one is ever needed, move the page to its own domain first.
3. **No relay address in Pushover links.** The link carries the request id and the relay's host sealed to each approver's key. Request details are sealed (AES-256-GCM) to a key on the phone: the WebAuthn **PRF** output of the passkey (iOS 18+, reliable from 18.4, iCloud Keychain passkeys), via HKDF. No key is stored by the page: the phone re-derives it with Face ID each time. Pushover sees only the runbook name, step, client and match code.
- Enrolling and removing approvers: **only on this PC** (Settings → Remote). No courier command reaches it. Approvals are single-use, expire with the request (24 h), and the kill switch voids pending ones (and ends a pairing).

**As built:**

- **Pairing.** The PC opens a 10-minute relay slot and shows a QR code: `…/local-companion/#pair=<slot>&relay=<host>&pk=<the PC's one-time P-256 key>`. A fragment never leaves the phone.
  1. The page creates the passkey (ES256, discoverable, user verification required, PRF).
  2. It signs `SHA-256("glim-pair-v1|<slot>")` with Face ID and gets the PRF output.
  3. It derives the approver key: HKDF(PRF, info `glim-approve-v1 key`).
  4. It seals {credential id, SPKI public key, approver key, assertion} to the PC's key: ECDH P-256, HKDF with salt = page key ‖ PC key and info `glim-pair-v1|<slot>`, then AES-256-GCM.
  5. It posts the sealed bundle to `/v1/pair/<slot>`.
  6. The PC opens it and verifies the assertion (Face ID, origin, RP id, challenge). Both screens show the key's fingerprint, and the owner confirms on the PC.
  7. The approver goes into settings.json and its key into Credential Manager. One try per pairing: a bad answer ends it.
- **A request.** The PC sends the relay `{t:"approval", id, exp, body}` (body = details sealed to each approver) and sends Pushover the text plus the link `#r=<id>&h=<host sealed per approver>`.
  1. The page asks Face ID with PRF, opens the host, fetches `/v1/approve/<id>` and opens the details.
  2. It shows them with the match code.
  3. Approve asks Face ID again to sign the binding. Stop posts `{answer:"stop"}`.
  4. The relay keeps the single answer until the PC acknowledges it.
  5. The PC uses up the request (`take_approval`) whatever the answer, verifies, and resumes the run at the paused step under the same hash. Stop gives `declined`; anything that doesn't verify ends the run failed with its code.
- **What stays the same:** there is still no remote approval of a runbook *file*, and the level is still set only in the file.

### 8.0 Owner's original questions (kept for the record)

### 8.1 What the Phone channel can do today

- Settings → Phone sends **Pushover** notifications (`phone.rs`, owner pick 2026-10-18). It is **one-way, PC → phone**: one HTTPS POST to `api.pushover.net` with a fixed short text, `priority: 0`, no `url` field, and nothing ever comes back. Glim's rules (CLAUDE.md) limit the text to a fixed template built in Rust: a kind, an agent name from a fixed list, and a folder name.
- It has **no way to approve anything** and **no key on the phone that Glim knows**.
- Pushover itself offers emergency priority with an **acknowledge** button and a callback URL. **That does NOT meet the rule, and I recommend against it.** An acknowledgement is just Pushover's server saying "the user tapped", authenticated by the Pushover account login, not by a key on the phone. Pushover, anyone with the Pushover login, or anyone who can reach the callback URL could fake it. The PC could not tell.

So the phone needs **a new signing key that Glim pins**. Pushover stays only the doorbell.

### 8.2 Proposal: a passkey (WebAuthn) on the owner's iPhone, verified by the PC

- **Key:** a WebAuthn passkey (ES256), with user verification required (Face ID). Its **public key is pinned on the PC** at pairing. The private key never leaves the phone's secure hardware. The PC accepts an approval only if the signature verifies under that pinned key, the UV flag is set, and the signed challenge matches the PC's own pending request.
- **What is signed:** the challenge is `SHA-256("glim-approve-v1" | run id | request id | runbook | step number | step kind | client | created | expires | "run")`. The approval page **computes this challenge itself from the fields it displays**, so what the owner sees is what gets signed. The PC rebuilds the same bytes from its own checkpoint and compares. A request is good once (its id is burned) and only until `expires` (24 h).
- **Flow:**
  1. A remote run reaches a level-2 step. The PC saves the checkpoint (§3.3) and creates an approval request.
  2. The PC posts the request to the relay, which holds it like a task.
  3. The PC sends a Pushover ping: fixed text **"Glim: a remote runbook step is waiting for your OK"**, plus a `url` to the approval page. The URL carries only the request id and a fragment with the relay address. A fragment never leaves the phone's browser.
  4. The owner taps the link. The page fetches the request from the relay and shows runbook, step, requesting client and time. **Run** asks for Face ID and signs. **Stop** sends a decline.
  5. The page posts the assertion to the relay, and the relay hands it to the PC over the existing WebSocket.
  6. The PC verifies it and resumes the run. The approval page and the cloud see the outcome through `run-status`.
- **The PC opens no new connection.** It still talks only to the relay (existing WebSocket) and Pushover (existing). The relay gains two routes with no cloud token: `GET/POST /v1/approvals/<request id>`. The request id (128 random bits) is the only capability. That is harmless, because the assertion, not the id, is what authorizes.
- **A cloud client can never approve its own request.** Cloud clients hold only Ed25519 task keys. The PC accepts approvals only as WebAuthn assertions under the pinned phone key, and never through a task.
- **Pairing (PC only):** Settings → Phone → "Pair this phone for approvals". The PC shows a QR code with a one-time pairing id (10 min, single use). The phone opens the page and creates the passkey. The PC pins the public key. Both screens then show the key's **fingerprint (4 groups of 4)**, and the owner compares them before the PC marks it active. Removing it is a click on the PC only.

### 8.3 Things that weaken "only my phone can approve" (owner, please decide)

1. **iPhone passkeys sync through iCloud Keychain.** Apple doesn't allow a device-bound passkey on iPhone. The key would then live on **every device signed into the owner's Apple ID** (iPad, Mac), not only the phone. Options:
   - **(a)** Accept synced passkeys. This is the strongest option with no extra hardware: still Face ID/Touch ID, still the owner's Apple ID, never the cloud.
   - **(b)** Require a **hardware security key** (NFC YubiKey tapped on the phone). It is device-bound, and the PC can refuse synced credentials (the backup-eligible flag). It costs about $50 and the key must be at hand.
   - Recommendation: **(a) to start**, with the PC showing "synced passkey" in Settings so it is never a surprise. (b) can be added later as a second, stricter option.
2. **Where the approval page is served from.** WebAuthn binds the passkey to the page's origin, and the page's code decides what it shows and what it hashes.
   - If the **relay serves the page**, a compromised relay could show one runbook and sign another. The PC would still check the challenge against its own request, but the owner might approve something they didn't mean to.
   - **Recommendation:** serve the page as a **static file from GitHub Pages on this public repo**. It is reviewed in PRs, and the relay only carries bytes. A compromise then needs both the relay and the repo. The page origin would be the repo owner's `github.io` address, which is already public. The relay address goes only in the URL fragment, never in the repo.
   - Fallback if GitHub Pages is unwanted: the relay serves it, and the risk above is accepted.
3. **Pairing goes through the relay**, and a passkey's creation response can be made up by anyone who has the pairing id (iPhone passkeys give no attestation). That is why the fingerprint compare in §8.2 is required, not optional. It only works if the page isn't the relay's (point 2).
4. **New data at third parties.** Pushover gets the fixed text and a URL with a random request id and the relay's address in the fragment. Pushover can read the whole URL, fragment included, so **Pushover would learn the relay host** (the account's workers.dev subdomain, the business name). The relay would see runbook name, step and client for a pending request; tasks today are sealed, so it sees none of them. Options:
   - Leave the relay address out of the URL and let the page remember it from pairing (in the phone's browser storage).
   - Seal the request details to a phone key set up at pairing.
   - Recommendation: **both**. The page remembers the relay address from pairing, and the details are sealed to a phone-side key created at pairing (confidentiality only; authority stays with the passkey).
5. **Stop is not signed.** Anyone who has a request id could decline it, which is a denial of service only. The relay could drop requests anyway. Recommendation: accept this; a signed Stop adds a Face ID prompt for no real gain.
6. **This adds a network-facing part** (the approval page and two relay routes). The PR will call it out per CLAUDE.md, with tests that a wrong key, a missing UV flag, a changed challenge, a reused request, an expired request and a changed runbook hash are each refused.

**Nothing in §8 is built until the owner OKs it in chat**, including choices for points 1, 2 and 4.
