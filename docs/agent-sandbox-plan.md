# Agent sandbox: proposal (owner 2026-11-21, design item 1). Nothing has been created yet.

Goal: an agent client such as Muse, while it holds a lease, can start or message Claude Code sessions only as a separate low-privilege Windows user, `glim-agent`.
- **What that user can't reach:** your profile, your Credential Manager, your gh/wrangler/Supabase logins, other folders.
- **What it has instead:** its own GitHub credential, which can only push branches and open PRs on repos you choose.
- **Enforcement:** Windows ACLs and GitHub permissions, never prompt text.

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

It must fail on every path except `C:\GlimAgent`. I'll also list every folder on C:\ and other drives where `Users`, `Authenticated Users` or `Everyone` can write, and report them to you. Some are normal (`C:\Users\Public`, `ProgramData`); you decide on any others.

**How Glim starts a sandbox process:**
1. `CreateProcessWithLogonW(glim-agent, password from your vault, LOGON_WITH_PROFILE)`.
2. The environment block is built from glim-agent's own profile (`CreateEnvironmentBlock(…, bInherit=FALSE)`). None of your variables come along, so `CLOUDFLARE_API_TOKEN` and `SUPABASE_ACCESS_TOKEN` are not inherited. A test checks this.
3. The process runs in a job object that Glim kills when the lease ends, on the kill switch, or on "Lock now". `CreateProcessWithLogonW` doesn't put the new process in Glim's own job, so it is created suspended, assigned to the job (`AssignProcessToJobObject`), then resumed.
4. The working folder is always under `C:\GlimAgent\work`.

**Network:** the sandbox can reach the internet. Claude Code needs api.anthropic.com and git needs github.com. Per-user outbound filtering (`New-NetFirewallRule -LocalUser`) may work on this Windows edition; I'd test it before relying on it. Until then, the limit is what it holds: only its own credentials.

## 3. Its Claude login

Sessions run `claude` as glim-agent, so it needs its own login in `C:\Users\glim-agent\.claude`. Glim would add "Sign the sandbox in to Claude", which opens a console as glim-agent for `claude /login`.

**Your call:** which Claude account it signs into.
- **A separate account or seat:** nothing of yours is reachable.
- **Your own account:** its token in the sandbox can reach your claude.ai chats.

I recommend a separate one.

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

## 5. What you'd decide

1. Create `glim-agent` as above. **Yes / no / changes**
2. Which Claude account the sandbox signs into. **Separate (recommended) / yours**
3. The GitHub machine account plus forks. **Yes / App instead**
   - Which repos? local-companion only to start?
4. Turn on "Require approval for all outside collaborators" for Actions on those repos. **Yes / no**

Until (b) is approved and built, an agent client's session commands stay refused even with a lease (`no-sandbox`). Runbooks it starts still run as you, as every remote runbook does: only those whose file allows it, each `phone` step waiting for your Face ID.
