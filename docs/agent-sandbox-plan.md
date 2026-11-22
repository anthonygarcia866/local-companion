# Agent sandbox: plan (owner 2026-11-21, design item 1; decided 2026-11-22, §6). Being built in small PRs (§7); nothing on this PC is created until the owner runs the step shown to them.

Goal: an agent client such as Muse, while it holds a lease, can start or message Claude Code sessions only as a separate low-privilege Windows user, `glim-agent`.
- **What that user can't reach:** your profile, your Credential Manager, your gh/wrangler/Supabase logins, other folders.
- **What it has instead:** its own GitHub credential, which can only push branches and open PRs on repos you choose.
- **Enforcement:** Windows ACLs, a per-user network limit and GitHub permissions, never prompt text.

## 0. Threat model: prompt injection first (owner 2026-11-21)

Muse reads email, DMs and the web, so anyone who can put text in front of it can try to steer it. Plan for the worst case: **a leased agent is fully controlled by an attacker.** Everything below must hold even then, with a lease open:

- **The lease is not a permission to act as you.** It only widens which of the seven commands the agent may send. Session commands still run only as `glim-agent` (sections 1-4); `no-sandbox` until that exists.
- **Face ID gates stay per action, never per lease.** Merging to main, deploying, writing a secret, changing a file outside the sandbox, and every runbook `phone` step each need their own passkey signature over that exact action. No lease, setting or agent message turns one off or batches it. (Design item (c), built after this plan.)
- **What the phone shows comes from the PC**, not from the agent. Anything the agent wrote (a PR title, a branch name, a reason) is shown as quoted data, clearly labelled, never as the action's description. The match code is the PC's.
- **The agent can ask, not grant.** An injected Muse can send `lease-request` (capped per hour, one open at a time); only you can approve. Approve only a lease you started yourself; a request you didn't expect is a sign Muse was steered.
- **Limits hold under a lease:** rate limits, the kill switch, Lock now, the network limit (section 2) and the GitHub rules (section 5).
- **Nothing the agent can reach holds your credentials:** separate Windows user, its own empty Credential Manager, its own GitHub identity, its own Claude login (recommended).

## 1. Windows user

Run once, as you (elevated), from a script I'd show you first. Nothing is created until you say so.

```powershell
# A 32-character random password. Nobody types it; it goes straight into YOUR
# Credential Manager as Glim/AgentUserPassword, never on screen or disk.
New-LocalUser -Name glim-agent -Password $pw -PasswordNeverExpires -UserMayNotChangePassword -AccountNeverExpires -Description "Glim agent sandbox"
# Member of Users only: never Administrators or Remote Desktop Users.
# Hidden from the sign-in screen:
New-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList' -Name glim-agent -Value 0 -PropertyType DWord
# User rights (secedit): deny "Log on through Remote Desktop Services" and "Access this computer from the network".
```

"Deny log on locally" can't be set: `CreateProcessWithLogonW` (below) needs a local logon. The password is random and lives only in your vault, and the account is hidden from sign-in, so nobody can sign in as it.

## 2. Its workspace and what it can't read

```powershell
New-Item -ItemType Directory C:\GlimAgent\work
# SIDs, not names (SYSTEM / Administrators are localized on non-English Windows):
icacls C:\GlimAgent /inheritance:r /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" "${env:USERNAME}:(OI)(CI)F" "glim-agent:(OI)(CI)M"
# Belt and braces on top of the default profile ACL (you/SYSTEM/Administrators only):
icacls $env:USERPROFILE /deny "glim-agent:(OI)(CI)F"
```

What it can't reach:
- **Credential Manager** is per user, so it gets its own empty vault.
- **gh, wrangler, Supabase and Claude logins** live in your profile, which the deny ACE closes.
- **G: (Google Drive)** is mounted per user, so it doesn't see it.

Before any session runs, a check script (also shown to you first) proves it. As glim-agent it tries to:
- read `%USERPROFILE%\.config\gh`, `%APPDATA%\xdg.config\.wrangler` and `~\.claude`
- list Credential Manager
- read every drive root

It must fail on every path except `C:\GlimAgent`.

> **Amended 2026-11-22 (while writing `check.ps1`):** "every path except `C:\GlimAgent`" can't hold for drive roots: any Windows user may list `C:\`, and folders anyone can read stay readable (that's what the writable-folder list below is for). So `check.ps1` FAILS on the owner's profile and every login/vault folder named above (plus `%LOCALAPPDATA%\Glim`, `%APPDATA%\Glim`, `~\.claude2`, G:) and the login files in them, and on any of `CLOUDFLARE_API_TOKEN`, `SUPABASE_ACCESS_TOKEN`, `ANTHROPIC_API_KEY`, `GH_TOKEN`, `GITHUB_TOKEN` set machine-wide.
>
> **Amended again 2026-11-22 (security review):** the proofs are NOT made by running as glim-agent. A process running as glim-agent starts from glim-agent's own profile and environment, which glim-agent writes (a .NET profiler DLL via `COR_PROFILER*`, a planted module), so it could report anything. `check.ps1` computes glim-agent's effective access on the owner's side (Windows AuthZ, from its SID and each object's DACL) and reads its groups from the account database. The owner also decided (2026-11-22) to **lock what all users can write**: setup denies glim-agent write + delete on C:\tests, C:\glimtest, C:\c, C:\Drivers, every top-level folder and PATH folder that all users can write, and each drive root (that folder only); `check.ps1` fails if glim-agent can write any of them or any PATH folder.

I'll also list every folder on C:\ and other drives where `Users`, `Authenticated Users` or `Everyone` can write, and report them to you. Some are normal (`C:\Users\Public`, `ProgramData`); you decide on any others.

**How Glim starts a sandbox process:**
1. `CreateProcessWithLogonW(glim-agent, password from your vault, LOGON_WITH_PROFILE)`.
2. The environment block is built from glim-agent's own profile (`CreateEnvironmentBlock(…, bInherit=FALSE)`). None of your variables come along, so `CLOUDFLARE_API_TOKEN` and `SUPABASE_ACCESS_TOKEN` are not inherited. A test checks this.
3. The process runs in a job object that Glim kills when the lease ends, on the kill switch, or on "Lock now". `CreateProcessWithLogonW` doesn't put the new process in Glim's own job, so it is created suspended, assigned to the job (`AssignProcessToJobObject`), then resumed.
4. The working folder is always under `C:\GlimAgent\work`.

**Network: GitHub only (owner 2026-11-21).** glim-agent's processes may reach GitHub and nothing else.
- **Allowed hosts:** `github.com`, `api.github.com`, `codeload.github.com`, `objects.githubusercontent.com` (git over https on 443).
- **Plus `api.anthropic.com`** (owner 2026-11-22, §6 e): Claude Code must reach it to think. Nothing else.
- **How, by hostname:** a small local allowlisting proxy (HTTPS CONNECT only, checks the host name against the list above, no TLS interception) listens on 127.0.0.1. glim-agent's environment points `HTTPS_PROXY` at it.
- **Enforced by the firewall, not by the env var:** a Windows Firewall outbound rule blocks every connection from glim-agent's SID except to 127.0.0.1 on the proxy port. A process that ignores `HTTPS_PROXY` then simply can't connect. (Firewall rules match IPs, and GitHub's change, so the hostname check lives in the proxy.)
- **Verify before relying on it** (CLAUDE.md: enforcement is checked by trying): as glim-agent, `curl.exe https://example.com` direct and through the proxy must both fail; `git ls-remote https://github.com/<fork>` must work. Per-user outbound rules (`-LocalUser`) must be proven on this Windows edition first. If they don't hold, sandbox sessions stay off (`no-sandbox`) until another enforcement works.
- **No sandbox network to:** your relay, Pushover, Cloudflare, Supabase, Stripe, email, or anything on this PC but the proxy.

## 3. Its Claude login

**Decided (owner 2026-11-22, §6 b): a dedicated Anthropic API key, not a Claude login.** Not the owner's Claude or Claude2 login: a sandbox holding Claude2's login could reach account-2's cloud environment, where that account's courier key lives.

- The owner creates it in the Anthropic Console (console.anthropic.com): a workspace of its own (e.g. `glim-agent`) with a **monthly spend limit of $50**, and one API key in that workspace. The key can spend only within that workspace's limit.
- It is stored **only for glim-agent**: in glim-agent's own Credential Manager, written by a console started as glim-agent into which the owner pastes it (never chat, never the owner's vault, never a file). Claude Code in the sandbox gets it as `ANTHROPIC_API_KEY` from a small launcher that reads glim-agent's vault; it never reaches the owner's environment.
- If it leaks, the damage is that workspace's $50/month; the owner revokes it in the Console.

## 4. Its GitHub credential

The constraint: a fine-grained token on YOUR account acts as you, so any rule you can bypass, it can bypass too, pushing to main included. So it must be a different GitHub identity.

**Recommended: a machine account plus forks.**
1. You create one GitHub machine account, for example `<you>-glim-agent`. GitHub allows one free machine account per person.
2. It forks each repo you choose. For a private repo, you first add it as a collaborator with the **Read** role so it can fork.
3. Its credential is a fine-grained PAT owned by the machine account:
   - repository access: only its forks
   - permissions: Contents read/write, Pull requests read/write, Metadata read
   - expiry: 30 days
   - stored only in glim-agent's own Credential Manager, through `git credential-manager`, by a Face ID runbook, never in chat
4. It can push any branch to its fork, but nothing to your repos. Its PRs go to your repo from the fork.

Why forks:
- Workflows from fork PRs get **no secrets**.
- With "Require approval for all outside collaborators" (Settings → Actions → General, which I'd turn on for each chosen repo), they don't even run until you approve.
- That holds only for workflows triggered by `pull_request`. Any `pull_request_target` or `workflow_run` workflow runs with your secrets on fork PRs, so each chosen repo is audited for those first.
- For private repos, "Send secrets and variables to workflows from fork pull requests" must stay off.
- A same-repo branch push would run `push` workflows **with** your repo secrets. That is the hole this closes.

**To verify before relying on it:** whether a fine-grained PAT owned by the machine account can open the PR on your repo. GitHub's support for cross-owner fine-grained PRs is limited. If it can't:
- **Fallback A:** you open the PR from its branch with one Face ID runbook.
- **Fallback B:** a classic PAT with only `public_repo` for public repos. That is broader, and I'd only accept it for a machine account that owns nothing else.

**Alternative: a GitHub App** installed on the chosen repos (contents and pull_requests write).
- Its tokens can be refused on main by a ruleset with no bypass.
- But same-repo branches still get secrets in `push` workflows.
- And Glim would have to mint its 1-hour tokens with the App's private key, which means a new network call (api.github.com) through Glim's choke point.

Not recommended.

## 5. Main is protected by GitHub, not just by token scope (owner 2026-11-21)

A token's scope can be wrong, widened later or leaked. So every repo the agent works on also has a **branch protection rule (or ruleset) on main**:
- a pull request is required to change main (no direct pushes, by anyone, admins included);
- no force pushes, no deleting main;
- the repo's CI checks must pass before merge.
No review approval is required (you're the only reviewer, and GitHub won't let you approve your own PR); merges still follow CLAUDE.md's merge rules and, for the agent, design item (c): merging is a Face ID action.

**What this account's GitHub plan allows (checked 2026-11-21, read-only API calls):**
- The account is on GitHub **Free**. For a private repo, both branch protection and rulesets answer `403 Upgrade to GitHub Pro or make this repository public to enable this feature` (tried on dale-cuts). So **none of the 5 private repos can have main protected** on this plan.
- local-companion is public, so it can. **Done 2026-11-22 (owner §6 f):** ruleset `main: PRs only, no force-push, no deletion` (id 27855361), active, no bypass list, target the default branch: deletion and force pushes blocked, a pull request required (0 approvals, merge commits only). `GET /rules/branches/main` lists all three. Required status checks were left out on purpose: Windows CI runs only on `windows/**`/`relay/**` changes, so a docs-only PR would wait forever for it. Changing or removing the ruleset later is a Face ID runbook (§7), never a plain `gh api` call.

**So, until you decide otherwise:**
- **Muse works on public repos only** (local-companion to start). No private repo is forked, shared with the machine account, or reachable from the sandbox.
- Before the sandbox's first session, local-companion's main gets the protection above. You turn it on in GitHub (Settings → Rules → Rulesets → New branch ruleset, target `main`: Restrict deletions, Require a pull request before merging with 0 approvals, Require status checks to pass (Windows CI, Docs check), Block force pushes, no bypass list), or approve a Face ID runbook that sets it through `gh api`. Then I check it holds: a direct push to main must be refused.
- **Private repos later need one of:** GitHub Pro on this account (paid), making the repo public, or moving it to an organization on a plan that protects private branches. Your call; nothing changes until then.

## 6. Decisions (owner 2026-11-22)

a. **Create `glim-agent`** as in §1-2. Claude shows the exact steps first and says when the owner's Windows password (UAC) is needed.
b. **Dedicated Anthropic API key** with a $50/month workspace spend limit, stored only for glim-agent (§3).
c. **GitHub machine account plus forks** (§4). It never gets write access to the owner's repos; its PRs reach main only through the Face ID merge runbook.
d. **Actions on its PRs:** run without asking unless the PR touches `.github/`; those need a Face ID runbook. Checked 2026-11-22: every local-companion workflow is triggered by `pull_request`, `push`, `schedule`, `workflow_dispatch` or a tag, none by `pull_request_target` or `workflow_run`, and the one secret (`GLIM_VOICE_API_KEY`) is passed only on `schedule`/`workflow_dispatch` runs, so fork PRs never see it. How (GitHub has no "auto-approve unless a path" setting): the repo keeps "Require approval for all outside collaborators", and Glim approves a waiting fork run itself (`POST /actions/runs/<id>/approve`, through the owner's gh, from a runbook) only when the PR's changed files include nothing under `.github/`; otherwise it asks Face ID with the file list.
e. **Network:** GitHub + `api.anthropic.com` only (§2).
f. **Main ruleset:** set 2026-11-22 (§5). Changing or removing it needs a Face ID runbook.
g. **Private repos stay out** for now.

Also decided: **no Pushover.** Muse forwards approval links; until Muse is connected, pending approvals are shown in Settings → Remote (§7, PR 1).

## 7. Build order (small PRs; security-critical rule)

Each PR: CI green, a fresh-agent security review with no critical/high, merge commit. Owner steps are shown in full before they're needed.

1. **Pending approvals in Settings → Remote** (built 2026-11-22, branch `remote-pending-approvals`, `remote/pending.rs`). A card listing each paused run's and each lease request's approval as a QR code of its link plus the match code, so the owner can approve without Pushover. Nothing new leaves the PC.
2. **Agent user setup script** (built 2026-11-22, branch `agent-sandbox-setup`; `scripts/agent-sandbox/`: `setup.ps1 [-DryRun] [-RotatePassword]`, `check.ps1`, `remove.ps1 [-DryRun]`, `cred.ps1`, `probe.ps1`; run by the owner, elevated): §1 user (random password into the owner's vault), §2 folder ACLs, sign-in hiding, the logon-right denials. Plus `check.ps1` (§2's proofs). **Owner: one UAC prompt (Windows password).**
3. **Network limit:** the allowlisting proxy (GitHub hosts + `api.anthropic.com`, CONNECT only) and the per-user firewall rule, with the empirical checks (`example.com` refused direct and via proxy; GitHub and the API reachable). **Owner: one UAC prompt** for the firewall rule.
4. **API key for glim-agent:** the launcher that reads glim-agent's vault. **Owner: create the workspace + $50 limit + key in the Console, then paste the key into the console window opened as glim-agent.**
5. **Machine account + fork + PAT:** **Owner: sign up the machine account** (its own email; GitHub allows one free machine account per person), then create its fine-grained PAT (only its forks; Contents + Pull requests read/write; 30 days) and paste it into the glim-agent console. Then the cross-owner PR check (§4).
6. **Sandbox sessions:** `start-session`/`send-to-session` for a leased agent run as glim-agent in `C:\GlimAgent\work` (CreateProcessWithLogonW, own environment, job object killed with the lease); `no-sandbox` goes away only after 2-5 are verified.
7. **Face ID runbooks:** merge a fork PR; approve a fork PR's Actions run when it touches `.github/` (and the no-ask approval otherwise, d); change or remove the main ruleset.

Until 6 is merged and verified, an agent client's session commands stay refused even with a lease (`no-sandbox`). Runbooks it starts still run as the owner, as every remote runbook does: only approved ones whose file gives them a remote level, each `phone` step waiting for Face ID.
