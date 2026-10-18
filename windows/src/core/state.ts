// App state — mirror of AppState.swift (the parts the island needs).

import type { AiCheckEvent } from "./ai-check";
import type { ResumeCard } from "../views/resume";
import type { BotEmoteName, BotStateName, Dock, IslandMode, IslandViewName, PillSize } from "./layout";
import type { EyeShape } from "../mochi/engine";
import {
  DEFAULT_MAIN_PILL, HOST_OS, availablePills, orderPills, pillDefinition, sanitizeDeclared,
  toggleDeclared, type HostOs, type PillDefinition,
} from "./pills";
import type { PlanUsage } from "./plan";
import type { ProviderId } from "./providers";
import type { FileDiff } from "./diff";
import type { Bindings } from "./shortcuts";
import { pillColor } from "./pill-colors";
import type { AgentLimit } from "./limit";
import type { WritingAsk } from "./ask";
import { needsCard, type VoiceState } from "./voice";
import type { DraftState } from "./draft";

export type AgentSource = "claudeCode" | "n8n" | "agent";
export type PillBadge = "approval" | "finished" | "error";

export interface AgentTask {
  id: string;
  name: string;
  color: string;
  state: BotStateName;
  stepIndex: number;
  steps: string[];
  /**
   * Position of the newest step in the whole session. `steps` is capped, so
   * `stepIndex` stops moving once it is full; this keeps counting.
   */
  stepSeq?: number;
  source: AgentSource;
  isIntegration: boolean;
  emote?: BotEmoteName | null;
  miniEye?: EyeShape | null;
  pillBadge?: PillBadge | null;
  sessionCwd?: string | null;
  /** Claude Code's session, so "Open terminal" can find the window it runs in. */
  sessionId?: string | null;
  /** When its session last sent an event (epoch ms): a pill whose session went quiet can be taken over. */
  lastEventAt?: number;
  /** Claude's final message after Stop, one line; cleared when a new turn starts. */
  finalLine?: string | null;
  /** The usage or spend limit that stopped it; read only while `state` is "ratelimit". */
  limit?: AgentLimit | null;
}

export interface ApprovalInfo {
  requestId: string;
  sessionId: string;
  /** The pill the request belongs to: VS Code's or Cursor's (Claude Code), or an agent's (Codex…). */
  pillId: string;
  tool: string;
  command: string;
  /** Set when Claude Code is asking a question rather than for a permission. */
  questions?: AskedQuestion[];
}

/** One question of an AskUserQuestion call. */
export interface AskedQuestion {
  question: string;
  options: { label: string; description: string }[];
  multiSelect: boolean;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
}

export type PromptContext =
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "file"; name: string; path?: string };

export interface ResultItem {
  label: string;
  detail: string;
  url?: string;
}

export interface SearchResult {
  title: string;
  items: ResultItem[];
  note?: string;
}

/** A fresh, idle task for a catalog pill, in the colour the user gave it if any. */
function taskFor(def: PillDefinition, colors: unknown, name = def.name): AgentTask {
  return {
    id: def.id, name, color: pillColor(def.id, def.color, colors), state: "idle", stepIndex: 0, steps: [],
    source: def.source, isIntegration: true,
  };
}

/** What an integration poller last reported. */
export interface IntegrationInfo {
  data: Record<string, unknown>;
  error: string | null;
  loaded: boolean;
  configured: boolean;
}

export interface Settings {
  autoCloseInterval: number;
  absenceInterval: number;
  /** Declared pills next to the main one (at most 4), in the order they were added. */
  activeIntegrations: string[];
  /** The always-on workspace pill: VS Code, Cursor, Codex or Antigravity. */
  mainPill: string;
  /** "primary", "active" (the foreground window's display), "cursor", or
   *  `at:<x>,<y>` for one display (logical origin). */
  screen: string;
  /** The notch pill's size (see core/layout.ts pillGeometry). */
  pillSize: PillSize;
  /** The dock on each display, by display key. Rust owns it (dragging the
   *  pill, Settings' picker): whatever the page sends back is ignored. */
  docks?: Record<string, string>;
  /** "normal" (the pill), "ember" or "hidden" (island/presence.ts). Rust owns
   *  it, like `docks`: set it through Bridge.setVisibility. */
  visibility?: "normal" | "ember" | "hidden";
  autostart: boolean;
  hooksInstalled: boolean;
  /** Show the Claude plan pill (5 h and weekly limits) in the island's header. */
  showPlanInNotch: boolean;
  /** The app's status line relay is installed in Claude Code's settings. */
  planRelayInstalled: boolean;
  /** Which local model server the chat talks to (see core/providers.ts). */
  chatProvider: ProviderId;
  /** The model picked for each provider, by provider id. */
  chatModels: Record<string, string>;
  /** Model server addresses once connected; empty means not connected. */
  ollamaUrl: string;
  lmstudioUrl: string;
  /** Global shortcuts the user changed, by action id (see core/shortcuts.ts). */
  shortcuts: Bindings;
  /**
   * A colour of the user's own for a pill's character, by pill ID ("#RRGGBB").
   * Empty means the catalog's colours; read it through core/pill-colors.ts.
   * Same key and values as the Mac's "pillColors".
   */
  pillColors: Record<string, string>;
  /**
   * Interface language: "" follows the system (when Glim has its language,
   * else English), or one of src/i18n's ten codes ("fr", "pt-BR", "zh-Hans"…).
   */
  language: string;
  /** The character on the desktop. Rust owns it: whatever the page sends back is ignored. */
  desktopMochi?: {
    onDesktop: boolean;
    spot: { x: number; y: number; space: string } | null;
  };
  /** 1c's brain: "claude" (the Claude API, default) or "local" (not built yet). */
  aiBrain: "claude" | "local";
  /** The Claude model for the rewrite (src-tauri/src/ai/claude.rs MODELS). */
  aiModel: string;
  /** Monthly cap on Glim's API spend estimate, in US dollars; 0 = none. */
  aiMonthlyCapUsd: number;
  /** Settings → Writing → Apps (1d): "on" / "off" by exe, only where it differs from the default. */
  writingApps: Record<string, string>;
  /** When each app on neither list was last asked about ("YYYY-MM-DD" by exe). Rust's. */
  writingAppsAsked?: Record<string, string>;
  /** The Google Docs tip was dismissed with "Got it" (core/docs-tip.ts). Rust's. */
  writingDocsTipDismissed?: boolean;
  /** Settings → Writing → Rewrite tones (1d): a tone id by category or exe, only where it differs. */
  rewriteTones: Record<string, string>;
  /** Settings → Writing → Strictness (1d): "spelling", "standard" or "full". */
  checkerStrictness: "spelling" | "standard" | "full";
  /** Settings → Privacy (1d): the user's own paused apps (exe names) and sites. */
  pauseApps: string[];
  pauseSites: string[];
  /** Settings → General → File drop zone: the pill takes files dragged onto it. */
  fileDropZone: boolean;
  /** Settings → Writing → Check with AI automatically when I pause typing (§15.1). Off by default. */
  aiAutoCheck: boolean;
  /** Settings → Writing → Dictation → Speech model (voice 1.5b): whisper's "base" or "small". */
  voiceModel: "base" | "small";
  /** Settings → Writing → Dictation → Words Glim should know: whisper's prompt, never sent anywhere. */
  voiceWords: string;
  /** Settings → Writing (1e, §7a.6): the mini lantern beside the field. Off by default. */
  writingBubble: boolean;
  /** Settings → Email drafts → Your address (1.5d): the only sender, and "me". */
  draftFrom: string;
  /** Names picked on the Draft email card → addresses. Rust's (draft_forget removes one). */
  draftRecipients?: Record<string, string>;
}

export const DEFAULT_SETTINGS: Settings = {
  autoCloseInterval: 15,
  absenceInterval: 180,
  activeIntegrations: [],
  mainPill: DEFAULT_MAIN_PILL,
  screen: "primary",
  pillSize: "large",
  autostart: false,
  hooksInstalled: false,
  showPlanInNotch: false,
  planRelayInstalled: false,
  chatProvider: "ollama",
  chatModels: {},
  ollamaUrl: "",
  lmstudioUrl: "",
  shortcuts: {},
  pillColors: {},
  language: "",
  aiBrain: "claude",
  aiModel: "claude-haiku-5-5",
  aiMonthlyCapUsd: 0,
  writingApps: {},
  rewriteTones: {},
  checkerStrictness: "full",
  pauseApps: [],
  pauseSites: [],
  fileDropZone: true,
  aiAutoCheck: false,
  voiceModel: "base",
  voiceWords: "",
  writingBubble: false,
  draftFrom: "",
};

type Listener = () => void;

/** Live diffs kept per pill (oldest dropped first) — same cap as macOS. */
export const MAX_DIFFS_PER_PILL = 50;
/** A pill's diffs are forgotten after an hour without a new one, as on macOS. */
export const DIFF_TTL_MS = 3_600_000;

/**
 * One writing suggestion from the instant checker (src-tauri/src/writing),
 * for the island's list. Live only: never logged or stored.
 */
export interface WritingSuggestion {
  /** Offsets in characters in the field (for applying fixes, 1b). */
  start: number;
  end: number;
  problem: string;
  before: string;
  after: string;
  kind: string;
  message: string;
  /** "" means "remove it". */
  replacements: string[];
  /** The first replacement is clearly the best: Apply all takes it. */
  clear: boolean;
}

/**
 * What clicking a replacement does (1b, src-tauri/src/writing): typed into
 * the field ("apply"), copied on click ("copy", fields Glim can't type in),
 * nothing ("view", apps not yet tested for fixes), or nothing until the user
 * is back in that field ("away").
 */
export type WritingFixMode = "apply" | "copy" | "view" | "away";

/** The list's check id (a click names it), its fix mode and the app's name. */
export interface WritingFix {
  checkId: number;
  mode: WritingFixMode;
  app: string;
}

/**
 * A click's outcome (a code from Rust). Apply all also says how many fixes
 * went in, of how many (`total` 0 for a single fix).
 */
export interface WritingResult {
  outcome: string;
  app: string;
  done?: number;
  total?: number;
  /** Apply all: suggestions left to the user, their best option not clear. */
  skipped?: number;
}

/** The Rewrite card's state (1c): see core/rewrite.ts. */
export interface RewriteState {
  /** choose a tone → sending → done (the rewrite) or failed (a code). */
  phase: "choose" | "sending" | "done" | "failed";
  /** Rust's id for the rewrite; 0 until Rust has taken the request. */
  id: number;
  tone: string;
  text: string;
  mode: WritingFixMode;
  app: string;
  /** Glim would type it: "Use this". Otherwise Copy only. */
  offerable: boolean;
  model: string;
  costUsd: number;
  code: string;
  retryAfter: number | null;
  /** What "Use this" or "Copy" did (a writing outcome code), "" before. */
  outcome: string;
  /** "Use this" or "Copy" is on its way. */
  busy: boolean;
  /** The field's default tone (1d, §14.3): first and highlighted; a second
   *  press of the hotkey sends it. "" until Rust says. */
  defaultTone: string;
}

class AppState {
  mode: IslandMode = "hidden";
  /** Where the island is docked on its display (Rust tells: "placement"). */
  dock: Dock = "top-center";
  view: IslandViewName = "overview";

  tasks: AgentTask[] = [];
  focusId: string | null = null;

  stateOverride: BotStateName | null = null;

  /** Cursor in logical screen pixels, origin top-left (like AppState.mousePosition). */
  mouse = { x: 0, y: 0 };
  /** Cursor relative to the island's top-left corner. */
  mouseInIsland = { x: 0, y: 0 };

  isPinned = false;
  paused = false;

  uploadProgress = 0;
  uploadDuration = 2.4;
  fileDragOver = false;

  promptContext: PromptContext | null = null;
  droppedFile: { name: string; path: string } | null = null;
  noteMessage: string | null = null;
  searchResult: SearchResult | null = null;
  chatHistory: ChatMessage[] = [];
  pendingApproval: ApprovalInfo | null = null;
  /**
   * Requests that came while a card was up, oldest first. Each was acknowledged
   * (its agent keeps waiting for Glim) and comes up in turn when the card
   * before it ends; hooks.ts withdraws one whose turn ends or time runs out.
   */
  approvalQueue: ApprovalInfo[] = [];
  /** The focused field's writing suggestions (Rust: "writing-suggestions"). */
  writing: WritingSuggestion[] = [];
  writingFix: WritingFix = { checkId: 0, mode: "view", app: "" };
  /** The last click's outcome (a code from Rust), shown for a few seconds. */
  writingResult: WritingResult | null = null;
  /** A fix is on its way: the buttons wait for the outcome. */
  writingBusy = false;
  /** The Rewrite card (1c, core/rewrite.ts); null when it is closed. */
  rewrite: RewriteState | null = null;
  /** "Check with AI" (core/ai-check.ts): the last check's phase; null for none. */
  aiCheck: AiCheckEvent | null = null;
  /**
   * The caret is in a paragraph Glim checks, even with nothing found
   * (Suggestions.checkable): a lantern click in the open island then opens
   * the empty list, for Check with AI (owner, 2026-10-14).
   */
  writingCheckable = false;
  /** "Check writing in [app]?" (1d, core/ask.ts), waiting for an answer; null for none. */
  writingAsk: WritingAsk | null = null;
  /** "Pick up where you left off" (views/resume.ts): the repos shown; null when closed. */
  resume: ResumeCard[] | null = null;
  /** The Google Docs tip (core/docs-tip.ts) is up, or waiting behind an approval. */
  writingDocsTip = false;
  /** Push-to-talk dictation (core/voice.ts): where it is; null for nothing going on. */
  voice: VoiceState | null = null;
  /** The "Draft email" card (core/draft.ts); null for none. */
  draft: DraftState | null = null;
  /** The pill that was in front when the card came up; it comes back after. */
  focusBeforeApproval: string | null = null;

  integrations: Record<string, IntegrationInfo> = {};

  /** Claude's 5 h / weekly limits, from the status line (null until the first call). */
  planUsage: PlanUsage | null = null;
  /** The plan card is open in place of the overview's left card. */
  showingPlanDetail = false;
  /** Per-pill file diffs, in order of reception. Steps carry their ids. */
  sessionDiffs = new Map<string, FileDiff[]>();
  private sessionDiffTimers = new Map<string, number>();
  /** Never reset, so an id can never point at a newer diff than the one tapped. */
  private nextDiffId = 0;
  /**
   * Mochi is out of the island — on the desktop, flying, or being dragged
   * there — so the island's own Mochi is hidden (AppState.mochiOnDesktop).
   */
  mochiOnDesktop = false;

  lastActivity = performance.now();

  settings: Settings = { ...DEFAULT_SETTINGS };

  /** Which pills this build offers depends on it (Claude Desktop is Windows only). */
  os: HostOs = HOST_OS;

  private listeners = new Set<Listener>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Marks the UI dirty; the island re-renders on the next frame. */
  notify() {
    for (const fn of this.listeners) fn();
  }

  get focusTask(): AgentTask | null {
    return this.tasks.find((t) => t.id === this.focusId) ?? this.tasks[0] ?? null;
  }

  get effectiveState(): BotStateName {
    return this.stateOverride ?? this.focusTask?.state ?? "idle";
  }

  get otherTasks(): AgentTask[] {
    return this.tasks.filter((t) => t.id !== this.focusId);
  }

  setFocus(id: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    this.focusId = id;
    this.showingPlanDetail = false;
    t.pillBadge = null;
    this.notify();
  }

  /**
   * A permission card or a question comes up: its pill comes to the front, and
   * the pill that was there is remembered (HookServer.focusBeforeApproval).
   */
  beginApproval(info: ApprovalInfo) {
    this.pendingApproval = info;
    this.isPinned = true;
    if (this.focusBeforeApproval == null) this.focusBeforeApproval = this.focusId;
    this.setFocus(info.pillId);
  }

  /**
   * The card has its answer, or is withdrawn: the session carries on, and the
   * pill you were on comes back — unless you moved to another one meanwhile.
   */
  endApproval() {
    const req = this.pendingApproval;
    if (!req) return;
    this.pendingApproval = null;
    this.isPinned = false;
    this.updateTask(req.pillId, "working");
    this.setPillBadge(req.pillId, null);
    const previous = this.focusBeforeApproval;
    this.focusBeforeApproval = null;
    if (previous && this.focusId === req.pillId && this.tasks.some((t) => t.id === previous)) {
      this.focusId = previous;
    }
    // The next request in the queue is the card now (the island shows it:
    // island.ts closeApproval, hooks.ts dropPendingCard).
    const next = this.approvalQueue.shift();
    if (next) {
      this.beginApproval(next);
      this.updateTask(next.pillId, next.questions ? "question" : "approval");
    }
    this.notify();
  }

  /** A request waits behind the card on screen; its pill is badged meanwhile. */
  queueApproval(info: ApprovalInfo) {
    this.approvalQueue.push(info);
    this.setPillBadge(info.pillId, "approval");
    this.notify();
  }

  /** Takes the queued requests `drop` picks out of the queue, and returns them. */
  withdrawQueued(drop: (r: ApprovalInfo) => boolean): ApprovalInfo[] {
    const gone = this.approvalQueue.filter(drop);
    if (gone.length === 0) return gone;
    this.approvalQueue = this.approvalQueue.filter((r) => !drop(r));
    this.notify();
    return gone;
  }

  updateTask(id: string, state: BotStateName) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    this.notify();
  }

  appendStep(id: string, step: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    const newest = t.stepSeq ?? t.steps.length - 1;
    t.steps.push(step);
    if (t.steps.length > 20) t.steps.shift();
    t.stepIndex = t.steps.length - 1;
    t.stepSeq = newest + 1;
    this.notify();
  }

  setPillBadge(id: string, badge: PillBadge | null) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.pillBadge = badge;
    this.notify();
  }

  /** Stores a diff for a pill and returns its id (for the ticker step). */
  appendSessionDiff(pillId: string, diff: FileDiff): number {
    const id = this.nextDiffId++;
    const list = this.sessionDiffs.get(pillId) ?? [];
    list.push({ ...diff, id });
    while (list.length > MAX_DIFFS_PER_PILL) list.shift();
    this.sessionDiffs.set(pillId, list);
    // One timer per pill, re-armed on every diff — nothing polls.
    const prev = this.sessionDiffTimers.get(pillId);
    if (prev != null) window.clearTimeout(prev);
    this.sessionDiffTimers.set(
      pillId,
      window.setTimeout(() => this.clearSessionDiffs(pillId), DIFF_TTL_MS),
    );
    return id;
  }

  findDiff(pillId: string, id: number): FileDiff | null {
    return this.sessionDiffs.get(pillId)?.find((d) => d.id === id) ?? null;
  }

  clearSessionDiffs(pillId: string) {
    const timer = this.sessionDiffTimers.get(pillId);
    if (timer != null) window.clearTimeout(timer);
    this.sessionDiffTimers.delete(pillId);
    this.sessionDiffs.delete(pillId);
  }

  /** The always-on workspace pill, once the setting has been checked. */
  get mainPillId(): string {
    return sanitizeDeclared(this.settings, this.os).mainPill;
  }

  /**
   * True for a pill that stays when its session ends: the main pill and the
   * declared ones go back to idle instead of going away.
   */
  isKept(id: string): boolean {
    const d = sanitizeDeclared(this.settings, this.os);
    return id === d.mainPill || d.activeIntegrations.includes(id);
  }

  /**
   * Loads the catalog pills: the main pill always, the declared ones, and none
   * of the others — a pill that is mid-session stays until its session ends.
   * Safe to call any number of times. AppState.loadIntegrationTasks on macOS.
   */
  loadIntegrationTasks() {
    const d = sanitizeDeclared(this.settings, this.os);
    this.settings.mainPill = d.mainPill;
    this.settings.activeIntegrations = d.activeIntegrations;
    for (const def of availablePills(this.os)) {
      const shouldLoad = def.id === d.mainPill || d.activeIntegrations.includes(def.id);
      const idx = this.tasks.findIndex((t) => t.id === def.id);
      if (shouldLoad && idx < 0) this.tasks.push(taskFor(def, this.settings.pillColors));
      const busy = idx >= 0 && (this.tasks[idx].state !== "idle" || this.tasks[idx].steps.length > 0);
      if (!shouldLoad && idx >= 0 && !busy) this.tasks.splice(idx, 1);
    }
    // A colour picked in Settings reaches the pills that are already up.
    for (const t of this.tasks) {
      const def = pillDefinition(t.id);
      if (def) t.color = pillColor(def.id, def.color, this.settings.pillColors);
    }
    this.tasks = orderPills(this.tasks, d.mainPill);
    if (!this.focusId || !this.tasks.some((t) => t.id === this.focusId)) this.focusId = d.mainPill;
    this.notify();
  }

  /**
   * A session is over. The main and declared pills are put back as they were;
   * any other pill goes away (AppState.removeTask on macOS).
   */
  removeTask(id: string) {
    const idx = this.tasks.findIndex((t) => t.id === id);
    if (idx < 0) return;
    if (this.isKept(id)) {
      const t = this.tasks[idx];
      t.state = "idle";
      t.steps = [];
      t.stepIndex = 0;
      delete t.stepSeq;
      t.pillBadge = null;
      t.finalLine = null;
      const def = pillDefinition(id);
      if (def) t.name = def.name;
      this.clearSessionDiffs(id);
      this.notify();
      return;
    }
    this.tasks.splice(idx, 1);
    this.clearSessionDiffs(id);
    if (this.focusId === id) this.focusId = this.tasks[0]?.id ?? this.mainPillId;
    this.notify();
  }

  /**
   * Creates the pill of a tagged agent on its first event; no-op if it exists.
   * Inserted right after the main pill so it is in the visible slice(0,4). A
   * catalog agent wears its catalog colour, as on macOS.
   */
  upsertExternalAgent(id: string, name: string, color: string) {
    if (this.tasks.some((t) => t.id === id)) return;
    const def = pillDefinition(id);
    this.insertAfterMain({
      id, name, color: def ? pillColor(def.id, def.color, this.settings.pillColors) : color,
      state: "idle", stepIndex: 0, steps: [],
      source: "agent", isIntegration: false,
    });
  }

  /**
   * The pill a Claude Code session belongs to (VS Code or Cursor). It is made
   * for the session when it is neither the main pill nor declared, as
   * upsertWorkspaceTask does on macOS.
   */
  upsertWorkspacePill(id: string, name: string, cwd: string): AgentTask | null {
    let t = this.tasks.find((x) => x.id === id);
    if (!t) {
      const def = pillDefinition(id);
      if (!def) return null;
      t = taskFor(def, this.settings.pillColors, name);
      // A session pill (hooks.ts) is its catalog pill's, under its own id.
      t.id = id;
      this.insertAfterMain(t);
    }
    t.name = name;
    if (cwd) t.sessionCwd = cwd;
    return t;
  }

  private insertAfterMain(t: AgentTask) {
    const at = this.tasks.findIndex((x) => x.id === this.mainPillId) + 1;
    this.tasks.splice(at, 0, t);
    if (!this.focusId) this.focusId = t.id;
    this.notify();
  }

  /** Declares or undeclares a pill (max 4 next to the main one). */
  toggleIntegration(id: string) {
    const next = toggleDeclared(sanitizeDeclared(this.settings, this.os), id, this.os);
    if (!next) return;
    this.settings.activeIntegrations = next;
    if (!next.includes(id) && this.focusId === id) this.focusId = this.mainPillId;
    this.loadIntegrationTasks();
  }

  /**
   * What the island opens on. A card waiting for an answer comes first, so
   * reopening a folded island shows it again (Mac #117, #290).
   */
  defaultView(): IslandViewName {
    return this.lanternView() ?? (this.tasks.length === 0 ? "empty" : "overview");
  }

  /**
   * What a click on the lantern (the pill's or the ember's) opens, whatever
   * the island shows: a card waiting for an answer (urgent), then the writing
   * suggestions while the badge shows them; null for neither. An agent's
   * finished or error card never takes the lantern from the suggestions:
   * those stay one click away under Home (the owner, 2026-10-10).
   */
  lanternView(): IslandViewName | null {
    if (this.pendingApproval) return this.pendingApproval.questions ? "question" : "approval";
    if (this.writingAsk) return "writingAsk";
    if (this.writingDocsTip) return "writingDocsTip";
    if (this.draft) return "draft";
    if (needsCard(this.voice)) return "voice";
    if (this.writing.length > 0) return "suggestions";
    return null;
  }
}

export const State = new AppState();
