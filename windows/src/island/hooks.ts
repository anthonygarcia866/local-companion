// Hook events from Claude Code and every other agent → island state.
// Port of HookServer.processEvent / processPermissionRequest from the macOS app.
// Difference from macOS: no terminal filter. On Windows the hook fires from any
// terminal (Windows Terminal, VS Code, PowerShell…) and all of them are handled.
// The relay has already mapped every agent's events and fields onto Claude
// Code's (hook/src/normalize.rs), so one handler serves them all.

import { Bridge, onEvent } from "../core/bridge";
import { buildFileDiff, fileName, makeDiffStep, toOneLine } from "../core/diff";
import { State, type AskedQuestion } from "../core/state";
import { SESSION_PILL_SEP, pillDefinition } from "../core/pills";
import { APPROVAL_AGENTS, agentColor, agentName, validateAgent } from "./agents";
import type { Island } from "./island";
import { parseClaudePlan, restorePlanUsage } from "../core/plan";
import { isLimitNotice, limitKind, parseLimit, type AgentLimit } from "../core/limit";
import { phone, phoneSettings, projectOf, type Ping, type PhoneSettings } from "../core/phone";
import { setClaudePlanUsage, storedClaudePlanUsage } from "../views/usage";
import { N_, t } from "../i18n/i18n";

const CLAUDE_ID = "integration_claude";
const CURSOR_ID = "agent_cursor";

/**
 * One per request, on screen or queued, from the moment it arrived: the relay
 * waits 108 s for Glim, so after 110 s its card (or its place in the queue)
 * would be lying.
 */
const requestTimers = new Map<string, number>();

function clearRequestTimer(requestId: string) {
  const timer = requestTimers.get(requestId);
  if (timer != null) window.clearTimeout(timer);
  requestTimers.delete(requestId);
}

function armRequestTimer(island: Island, requestId: string) {
  clearRequestTimer(requestId);
  requestTimers.set(requestId, window.setTimeout(() => {
    requestTimers.delete(requestId);
    if (State.pendingApproval?.requestId === requestId) dropPendingCard(island);
    else State.withdrawQueued((r) => r.requestId === requestId);
  }, 110_000));
}

/**
 * Takes the approval or question card down and gives the island back, or,
 * when a request was queued behind it, brings that one's card up.
 */
function dropPendingCard(island: Island): void {
  const req = State.pendingApproval;
  if (!req) return;
  clearRequestTimer(req.requestId);
  State.endApproval();
  island.dropPin();
  const next = State.pendingApproval;
  if (next) {
    island.alert(next.questions ? "question" : "approval");
  } else if (State.view === "approval" || State.view === "question") {
    island.setView(State.defaultView());
  }
  State.notify();
}

/**
 * Phone notifications (docs/phone-plan.md): with the feature on, the idle
 * time comes from Rust ("only when I'm away"), the watcher decides, and a
 * kind, the pill and its project go to Rust.
 */
function pingPhone(decide: (s: PhoneSettings, idleMs: number | null) => Ping | null) {
  const s = phoneSettings(State.settings.phone);
  if (!s.enabled) return;
  void Bridge.phoneIdleMs().then((idleMs) => {
    const p = decide(s, idleMs);
    if (p) void Bridge.phoneNotify(p.kind, p.pillId, p.project);
  });
}

/** The return to idle that Stop arms, per pill, so the next turn can cancel it. */
const stopTimers = new Map<string, number>();

function cancelStopTimer(id: string): boolean {
  const timer = stopTimers.get(id);
  if (timer == null) return false;
  window.clearTimeout(timer);
  stopTimers.delete(id);
  return true;
}

/** Events after which a pending permission request of the same session is moot. */
const TURN_OVER = new Set(["Stop", "StopFailure", "UserPromptSubmit", "SessionEnd", "Interrupt"]);

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  message?: string;
  /** UserPromptSubmit carries `prompt`; `message` belongs to Notification/Stop. */
  prompt?: string;
  /** Stop: the turn's final answer (Markdown) — Claude Code's, Hermes's, Codex's. */
  last_assistant_message?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
  /** Set by the relay when an edit was too big to forward whole (> 256 KB). */
  coucou_diff_truncated?: boolean;
  /** Optional agent tag: lowercase, digits and hyphens, ≤ 24 chars. */
  coucou_agent?: string;
  /** Hermes: where the session runs (telegram, discord…; "cli" in a terminal). */
  platform?: string;
  /** "cursor" when Claude Code runs in Cursor's terminal (set by the relay). */
  term_editor?: string;
  /** StopFailure: why the turn stopped ("rate_limit", "billing_error", "server_error"…). */
  error?: string;
  /** StopFailure: more about the error, as Claude Code wrote it. */
  error_details?: string;
  /** StatusLine (the plan usage relay): Claude Code's 5-hour and weekly limits. */
  rate_limits?: unknown;
}

const PROJECT_ALIASES: Record<string, string> = {
  "notch-buddy": "Notch Buddy",
  notchbuddy: "Notch Buddy",
  notch_buddy: "Notch Buddy",
};

function aliasProjectName(name: string): string {
  return PROJECT_ALIASES[name.toLowerCase()] ?? name;
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

/**
 * localizedStep() — same labels as the macOS app, in English and shown in the
 * interface language (src/i18n) when the step is recorded.
 */
const TOOL_LABELS: Record<string, string> = {
  Bash: N_("Runs"),
  Read: N_("Reads"),
  Write: N_("Writes"),
  Edit: N_("Edits"),
  Glob: N_("Searches"),
  Grep: N_("Searches"),
  WebSearch: N_("Searches the web"),
  WebFetch: N_("Fetches"),
  TodoWrite: N_("Tasks"),
  Task: N_("Agent"),
  LS: N_("Lists"),
  MultiEdit: N_("Edits"),
  NotebookEdit: N_("Notebook"),
  PowerShell: N_("Runs"),
  // Antigravity's tools (#298).
  run_command: N_("Runs"),
  view_file: N_("Reads"),
  write_to_file: N_("Writes"),
  replace_file_content: N_("Edits"),
  read_url_content: N_("Fetches"),
  search_web: N_("Searches the web"),
};

function stepLabel(tool: string, input: Record<string, unknown>): string {
  const label = TOOL_LABELS[tool] ? t(TOOL_LABELS[tool]) : tool;
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  const cmd = str("command");
  if (cmd) return `${label} · ${cmd.slice(0, 40)}`;
  const path = str("path");
  if (path) return `${label} · ${lastPathComponent(path)}`;
  const file = str("file_path");
  if (file) return `${label} · ${lastPathComponent(file)}`;
  const query = str("query");
  if (query) return `${label} · ${query.slice(0, 40)}`;
  return label;
}

/**
 * What the Allow button actually authorises. Approving "Write" tells you nothing
 * — approving `Write · C:\…\.env` tells you everything, and the difference is
 * the whole point of approving from the island rather than blind.
 *
 * Ordered by how specific the field is, so an unfamiliar tool still shows
 * whatever identifying string it carries instead of falling back to its name.
 */
const APPROVAL_FIELDS = [
  "command", // Bash, PowerShell
  "file_path", // Write, Edit, MultiEdit, NotebookEdit
  "path", // Read, LS
  "url", // WebFetch
  "query", // WebSearch
  "pattern", // Glob, Grep
  "prompt", // Task
] as const;

function approvalTarget(tool: string, input: Record<string, unknown>): string {
  for (const field of APPROVAL_FIELDS) {
    const value = input[field];
    if (typeof value === "string" && value.trim()) {
      return `${tool} · ${value.trim()}`;
    }
  }
  return tool;
}

/**
 * The questions of an AskUserQuestion call, if the island can show all of them
 * as options to pick from. Anything it cannot is left to the terminal.
 */
function askedQuestions(tool: string, input: Record<string, unknown>): AskedQuestion[] | null {
  if (tool !== "AskUserQuestion" || !Array.isArray(input.questions)) return null;
  const out: AskedQuestion[] = [];
  for (const raw of input.questions as Record<string, unknown>[]) {
    const question = typeof raw?.question === "string" ? raw.question : "";
    const options = (Array.isArray(raw?.options) ? (raw.options as Record<string, unknown>[]) : [])
      .filter((o) => typeof o?.label === "string" && o.label)
      .map((o) => ({
        label: o.label as string,
        description: typeof o.description === "string" ? o.description : "",
      }));
    // A question cut short by the relay would be answered under the wrong text.
    if (!question || question.endsWith("…") || options.length < 2) return null;
    out.push({ question, options, multiSelect: raw.multiSelect === true });
  }
  return out.length > 0 ? out : null;
}

/** A session that sent nothing for this long no longer holds its pill. */
const SESSION_QUIET_MS = 30 * 60_000;

/**
 * The pill a Claude Code session's events go to. The first session holds the
 * VS Code / Cursor pill; one that runs at the same time gets a pill of its own
 * (`<pill>~<session>`), so two sessions never overwrite each other's state,
 * steps or session id. The pill is free again once its session ends, or after
 * 30 minutes of silence (a terminal closed without SessionEnd). Events with no
 * session id stay on the pill itself.
 */
function sessionPill(pillId: string, sessionId: string): string {
  if (!sessionId) return pillId;
  const own = State.tasks.find((t) => t.id === pillId);
  const quiet = (t: { lastEventAt?: number; state: string }) =>
    Date.now() - (t.lastEventAt ?? 0) > SESSION_QUIET_MS && t.state !== "approval" && t.state !== "question";
  if (!own || !own.sessionId || own.sessionId === sessionId || quiet(own)) return pillId;
  const prefix = `${pillId}${SESSION_PILL_SEP}`;
  const extra = State.tasks.find((t) => t.id.startsWith(prefix) && t.sessionId === sessionId);
  if (extra) return extra.id;
  return prefix + sessionId.replace(/[^A-Za-z0-9-]/g, "").slice(0, 12);
}

/** "repo", then "repo · 2", "repo · 3" for the sessions after the first. */
function sessionPillName(id: string, projectName: string): string {
  if (!id.includes(SESSION_PILL_SEP)) return projectName;
  const base = id.slice(0, id.indexOf(SESSION_PILL_SEP));
  const others = State.tasks.filter((t) => t.id !== id && (t.id === base || t.id.startsWith(base + SESSION_PILL_SEP)));
  const mine = State.tasks.find((t) => t.id === id);
  if (mine) return mine.name;
  return `${projectName} · ${others.length + 1}`;
}

/** The Claude Code session's pill, named after its project for the session. */
function upsert(id: string, projectName: string, cwd: string, sessionId: string) {
  const t = State.upsertWorkspacePill(id, sessionPillName(id, projectName), cwd);
  if (!t) return;
  if (sessionId) t.sessionId = sessionId;
  t.lastEventAt = Date.now();
}

/** The session is over: the pill goes back as it was, or away if it was only there for it. */
function clearSession(id: string) {
  const t = State.tasks.find((x) => x.id === id);
  if (!t) return;
  if (!State.isKept(id)) {
    State.removeTask(id);
    return;
  }
  t.state = "idle";
  t.steps = [];
  t.stepIndex = 0;
  delete t.stepSeq;
  t.name = pillDefinition(id)?.name ?? t.name;
  t.pillBadge = null;
  t.sessionId = null;
  t.finalLine = null;
  t.limit = null;
}

/** The limit that stopped a pill's agent, for its card (state "ratelimit"). */
function setLimit(id: string, limit: AgentLimit) {
  const t = State.tasks.find((x) => x.id === id);
  if (t) t.limit = limit;
}

/** The final message stays on the card until the next turn starts. */
function clearFinalLine(id: string) {
  const t = State.tasks.find((x) => x.id === id);
  if (t) t.finalLine = null;
}

/**
 * PostToolUse of Edit / MultiEdit / Write → a diff stored for the pill and a
 * ticker step with its +N −M counts. Nothing is kept for a pill that does not
 * exist, so a stray event cannot grow memory. An edit the relay had to cut
 * would give wrong counts: the PreToolUse step ("Edits · file") stands alone.
 */
function recordDiff(agentId: string, payload: HookPayload) {
  if (payload.coucou_diff_truncated) return;
  if (!State.tasks.some((t) => t.id === agentId)) return;
  const diff = buildFileDiff(payload.tool_name ?? "", payload.tool_input ?? {});
  if (!diff) return;
  const id = State.appendSessionDiff(agentId, diff);
  State.appendStep(agentId, makeDiffStep(fileName(diff.path), diff.added, diff.removed, id));
}

export function registerHookHandlers(island: Island) {
  // The last plan numbers seen survive a restart, as on the Mac.
  State.planUsage ??= restorePlanUsage(storedClaudePlanUsage());
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  // Account-wide numbers from the status line relay, not part of any session:
  // keep the latest, nothing else (no reveal, no sound), paused or not.
  if (payload.hook_event_name === "StatusLine") {
    const usage = parseClaudePlan(payload.rate_limits);
    if (usage) setClaudePlanUsage(usage);
    return;
  }

  // Pause stops the writing features only (1d, §14.6): agents carry on, and a
  // permission request that comes while paused shows here as usual.

  const name = payload.hook_event_name ?? "";
  const cwd = payload.cwd ?? "";
  const raw = lastPathComponent(cwd);
  const projectName = aliasProjectName(raw || "Session");

  // Route to the right pill. Valid coucou_agent → dynamic "agent_<name>" pill.
  // "claude" is reserved; absent or invalid → Claude Code's own pill: Cursor's
  // when it runs in Cursor's terminal (Mac #120), VS Code's otherwise.
  const validAgent = validateAgent(payload.coucou_agent);
  const workspaceId = payload.term_editor === "cursor" ? CURSOR_ID : CLAUDE_ID;
  const agentId = validAgent ? `agent_${validAgent}` : sessionPill(workspaceId, payload.session_id ?? "");
  const isExternalAgent = validAgent !== null;
  const sessionId = payload.session_id ?? "";

  /** Alerts force the island open; work events only reveal the compact island. */
  const surface = (view: Parameters<Island["alert"]>[0], isAlert: boolean) => {
    if (State.mode === "expanded") {
      if (isAlert) island.setView(view);
    } else if (isAlert) {
      island.alert(view);
    } else if (State.mode === "hidden") {
      island.reveal();
    }
  };

  /** Ensure the agent pill exists (no-op for Claude Code). */
  const ensurePill = () => {
    if (isExternalAgent) {
      State.upsertExternalAgent(agentId, agentName(validAgent!), agentColor(validAgent!));
      const t = State.tasks.find((x) => x.id === agentId);
      if (t && cwd) t.sessionCwd = cwd;
      if (t && sessionId) t.sessionId = sessionId;
    } else {
      upsert(agentId, projectName, cwd, sessionId);
    }
  };

  /**
   * Called where a handler is about to replace `finished` with a newer state:
   * the timer Stop armed would otherwise put the pill back to idle over it. The
   * badge that timer was going to clear goes now.
   */
  const supersedeStop = () => {
    if (cancelStopTimer(agentId)) State.setPillBadge(agentId, null);
  };

  // The turn that asked for a permission is over — answered in the terminal,
  // interrupted, or a new prompt — so the card would be lying. It goes, and the
  // relay is released without a decision. Same rule as the Mac.
  // The same goes for a request still waiting in the queue.
  if (TURN_OVER.has(name)) {
    const sameTurn = (r: { pillId: string; sessionId: string }) =>
      r.pillId === agentId && r.sessionId === (payload.session_id ?? "");
    for (const gone of State.withdrawQueued(sameTurn)) {
      clearRequestTimer(gone.requestId);
      if (gone.requestId) void Bridge.approvalDecline(gone.requestId);
    }
    const pending = State.pendingApproval;
    if (pending && sameTurn(pending)) {
      if (pending.requestId) void Bridge.approvalDecline(pending.requestId);
      dropPendingCard(island);
    }
  }

  // Read after the card above is dropped: its pill may have handed the front
  // back to the pill you were on.
  const focused = State.focusId === agentId;

  switch (name) {
    case "SessionStart":
      ensurePill();
      clearFinalLine(agentId);
      // Hermes through its gateway says where the session comes from.
      if (validAgent === "hermes" && payload.platform && payload.platform !== "cli") {
        State.appendStep(agentId, payload.platform.charAt(0).toUpperCase() + payload.platform.slice(1));
      }
      surface("overview", false);
      break;

    case "UserPromptSubmit": {
      ensurePill();
      supersedeStop();
      clearFinalLine(agentId);
      State.updateTask(agentId, "thinking");
      // The field is `prompt`; reading `message` meant this step was always blank.
      const asked = payload.prompt ?? payload.message;
      if (asked) State.appendStep(agentId, asked.slice(0, 60));
      surface("overview", false);
      break;
    }

    case "PreToolUse": {
      ensurePill();
      supersedeStop();
      clearFinalLine(agentId);
      State.updateTask(agentId, "working");
      const tool = payload.tool_name ?? "Tool";
      State.appendStep(agentId, stepLabel(tool, payload.tool_input ?? {}));
      surface("overview", false);
      break;
    }

    case "PostToolUse":
      supersedeStop();
      // The question was answered in the terminal: the card would be lying.
      if (
        payload.tool_name === "AskUserQuestion" &&
        State.pendingApproval?.questions &&
        State.pendingApproval.sessionId === (payload.session_id ?? "")
      ) {
        dropPendingCard(island);
      }
      State.updateTask(agentId, "working");
      recordDiff(agentId, payload);
      break;

    case "PostToolUseFailure":
      supersedeStop();
      State.updateTask(agentId, "working");
      State.appendStep(agentId, t("⚠ failed"));
      break;

    case "Notification": {
      const message = payload.message ?? "";
      if (isLimitNotice(message)) {
        supersedeStop();
        const limit = parseLimit("usage", [message], Date.now());
        setLimit(agentId, limit);
        State.updateTask(agentId, "ratelimit");
        const project = projectOf(State.tasks, agentId);
        pingPhone((s, idleMs) => phone.limit(agentId, project, "usage", limit.resets, s, new Date(), idleMs));
      } else if (message.endsWith("?")) {
        supersedeStop();
        State.updateTask(agentId, "question");
        State.appendStep(agentId, message);
      }
      break;
    }

    case "Stop": {
      State.updateTask(agentId, "finished");
      {
        const waits = [State.pendingApproval, ...State.approvalQueue].some((r) => r?.pillId === agentId);
        const project = projectOf(State.tasks, agentId);
        pingPhone((s, idleMs) => phone.finished(agentId, project, waits, s, new Date(), idleMs));
      }
      // Claude Code puts the turn's answer in the Stop payload itself, so there is
      // no transcript to read (the relay does not even forward its path). Other
      // agents report their last words the same way (Hermes, Codex) or as
      // `message`.
      const finalText = toOneLine(payload.last_assistant_message ?? payload.message ?? "");
      if (finalText) {
        State.appendStep(agentId, finalText);
        const t = State.tasks.find((x) => x.id === agentId);
        if (t) t.finalLine = finalText;
      }
      // A card waiting for an answer is never covered by another alert.
      if (focused && !State.pendingApproval) surface("finished", true);
      else State.setPillBadge(agentId, "finished");
      cancelStopTimer(agentId);
      stopTimers.set(
        agentId,
        window.setTimeout(() => {
          stopTimers.delete(agentId);
          if (isExternalAgent) {
            State.removeTask(agentId);
          } else {
            State.updateTask(agentId, "idle");
            State.setPillBadge(agentId, null);
          }
        }, 5200),
      );
      break;
    }

    case "Interrupt":
      // Codex: the user stopped the turn. Back to idle, nothing to celebrate.
      supersedeStop();
      State.updateTask(agentId, "idle");
      State.setPillBadge(agentId, null);
      break;

    case "StopFailure": {
      supersedeStop();
      // A usage or spend limit is its own state, not an error: the amber card
      // says when it resets, and offers no Retry before then.
      const kind = limitKind(payload.error);
      if (kind) {
        const limit = parseLimit(kind, [payload.last_assistant_message, payload.error_details, payload.message], Date.now());
        setLimit(agentId, limit);
        State.updateTask(agentId, "ratelimit");
        const project = projectOf(State.tasks, agentId);
        pingPhone((s, idleMs) => phone.limit(agentId, project, kind, limit.resets, s, new Date(), idleMs));
        if (focused && !State.pendingApproval) surface("error", true);
        break;
      }
      State.updateTask(agentId, "error");
      if (focused && !State.pendingApproval) surface("error", true);
      else State.setPillBadge(agentId, "error");
      break;
    }

    case "SessionEnd":
      // Nothing left for the timer to do, and it must not outlive the session: a
      // pill recreated within 5.2 s would be removed by it.
      cancelStopTimer(agentId);
      State.clearSessionDiffs(agentId);
      if (isExternalAgent) {
        State.removeTask(agentId);
      } else {
        State.updateTask(agentId, "idle");
        clearSession(agentId);
      }
      break;

    case "SubagentStart":
      State.appendStep(agentId, t("+ subagent"));
      break;

    case "SubagentStop":
      State.appendStep(agentId, t("• subagent done"));
      break;

    case "PermissionRequest": {
      // Only Claude Code and the agents the relay can answer for (Codex, Copilot
      // CLI, Muse Code — same as the Mac) get a card. Anyone else's request is
      // declined at once, so the agent asks in its own terminal.
      if (isExternalAgent && !APPROVAL_AGENTS.has(validAgent!)) {
        if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
        break;
      }

      const requestId = payload.request_id ?? "";
      ensurePill();
      supersedeStop();
      const tool = payload.tool_name ?? "Tool";
      const input = payload.tool_input ?? {};
      // Claude Code asking a question is not a permission to grant: the island
      // shows the options and sends back the one that was picked. Only Claude
      // Code asks questions this way.
      const questions = isExternalAgent ? null : askedQuestions(tool, input);
      const view = questions ? "question" : "approval";
      const info = {
        requestId,
        sessionId,
        pillId: agentId,
        tool,
        command: approvalTarget(tool, input),
        ...(questions ? { questions } : {}),
      };
      // One card, one request. A second one must never quietly replace the first
      // (a human would be staring at request B while A waits for a decision
      // nobody can give), and it isn't sent back to a terminal nobody is
      // watching either: it waits in Glim's queue, acknowledged so its agent
      // keeps waiting, and comes up when the card before it ends.
      const queued = State.approvalQueue.some((r) => r.requestId === requestId);
      if ((State.pendingApproval && State.pendingApproval.requestId !== requestId) || queued) {
        if (!queued) State.queueApproval(info);
        // Its pill shows that something on it is waiting; on the card's own pill,
        // the card's kind (a question stays a question).
        const shown = State.pendingApproval;
        State.updateTask(agentId, shown?.pillId === agentId ? (shown.questions ? "question" : "approval") : view);
        if (requestId) {
          void Bridge.approvalAck(requestId);
          if (!queued) armRequestTimer(island, requestId);
        }
        break;
      }
      // The card always comes up, even over another pill or an island that is
      // already open: its pill comes to the front, and the one you were on
      // comes back once you answer (Mac #117, #120).
      State.beginApproval(info);
      // The relay's short ack window closes in 800 ms; everything below this
      // line is synchronous, so the card really is up by the time it lands.
      if (requestId) void Bridge.approvalAck(requestId);
      State.updateTask(agentId, view);
      // Any agent's card (Claude Code, Codex, Copilot CLI, Muse Code) comes up
      // the same way: beginApproval brought its pill to the front.
      island.alert(view);
      // Glim answers within 108 s or not at all; after that the terminal has
      // taken over and the card would be lying. A repeat of the same request
      // keeps the clock it came with.
      if (!requestTimers.has(requestId)) armRequestTimer(island, requestId);
      break;
    }

    default:
      break;
  }
  State.notify();
}
