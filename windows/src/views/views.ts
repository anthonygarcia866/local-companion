// Island views — DOM ports of IslandViewContent.swift. Paddings, font sizes,
// colours and wording are copied from the Swift views so both platforms read
// identically.

import { AI_KIND, aiCheckNote, aiSending } from "../core/ai-check";
import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Ticker } from "./ticker";
import { State, type AgentTask, type WritingFix, type WritingResult, type WritingSuggestion } from "../core/state";
import { limitHasReset, type AgentLimit } from "../core/limit";
import { showsSwitcher, washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { createMiniLantern, pruneMiniLanterns } from "../mascot/minis";
import { buildPrompt } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { pillCard } from "./pill-card";
import { pillDefinition, sessionSubtitle } from "../core/pills";
import { PlanCard, buildPlanPill, claudePillVisible, planCardOpen } from "./usage";
import { buildRewrite } from "./rewrite";
import { buildAsk } from "./ask";
import { buildResume } from "./resume";
import { buildDocsTip } from "./docs-tip";
import { buildVoice } from "./voice";
import { buildDraft } from "./draft";
import { openToneCard } from "../core/rewrite";
import { buildDiffCard } from "./diff";
import { lastTextStep } from "../core/diff";
import { Bridge } from "../core/bridge";
import { SHORTCUTS, displayKeys, effective } from "../core/shortcuts";
import { armedFor, cardOnScreen, secondsLeft } from "../core/allow";
import { buildRecap } from "./recap";
import { language, t, tl, tn, type Msg } from "../i18n/i18n";

export interface ViewActions {
  setView(v: IslandViewName): void;
  /** "Cancel" on a dropped file: forgets it and goes back home. */
  cancelDrop(): void;
  collapse(): void;
  /** Folds a waiting card to the compact island without answering it. */
  foldApproval(): void;
  setFocus(id: string): void;
  openTerminal(): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  decide(d: "allow" | "deny"): void;
  /** Answers the question Claude Code asked: question text → chosen label. */
  answer(answers: Record<string, string | string[]>): void;
  /** Hands the pending request back to the terminal. */
  answerInTerminal(): void;
  setAutoClose(seconds: number): void;
  openSettingsWindow(): void;
}

export interface ViewHost {
  el: HTMLElement;
  sync(): void;
  /** Called when the view becomes active, for views with a text field. */
  focus?(): void;
  /** Called every frame while the view is on screen. True = needs another frame. */
  tick?(nowMs: number): boolean | void;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

export function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function btn(
  label: string | Msg,
  kind: "primary" | "secondary",
  onClick: () => void,
  kbd?: string,
): HTMLElement {
  return h(
    "button",
    { class: `btn ${kind}`, onclick: onClick },
    h("span", { text: label }),
    kbd ? h("span", { class: "kbd", text: kbd }) : null,
  );
}

/** AgentWho — coloured dot + task name + grey label. */
function agentWho(task: AgentTask | null, label: string): HTMLElement {
  const row = h("div", { class: "who-row" });
  if (task) {
    row.append(dot(task.color, 8), h("span", { class: "n", text: task.name }));
  }
  row.append(h("span", { text: label }));
  return row;
}

export function stack(padLeft: number, padRight: number, ...children: Node[]): HTMLElement {
  const el = h("div", { class: "stack" }, ...children);
  el.style.padding = `4px ${padRight}px 4px ${padLeft}px`;
  return el;
}

// ── Header ────────────────────────────────────────────────────────────────────

export function buildHeader(actions: ViewActions): ViewHost {
  const tabHome = h("button", { class: "tab", title: tl("Overview"), onclick: () => go("overview") }, svg(ICONS.house, 13));
  const tabChat = h("button", { class: "tab", title: tl("Ask"), onclick: () => go("prompt") }, svg(ICONS.bubble, 13));
  const tabDrop = h("button", { class: "tab", title: tl("Drop"), onclick: () => go("upload") }, svg(ICONS.plus, 13));
  // Only while the focused field has writing suggestions.
  const tabWriting = h("button", { class: "tab", title: tl("Writing suggestions"), onclick: () => go("suggestions") }, svg(ICONS.pencil, 13));

  const gearBtn = h("button", { title: tl("Settings"), onclick: () => go("settings") }, svg(ICONS.gear, 14));
  // Folds the open island back to the pill (a waiting card is folded, never
  // dropped). The island takes no focus, so neither does this.
  const minBtn = h("button", { class: "min-btn", title: tl("Minimize"), onclick: () => actions.collapse() }, svg(ICONS.minus, 14));
  // Plan usage pill (off by default): before the gear, as on the Mac.
  const claudePill = buildPlanPill();
  const planPills = h("div", { class: "plan-pills" }, claudePill.el);

  function go(v: IslandViewName) {
    actions.setView(v);
  }

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabHome, tabChat, tabDrop, tabWriting),
    h("div", { class: "header-actions" }, planPills, minBtn, gearBtn),
  );

  return {
    el,
    sync() {
      const v = State.view;
      tabHome.classList.toggle("on", v === "overview" || v === "empty");
      tabChat.classList.toggle("on", v === "prompt");
      tabDrop.classList.toggle("on", v === "upload");
      tabWriting.classList.toggle("on", v === "suggestions");
      tabWriting.style.display = State.writing.length > 0 || v === "suggestions" ? "" : "none";
      gearBtn.classList.toggle("on", v === "settings");
      clear(gearBtn);
      gearBtn.append(svg(v === "settings" ? ICONS.gearFill : ICONS.gear, 14));
      syncPlanPills();
      el.style.opacity = v === "confused" ? "0" : "1";
    },
  };

  function syncPlanPills() {
    const claudeOn = claudePillVisible();
    claudePill.el.style.display = claudeOn ? "" : "none";
    planPills.classList.toggle("on", claudeOn);
    if (claudeOn) claudePill.sync();
  }
}

// ── Overview ──────────────────────────────────────────────────────────────────

function buildOverview(actions: ViewActions): ViewHost {
  /** The diff open in the left card (a FileDiff id), as activeDiffId on macOS. */
  let activeDiffId: number | null = null;
  const closeDiff = () => {
    if (activeDiffId == null) return;
    activeDiffId = null;
    State.notify();
  };
  const ticker = new Ticker((diffId) => {
    activeDiffId = diffId;
    State.notify();
  });
  const who = h("div", { class: "who" });
  const tickerBody = h("div", { class: "card-body" }, who, ticker.el);
  const leftBody = h("div", { class: "left-body" });
  const jump = h(
    "button",
    { class: "icon-btn jump", title: tl("Open"), onclick: () => actions.openTarget() },
    svg(ICONS.arrowUpRight, 8),
  );
  const left = card(null, leftBody, jump);
  const pills = h("div", { class: "pills" });
  const right = card(null, pills);
  // Opened from a plan pill in the header: stands in for the left card.
  const plan = new PlanCard();
  let planTimer: number | null = null;

  const el = h("div", { class: "view overview" },
    h("div", { class: "left" }, left),
    h("div", { class: "right" }, right),
  );

  let pillIds = "";
  let lastFocus: string | null = null;
  let mode: "ticker" | "card" | "plan" | "diff" | null = null;
  let cardKey = "";

  // Leaving the overview or folding the island closes the diff, as on macOS.
  State.subscribe(() => {
    if (activeDiffId != null && (State.view !== "overview" || State.mode !== "expanded")) {
      activeDiffId = null;
    }
  });
  // Escape steps back out of the diff before it closes the island.
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key !== "Escape" || activeDiffId == null || State.view !== "overview") return;
      e.stopImmediatePropagation();
      closeDiff();
    },
    true,
  );

  /** The countdowns move every 30 s while a card is open, and only then. */
  function syncPlanTimer(open: boolean) {
    const stop = () => {
      if (planTimer != null) window.clearInterval(planTimer);
      planTimer = null;
    };
    if (!open) return stop();
    if (planTimer != null) return;
    planTimer = window.setInterval(() => {
      if (planCardOpen() && State.mode === "expanded") State.notify();
      else stop();
    }, 30_000);
  }

  return {
    el,
    tick(nowMs: number) {
      if (mode !== "ticker") return false;
      ticker.tick(nowMs);
      return ticker.animating;
    },
    sync() {
      const task = State.focusTask;
      if (task?.id !== lastFocus) {
        lastFocus = task?.id ?? null;
        activeDiffId = null;
        cardKey = "";
        mode = null;
      }

      // A workspace or agent pill with a live session keeps the ticker; any
      // other pill shows its status card, like IntegrationCardView.
      const sessionActive = task != null && hasSessionTicker(task);

      const planOpen = planCardOpen();
      if (mode === "plan" && !planOpen) {
        mode = null;
        cardKey = "";
      }
      syncPlanTimer(planOpen);

      // A diff that has since been dropped (cap, expiry, session end) just closes.
      const diff = task && activeDiffId != null ? State.findDiff(task.id, activeDiffId) : null;
      if (!diff) activeDiffId = null;

      if (planOpen) {
        if (mode !== "plan") {
          clear(leftBody);
          leftBody.append(plan.el);
          mode = "plan";
        }
        plan.sync();
      } else if (task && diff) {
        const key = `diff~${task.id}~${diff.id}`;
        if (key !== cardKey) {
          cardKey = key;
          mode = "diff";
          clear(leftBody);
          leftBody.append(buildDiffCard(diff, {
            dismiss: () => closeDiff(),
            open: (path) => void Bridge.openFileInVSCode(path),
          }));
        }
      } else if (task && sessionActive) {
        if (mode !== "ticker") {
          clear(leftBody);
          leftBody.append(tickerBody);
          mode = "ticker";
          cardKey = "";
        }
        clear(who);
        // The agent's name is already the pill's: the label says what kind of
        // pill it is, as on the Mac (PillDefinition.sessionSubtitle).
        who.append(
          dot(task.color, 7),
          h("span", { class: "name", text: task.name }),
          h("span", { class: "tool", text: t(sessionSubtitle(task.id)) }),
        );
        if (task.steps.length > 1) {
          who.append(h("span", {
            class: "count",
            text: `${Math.min(task.stepIndex + 1, task.steps.length)}/${task.steps.length}`,
          }));
        }
        ticker.sync(task);
      } else if (task) {
        const info = State.integrations[task.id];
        const key = [language(), task.id, task.color, info?.error, info?.configured].join("~");
        if (key !== cardKey) {
          cardKey = key;
          mode = "card";
          clear(leftBody);
          leftBody.append(pillCard(task, () => actions.openSettingsWindow()));
        }
      }

      jump.style.display = mode === "plan" || mode === "diff" ? "none" : "";

      const others = State.otherTasks.slice(0, 4);
      // The right card is the switcher for the other pills: with none, it was
      // an empty card. The session card takes the whole width instead.
      el.classList.toggle("solo", !showsSwitcher(others.length));
      const pillKey = others.map((t) => `${t.id}:${t.color}:${t.pillBadge ?? ""}`).join("|");
      if (pillKey !== pillIds) {
        pillIds = pillKey;
        clear(pills);
        for (const t of others) pills.append(buildPill(t, actions));
        pruneMiniLanterns();
      }
    },
  };
}

/**
 * IntegrationCardView.agentSessionActive: a workspace tool or an agent — or
 * any other tagged agent — with something going on.
 */
export function hasSessionTicker(task: AgentTask): boolean {
  // A pill made for an agent's session (Gemini CLI, Codex… not declared) only
  // exists while that session does: it keeps the ticker from the first event.
  if (task.source === "agent" && !task.isIntegration) return true;
  const category = pillDefinition(task.id)?.category;
  const isSession = category === "workspace" || category === "agent" ||
    (category == null && task.id.startsWith("agent_"));
  return isSession && (task.state !== "idle" || task.steps.length > 0);
}

function buildPill(task: AgentTask, actions: ViewActions): HTMLElement {
  const label = task.id === "integration_claude" ? "VS Code" : task.name;
  const mini = createMiniLantern(task, 24);
  const pill = h(
    "div",
    { class: "pill", onclick: () => actions.setFocus(task.id) },
    mini,
    h("span", { class: "lbl", text: label }),
  );
  pill.style.borderColor = `${task.color}24`;
  pill.addEventListener("mouseenter", () => {
    pill.style.background = `${task.color}2e`;
    pill.style.borderColor = `${task.color}8c`;
    pill.style.boxShadow = `0 2px 10px ${task.color}59`;
    (pill.querySelector(".lbl") as HTMLElement).style.color = lighten(task.color, 0.3);
  });
  pill.addEventListener("mouseleave", () => {
    pill.style.background = "";
    pill.style.borderColor = `${task.color}24`;
    pill.style.boxShadow = "";
    (pill.querySelector(".lbl") as HTMLElement).style.color = "";
  });

  if (task.pillBadge) {
    const colors = { approval: "#F5A524", finished: "#22C55E", error: "#F4505E" } as const;
    const icons = { approval: ICONS.bang, finished: ICONS.check, error: ICONS.xmark } as const;
    const inner = h("i", { style: `background:${colors[task.pillBadge]}` }, svg(icons[task.pillBadge], 6, { stroke: task.pillBadge === "finished" ? 3 : 0 }));
    const badge = h("div", { class: "pill-badge" }, inner);
    badge.style.boxShadow = `0 0 4px ${colors[task.pillBadge]}99`;
    pill.append(badge);
  }
  return pill;
}

function lighten(hex: string, amount: number): string {
  const v = parseInt(hex.replace("#", ""), 16);
  const c = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map((x) =>
    Math.min(255, Math.round(x + amount * 255)),
  );
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

// ── Empty ─────────────────────────────────────────────────────────────────────

function buildEmpty(actions: ViewActions): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px;flex-direction:row;align-items:center;gap:16px" },
    h(
      "div",
      { style: "display:flex;flex-direction:column;gap:5px" },
      h("div", { class: "title", text: tl("Nothing running right now.") }),
      h("div", { class: "sub", text: tl("Drop a file or window, or ask me anything.") }),
    ),
    h("div", { class: "grow" }),
    btn(tl("Ask Claude"), "primary", () => actions.setView("prompt")),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Approval ──────────────────────────────────────────────────────────────────

/** How long a fresh permission card ignores clicks on its buttons. */
const CLICK_GUARD_MS = 600;

/**
 * The ⌃ in the corner of a waiting card: folds the island to its compact size
 * and leaves the request waiting — nothing is answered (Mac #290). Opening the
 * island again brings the card back.
 */
function foldButton(actions: ViewActions): HTMLElement {
  return h(
    "button",
    { class: "icon-btn fold", title: tl("Later — keep it waiting"), onclick: () => actions.foldApproval() },
    svg(ICONS.chevronUp, 8, { stroke: 2.4 }),
  );
}

/** A key chord on a button ("Ctrl+Alt+Shift+Y"), or nothing when the action has no key. */
function keyOf(id: "allowApproval" | "denyApproval"): string {
  const d = SHORTCUTS.find((s) => s.id === id);
  const b = d ? effective(d, State.settings.shortcuts) : null;
  return b && b.enabled && b.keys ? displayKeys(b.keys) : "";
}

export function buildApproval(actions: ViewActions): ViewHost {
  const who = h("div");
  const code = h("div", { class: "code" });
  // The Allow key's second press (fix e): "Press again to allow · 3…2…1".
  const confirm = h("div", { class: "suggestion-more", role: "status", "data-role": "allow-confirm" });
  const row = h("div", { class: "actions" });
  const denyKey = h("span", { class: "kbd", "data-role": "deny-key" });
  const allowKey = h("span", { class: "kbd", "data-role": "allow-key" });
  const el = h("div", { class: "view" },
    card("amber", stack(116, 16, who, code, confirm, row), foldButton(actions)));
  let rowKey = "";
  // The card pops up under a cursor that was busy with something else: a click
  // meant for the window underneath must not land on Allow. Clicks in the first
  // moments after a new request appears are ignored.
  let shownFor: string | null = null;
  let shownAt = 0;
  const guarded = (d: "allow" | "deny") => () => {
    if (performance.now() - shownAt < CLICK_GUARD_MS) return;
    actions.decide(d);
  };
  return {
    el,
    sync() {
      const req = State.pendingApproval?.requestId ?? null;
      if (req !== shownFor) {
        shownFor = req;
        shownAt = performance.now();
      }
      clear(who);
      const whoRow = agentWho(State.focusTask, t("needs permission"));
      // Requests queued behind this one come up in turn (State.approvalQueue).
      const waiting = State.approvalQueue.length;
      if (waiting > 0) whoRow.append(h("span", { class: "queued", text: t("{count} more waiting", { count: String(waiting) }) }));
      who.append(whoRow);
      // The whole point of approving here rather than in the terminal: this line
      // is the command, the file path or the URL being authorised, not just the
      // name of the tool asking.
      const command = State.pendingApproval?.command || State.pendingApproval?.tool || "…";
      code.textContent = command;
      // On screen (open, not folded) with the whole command in view: the
      // Allow key's clock (core/allow.ts). A box that scrolls or cuts off
      // isn't the whole command. Measured when the key is pressed, without
      // the "Press again" line, which only takes room while it is showing.
      if (req && State.mode === "expanded") {
        cardOnScreen(req, performance.now(), () => {
          const shown = confirm.style.display;
          confirm.style.display = "none";
          const fits = code.scrollHeight <= code.clientHeight + 1 && code.scrollWidth <= code.clientWidth + 1;
          confirm.style.display = shown;
          return fits;
        });
      }
      // A destructive command's first press (core/allow.ts): the card asks
      // for the second and counts down the seconds left for it.
      const now = performance.now();
      const armed = req ? armedFor(now) : null;
      confirm.textContent = armed && armed.requestId === req ? t("Press again to allow · {seconds}", { seconds: String(secondsLeft(armed, now)) }) : "";
      confirm.style.display = confirm.textContent ? "" : "none";
      denyKey.textContent = keyOf("denyApproval");
      allowKey.textContent = keyOf("allowApproval");
      denyKey.style.display = denyKey.textContent ? "" : "none";
      allowKey.style.display = allowKey.textContent ? "" : "none";
      // Two buttons, built once. Rebuilding them between a mouse-down and a
      // mouse-up would swallow the click, and there is nothing left to vary:
      // "Always" is gone until the remembered-rules list exists to back it.
      if (rowKey === "built") return;
      rowKey = "built";
      clear(row);
      // The keys shown are Glim's global ones (Settings → Shortcuts), never a
      // bare Y or N: Glim doesn't take the keyboard, so a single letter would
      // go to the app in front. Their text changes in place, so the buttons
      // themselves are never rebuilt under a click.
      const deny = btn(tl("Deny"), "secondary", guarded("deny"));
      const allow = btn(tl("Allow"), "primary", guarded("allow"));
      deny.append(denyKey);
      allow.append(allowKey);
      row.append(deny, allow);
    },
  };
}

// ── Question ──────────────────────────────────────────────────────────────────

function buildQuestion(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title question-text" });
  const row = h("div", { class: "actions options" });
  const fold = foldButton(actions);
  const el = h("div", { class: "view" }, card("cyan", stack(116, 16, who, title, row), fold));

  // Where we are in the request on screen: which question, what is answered so
  // far, and what is ticked in a pick-several question.
  let requestId = "";
  let index = 0;
  let answers: Record<string, string | string[]> = {};
  let picked = new Set<string>();
  // The buttons are only rebuilt when what they show changes: rebuilding them
  // between a mouse-down and a mouse-up would swallow the click.
  let rowKey = "";

  const next = (question: string, answer: string | string[], total: number) => {
    answers[question] = answer;
    picked = new Set();
    index += 1;
    if (index >= total) actions.answer(answers);
    else State.notify();
  };

  return {
    el,
    sync() {
      const questions = State.pendingApproval?.questions;
      clear(who);
      // Only a request that is waiting can be folded away and come back.
      fold.style.display = State.pendingApproval ? "" : "none";

      // A question that arrived as a notification has nothing to pick from.
      if (!questions) {
        who.append(agentWho(State.focusTask, t("is asking a question")));
        const task = State.focusTask;
        title.textContent = (task && lastTextStep(task.steps)) ?? t("Claude needs an answer.");
        if (rowKey !== "terminal") {
          rowKey = "terminal";
          clear(row);
          row.append(h("div", { class: "sub", text: tl("Answer it in your terminal.") }));
        }
        return;
      }

      if (State.pendingApproval!.requestId !== requestId) {
        requestId = State.pendingApproval!.requestId;
        index = 0;
        answers = {};
        picked = new Set();
      }
      const q = questions[Math.min(index, questions.length - 1)];
      const asking = questions.length > 1
        ? t("is asking ({index} of {total})", { index: index + 1, total: questions.length })
        : t("is asking");
      who.append(agentWho(State.focusTask, asking));
      title.textContent = q.question;
      title.title = q.question;

      const key = `${requestId}:${index}:${[...picked].join("|")}`;
      if (rowKey === key) return;
      rowKey = key;
      clear(row);
      for (const option of q.options) {
        const on = picked.has(option.label);
        const button = btn(option.label, on ? "primary" : "secondary", () => {
          if (!q.multiSelect) {
            next(q.question, option.label, questions.length);
            return;
          }
          if (on) picked.delete(option.label);
          else picked.add(option.label);
          State.notify();
        });
        if (option.description) button.title = option.description;
        row.append(button);
      }
      if (q.multiSelect) {
        const done = btn(tl("Done"), "primary", () => {
          if (picked.size > 0) next(q.question, [...picked], questions.length);
        });
        if (picked.size === 0) done.classList.add("off");
        row.append(done);
      }
      row.append(
        h("button", {
          class: "link-btn",
          style: "color:#8e939c",
          text: tl("Answer in terminal"),
          onclick: () => actions.answerInTerminal(),
        }),
      );
    },
  };
}

// ── Error ─────────────────────────────────────────────────────────────────────

/**
 * What the error card's second button opens for this task, or null for none:
 * the session's terminal window (Open terminal, the same target as the
 * finished card), the Claude desktop app for its sessions, and nothing for
 * the local chat pills (source "n8n"), which have no window to go to.
 */
export function errorOpenLabel(task: AgentTask | null): string | null {
  if (!task || task.source === "n8n") return null;
  return task.id === "agent_claude-desktop" ? t("Open Claude") : t("Open terminal");
}

/**
 * The card's title for an agent stopped by a usage or spend limit: when it
 * resets if Claude Code said so ("Usage limit, resets 2 PM"), and that it has
 * reset once the time has passed.
 */
export function limitTitle(limit: AgentLimit, now: number): string {
  if (limitHasReset(limit, now)) return t("The limit has reset. Continue in the terminal.");
  const usage = limit.kind === "usage";
  if (limit.resets) {
    return usage
      ? t("Usage limit, resets {time}", { time: limit.resets })
      : t("Spend limit, resets {time}", { time: limit.resets });
  }
  return usage ? t("Usage limit") : t("Spend limit");
}

export function buildError(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title" });
  const detail = h("div", { class: "detail" });
  const retry = btn(tl("Retry"), "primary", () => actions.setView(State.defaultView()));
  const open = btn(tl("Open terminal"), "secondary", () => actions.openTerminal());
  // A limit is not an error: no Retry (nothing can run before the reset), and
  // OK to put the card away.
  const ok = btn(tl("OK"), "secondary", () => actions.collapse());
  const row = h("div", { class: "actions" }, retry, open, ok);
  const box = card("red", stack(116, 16, who, title, detail, row));
  const el = h("div", { class: "view" }, box);
  return {
    el,
    sync() {
      const task = State.focusTask;
      const limit = task?.state === "ratelimit" ? task.limit ?? null : null;
      box.style.setProperty("--wash", washRGBA(limit ? "amber" : "red"));
      retry.style.display = limit ? "none" : "";
      ok.style.display = limit ? "" : "none";
      if (limit) {
        clear(who);
        who.append(agentWho(task, task?.source === "agent" ? t("Agent") : "Claude Code"));
        title.textContent = limitTitle(limit, Date.now());
        detail.textContent = t("It can't work until the limit resets.");
        open.className = "btn primary";
        const span = open.children[0] as HTMLElement;
        const label = errorOpenLabel(task) ?? t("Open terminal");
        if (span.textContent !== label) span.textContent = label;
        open.style.display = errorOpenLabel(task) ? "" : "none";
        return;
      }
      open.className = "btn secondary";
      clear(who);
      // agentWho already shows an agent's name: its label is just the kind.
      const whoLabel = task?.source === "n8n" ? "n8n" : task?.source === "agent" ? t("Agent") : "Claude Code";
      who.append(agentWho(task, whoLabel));
      title.textContent = task?.source === "n8n" ? t("Workflow stopped.") : t("Session stopped on an error.");
      detail.textContent = (task && lastTextStep(task.steps)) ?? t("No detail available.");
      const label = errorOpenLabel(task);
      open.style.display = label ? "" : "none";
      const span = open.children[0] as HTMLElement;
      if (label && span.textContent !== label) span.textContent = label;
    },
  };
}

// ── Finished ──────────────────────────────────────────────────────────────────

function buildFinished(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title one-line" });
  const open = btn(tl("Open terminal"), "primary", () => actions.openTerminal());
  const row = h("div", { class: "actions" },
    open,
    btn(tl("OK"), "secondary", () => actions.collapse()),
  );
  const el = h("div", { class: "view" }, card("green", stack(116, 16, who, title, row)));
  return {
    el,
    sync() {
      clear(who);
      const agent = State.focusTask?.source === "agent";
      who.append(agentWho(State.focusTask, agent ? t("finished") : t("Claude Code finished")));
      // The final message, else the last step that is not a diff (FinishedView).
      const task = State.focusTask;
      title.textContent = task?.finalLine || (task && lastTextStep(task.steps)) || t("Session finished");
      // Sessions from the Claude desktop app live there, not in a terminal.
      const label = task?.id === "agent_claude-desktop" ? t("Open Claude") : t("Open terminal");
      const span = open.firstElementChild as HTMLElement;
      if (span.textContent !== label) span.textContent = label;
    },
  };
}

// ── Confused ──────────────────────────────────────────────────────────────────

function buildConfused(): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 128px" },
    h("div", { class: "title", text: tl("Too many hits at once.") }),
    h("div", { class: "sub", text: tl("Give me a sec — back to work in three seconds.") }),
  );
  return { el: h("div", { class: "view" }, card("pink", body)), sync() {} };
}

// ── Note ──────────────────────────────────────────────────────────────────────

function buildNote(): ViewHost {
  const title = h("div", { class: "title" });
  const el = h("div", { class: "view" }, card(null, h("div", { class: "stack", style: "padding:0 18px 0 98px" }, title)));
  return {
    el,
    sync() {
      title.textContent = State.noteMessage ?? "";
    },
  };
}

// ── Writing suggestions (1a list, 1b fixes) ───────────────────────────────────

/** Suggestions shown at once; Previous / Next page through the rest. */
export const SUGGESTIONS_SHOWN = 3;

/**
 * Which suggestions a page shows: `page` clamped to the pages there are,
 * and `from`-`to` (1-based, for "1–3 of 12"). Paging is by buttons only: the
 * island's window never takes focus, so the mouse wheel can't be relied on.
 */
export function suggestionPage(total: number, page: number): { page: number; pages: number; from: number; to: number } {
  const pages = Math.max(1, Math.ceil(total / SUGGESTIONS_SHOWN));
  const p = Math.min(Math.max(0, page), pages - 1);
  const from = total === 0 ? 0 : p * SUGGESTIONS_SHOWN + 1;
  return { page: p, pages, from, to: Math.min(total, (p + 1) * SUGGESTIONS_SHOWN) };
}

/**
 * The note under the list: "Fixing…" while a fix is on its way, the last
 * click's outcome while it is fresh, otherwise what a click does here. After
 * Apply all: how many went in ("Fixed 2 of 3"), that Ctrl+Z takes back one
 * per press, and why it stopped if it did.
 */
export function suggestionNote(fix: WritingFix, result: WritingResult | null, busy: boolean): string {
  if (busy) return t("Fixing…");
  const app = result?.app || fix.app;
  if (result?.total) {
    const counts = { done: result.done ?? 0, total: result.total, app };
    const fixed = counts.done > 0
      ? t("Fixed {done} of {total}. Ctrl+Z in {app} undoes one fix per press.", counts)
      : t("Fixed {done} of {total}.", counts);
    const left = result.skipped ? ` ${t("Left {skipped} for you to pick: the best option isn't clear.", { skipped: result.skipped })}` : "";
    return result.outcome === "applied" ? `${fixed}${left}` : `${fixed}${left} ${stopNote(result.outcome, app)}`;
  }
  return outcomeNote(result?.outcome, app) ?? modeNote(fix);
}

/** Why Apply all stopped, in plain words (owner, 2026-10-12). */
function stopNote(outcome: string, app: string): string {
  switch (outcome) {
    case "changed": return t("It stopped because the text changed after the check. Checked again.");
    case "away": return t("It stopped because the cursor left the field.");
    case "paused": return t("It stopped because Glim was paused.");
    case "blocked": return t("It stopped because that field is protected.");
    case "elevated": return t("It stopped because {app} runs as administrator, and Glim can't type there.", { app });
    case "keys-held": return t("It stopped because a key was held down. Release it and try again.");
    case "unsupported": return t("It stopped because {app} didn't take the next fix, so nothing more changed.", { app });
    case "not-clean": return t("It stopped because {app} changed the next fix as it was typed (its AutoCorrect, or a text expander). Check that word: Ctrl+Z undoes it.", { app });
  }
  return outcomeNote(outcome, app) ?? "";
}

/** A click's outcome in words; undefined for none. */
export function outcomeNote(outcome: string | undefined, app: string): string | undefined {
  switch (outcome) {
    case "applied": return t("Fixed. Ctrl+Z in {app} undoes it.", { app });
    case "copied": return t("Copied. Paste it with Ctrl+V.");
    case "changed": return t("The text changed, so nothing was fixed. Checked again.");
    case "away": return t("Click in the field first.");
    case "paused": return t("Glim is paused.");
    case "blocked": return t("Nothing was fixed: that field is protected.");
    case "elevated": return t("Glim can't type into apps running as administrator.");
    case "keys-held": return t("Release the keys you're holding, then try again.");
    case "unsupported": return t("{app} didn't take the fix, so nothing changed.", { app });
    case "not-clean": return t("That didn't apply cleanly. Press Ctrl+Z in {app} to undo it.", { app });
    case "unclear": return t("Nothing was applied: no suggestion has a clearly best option. Pick one yourself.");
    case "clipboard-busy": return t("Couldn't copy: another app is using the clipboard.");
  }
  return undefined;
}

/** What a click does in this field. */
function modeNote(fix: WritingFix): string {
  switch (fix.mode) {
    case "apply": return t("Click a fix to apply it. Ctrl+Z in {app} undoes it.", { app: fix.app });
    case "away": return t("Click in the field first.");
    case "copy": return t("Glim can't type in this field. Click a fix to copy it.");
    default: return t("Suggestions only in {app} for now.", { app: fix.app });
  }
}

/** How long "Fixing…" waits for Rust's answer before the buttons come back
 * (per fix: Apply all waits this long for each). */
const FIX_TIMEOUT_MS = 3000;

/**
 * The instant checker's suggestions for the focused field: the problem in a
 * little context, Harper's explanation (English: the checker is en-US only),
 * and its replacements. A replacement is a button where fixes are typed or
 * copied (1b); a click sends only which list, row and replacement it was.
 * Rust types or copies the checker's own words. Nothing here is logged.
 */
export function buildSuggestions(actions?: ViewActions): ViewHost {
  // The page being shown, and the list it belongs to: a new list (a new
  // check) starts again at the first page.
  let page = 0;
  let pageOf: WritingSuggestion[] | null = null;
  const title = h("div", { class: "title one-line" });
  // Apply all: each row's first replacement, typed by Rust last to first.
  const allTip = tl("Applies each suggestion's first option, last to first, and skips any whose best option isn't clear. Ctrl+Z undoes one fix per press.");
  const applyAll = h("button", { class: "apply-all", type: "button", title: allTip, "aria-label": allTip, text: tl("Apply all"), onclick: () => fixAll() }) as HTMLButtonElement;
  // 1c: the Rewrite card for this paragraph (nothing is sent until a tone is picked).
  const rewriteTip = tl("Rewrite this paragraph with Claude (Ctrl+Alt+Shift+R)");
  const rewrite = h("button", { class: "rewrite-open", type: "button", title: rewriteTip, "aria-label": rewriteTip, text: tl("Rewrite…"), onclick: () => openRewrite() });
  const openRewrite = () => {
    openToneCard();
    actions?.setView("rewrite");
  };
  const head = h("div", { class: "suggestion-head" }, title, rewrite, applyAll);
  const list = h("div", { class: "suggestion-list" });
  const range = h("span", { class: "suggestion-range" });
  const prev = h("button", { class: "suggestion-page", title: tl("Previous suggestions"), "aria-label": tl("Previous suggestions"), onclick: () => turn(-1) }, svg(ICONS.chevronLeft, 16, { stroke: 2.6 }));
  const next = h("button", { class: "suggestion-page", title: tl("Next suggestions"), "aria-label": tl("Next suggestions"), onclick: () => turn(1) }, svg(ICONS.chevronRight, 16, { stroke: 2.6 }));
  const pages = h("div", { class: "suggestion-pages" }, prev, range, next);
  const note = h("div", { class: "suggestion-more", role: "status" });
  const dismiss = h("button", { class: "suggestion-dismiss", title: tl("Dismiss suggestions"), "aria-label": tl("Dismiss suggestions"), onclick: () => void Bridge.writingDismiss() }, svg(ICONS.xmark, 12));
  // §15.1: this paragraph to Claude (Haiku), for what Harper can't see.
  const aiTip = tl("Send this paragraph to Claude to find grammar and meaning errors the checker misses");
  const aiCheck = h("button", { class: "ai-check", type: "button", title: aiTip, "aria-label": aiTip, text: tl("Check with AI"), onclick: () => checkWithAi() }) as HTMLButtonElement;
  const checkWithAi = () => {
    if (aiSending(State.aiCheck)) return;
    void Bridge.writingAiCheck();
  };
  const pager = h("div", { class: "suggestion-pager" }, pages, note, aiCheck, dismiss);
  const el = h("div", { class: "view" }, card(null, stack(98, 16, head, list, pager)));
  const sync = () => {
    const items = State.writing;
    if (items !== pageOf) {
      pageOf = items;
      page = 0;
    }
    const at = suggestionPage(items.length, page);
    page = at.page;
    title.textContent = items.length > 0
      ? tn("{count} writing suggestion", "{count} writing suggestions", items.length)
      : t("Nothing to fix here.");
    clear(list);
    items.slice(at.from - 1, at.to).forEach((s, i) => list.append(suggestionRow(s, at.from - 1 + i, pick)));
    pages.style.display = at.pages > 1 ? "" : "none";
    range.textContent = t("{from}–{to} of {total}", { from: at.from, to: at.to, total: items.length });
    (prev as HTMLButtonElement).disabled = at.page === 0;
    (next as HTMLButtonElement).disabled = at.page === at.pages - 1;
    // A fix on its way or just done says so first; then Check with AI's.
    const fixNote = suggestionNote(State.writingFix, State.writingResult, State.writingBusy);
    note.textContent = State.writingBusy || State.writingResult ? fixNote : aiCheckNote(State.aiCheck) || fixNote;
    aiCheck.disabled = aiSending(State.aiCheck) || State.writingBusy;
    // Where fixes are typed, and only for two or more.
    const mode = State.writingFix.mode;
    applyAll.style.display = (mode === "apply" || mode === "away") && items.length > 1 ? "" : "none";
    applyAll.disabled = mode === "away" || State.writingBusy;
  };
  const turn = (by: number) => {
    page += by;
    sync();
  };
  // "Fixing…" until Rust answers, or until `fixes` fixes' time runs out.
  const wait = (fixes: number) => {
    State.writingBusy = true;
    State.writingResult = null;
    sync();
    setTimeout(() => {
      if (!State.writingBusy) return;
      State.writingBusy = false;
      sync();
    }, FIX_TIMEOUT_MS * fixes);
  };
  const fixAll = () => {
    const fix = State.writingFix;
    if (State.writingBusy || fix.mode !== "apply") return;
    wait(State.writing.length);
    void Bridge.writingFixAll(fix.checkId);
  };
  // A replacement was clicked: row `index` of the whole list, replacement `choice`.
  const pick = (index: number, choice: number) => {
    const fix = State.writingFix;
    if (State.writingBusy) return;
    if (fix.mode === "apply") {
      wait(1);
      void Bridge.writingFix(fix.checkId, index, choice);
    } else if (fix.mode === "copy") {
      void Bridge.writingCopy(fix.checkId, index, choice);
    }
  };
  return { el, sync };
}

function suggestionRow(s: WritingSuggestion, index: number, pick: (index: number, choice: number) => void): HTMLElement {
  const context = h(
    "div",
    { class: "suggestion-context" },
    h("span", { class: "ctx", text: s.before.replace(/\s+/g, " ") }),
    h("span", { class: "problem", text: s.problem }),
    h("span", { class: "ctx", text: s.after.replace(/\s+/g, " ") }),
  );
  const mode = State.writingFix.mode;
  const fixes = h("div", { class: "suggestion-fixes" });
  s.replacements.forEach((r, choice) => {
    const label = r === "" ? t("Remove it") : r;
    // View-only apps; and nothing to copy for a removal.
    if (mode === "view" || (mode === "copy" && r === "")) {
      fixes.append(h("span", { class: "fix", text: label }));
      return;
    }
    const aria = r === "" ? t("Remove it") : mode === "copy" ? t("Copy “{fix}”", { fix: r }) : t("Replace with “{fix}”", { fix: r });
    const button = h(
      "button",
      { class: mode === "copy" ? "fix fix-btn copy" : "fix fix-btn", type: "button", title: aria, "aria-label": aria, onclick: () => pick(index, choice) },
      ...(mode === "copy" ? [svg(ICONS.copy, 11)] : []),
      h("span", { text: label }),
    ) as HTMLButtonElement;
    button.disabled = mode === "away" || State.writingBusy;
    fixes.append(button);
  });
  // The AI's own findings (§15.1) say so.
  const message = h("div", { class: "suggestion-message" },
    ...(s.kind === AI_KIND ? [h("span", { class: "ai-badge", title: t("Found by Claude (Check with AI)"), text: t("AI") })] : []),
    h("span", { text: s.message }),
  );
  return h("div", { class: "suggestion" }, context, message, fixes);
}

// ── In-island settings ────────────────────────────────────────────────────────

function buildSettings(actions: ViewActions): ViewHost {
  const autoLabel = h("span", {});
  const segButtons = [10, 15, 30].map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, `${s}s`),
  );
  const claudeBadge = h("span", { class: "status-badge" });

  const rows = h(
    "div",
    { class: "settings-rows" },
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      autoLabel,
      h("div", { class: "seg" }, ...segButtons),
    ),
    h(
      "div",
      { class: "settings-row", style: "gap:14px" },
      claudeBadge,
      h("div", { class: "grow" }),
      h("button", {
        class: "link-btn",
        style: "color:#8e939c;font-size:11.5px",
        text: tl("Settings…"),
        onclick: () => actions.openSettingsWindow(),
      }),
    ),
  );

  const el = h("div", { class: "view" },
    card(null, h("div", { class: "stack", style: "padding:14px 16px 14px 84px" }, rows)));

  return {
    el,
    sync() {
      const s = State.settings;
      autoLabel.textContent = t("Auto-close · {seconds}s", { seconds: Math.round(s.autoCloseInterval) });
      segButtons.forEach((b, i) => b.classList.toggle("on", s.autoCloseInterval === [10, 15, 30][i]));
      clear(claudeBadge);
      claudeBadge.append(
        dot(s.hooksInstalled ? "#22C55E" : "#F4505E", 6),
        h("span", { text: "Claude Code" }),
      );
    },
  };
}

// ── Placeholders filled in later stages ───────────────────────────────────────

function buildPlaceholder(title: Msg, sub: string): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px" },
    h("div", { class: "title", text: title }),
    h("div", { class: "sub", text: sub }),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Registry ──────────────────────────────────────────────────────────────────

export function buildViews(
  actions: ViewActions,
  onChatHeightChange: () => void,
): Map<IslandViewName, ViewHost> {
  const map = new Map<IslandViewName, ViewHost>();
  map.set("overview", buildOverview(actions));
  map.set("empty", buildEmpty(actions));
  map.set("approval", buildApproval(actions));
  map.set("question", buildQuestion(actions));
  map.set("error", buildError(actions));
  map.set("finished", buildFinished(actions));
  map.set("confused", buildConfused());
  map.set("note", buildNote());
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  map.set("recap", buildRecap(actions));
  map.set("suggestions", buildSuggestions(actions));
  map.set("rewrite", buildRewrite(actions));
  map.set("writingAsk", buildAsk(actions));
  map.set("resume", buildResume(actions));
  map.set("writingDocsTip", buildDocsTip(actions));
  map.set("voice", buildVoice(actions));
  map.set("draft", buildDraft(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(tl("Sending by email isn't in this version."), ""));
  map.set("searching", buildPlaceholder(tl("Claude is searching…"), ""));
  map.set("result", buildPlaceholder(tl("Result"), ""));
  return map;
}
