// Entry point: boot the bridge, wire the island, light the lantern.

import "./style.css";
import "./mascot/lantern.css";
import { isDock } from "./core/layout";
import { Bridge, onEvent } from "./core/bridge";
import { State, type Settings, type WritingFix, type WritingResult, type WritingSuggestion } from "./core/state";
import { Island } from "./island/island";
import { nextRewrite, type RewriteEvent } from "./core/rewrite";
import { nextAiCheck, type AiCheckEvent } from "./core/ai-check";
import { askPlacement, type WritingAsk } from "./core/ask";
import { answerAsk, askClock } from "./views/ask";
import { docsTipPlacement } from "./core/docs-tip";
import { closeDocsTip } from "./views/docs-tip";
import { needsCard, nextVoice, running, voicePlacement, type VoiceEvent } from "./core/voice";
import { voiceShown } from "./views/voice";
import { isLanternState } from "./mascot/lantern";
import { PresenceView, isPresenceKind } from "./island/presence";
import { registerHookHandlers } from "./island/hooks";
import { refreshConfigured } from "./island/pill-status";
import { registerShortcutHandlers } from "./island/shortcuts";
import { Recap } from "./recap/recap";
import { onLanguageChange, resolveLanguage, setLanguage, systemLanguages, t } from "./i18n/i18n";

/** Shows the language Settings asks for ("" = the system's, when Glim has it). */
function applyLanguage() {
  setLanguage(resolveLanguage(State.settings.language, systemLanguages()));
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  const island = new Island(root);
  /** Reads the repos' HANDOFF.md files (Rust) and opens the resume card. */
  const showResume = async () => {
    State.resume = (await Bridge.resumeCards()) ?? [];
    island.alert("resume");
    State.notify();
  };
  // Pill: the compact island on screen (it may have been folded or paused).
  const presence = new PresenceView({
    showPill: () => island.reveal(),
    // The ember's click with suggestions waiting: straight to the list.
    openSuggestions: () => island.setView(State.lanternView() ?? "suggestions"),
  });
  // A peek from the ember ends when the island closes: back to the ember.
  island.onClosed = () => presence.model.islandClosed();
  root.append(presence.el);

  const askHost = { setView: (v: Parameters<typeof island.setView>[0]) => island.setView(v), collapse: () => island.collapse() };

  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  // A language change redraws the island in place: the texts given as tl(…)
  // relabel themselves (views/dom.ts) and the views redraw the rest on this
  // sync. Nothing is rebuilt, so tasks, steps and the chat stay as they are.
  onLanguageChange(() => State.notify());
  applyLanguage();
  // Rust shows the tray and its errors in the same language as the webview.
  void Bridge.setSystemLanguages(systemLanguages());
  island.applySettings();
  State.loadIntegrationTasks();
  if (boot && !boot.cursorPoll) island.followPageCursor();
  await island.desktop.init();

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));
  await onEvent<boolean>("pointer-inside", (inside) => island.setPointerInside(inside));

  // Pause (1d, §14.6) is Rust's: the tray's Pause / Resume and the pause key
  // (pause.rs). It stops the writing features only; Glim stays where it is,
  // greyed with a pause badge, and agents carry on.
  await onEvent<boolean>("paused", (on) => {
    State.paused = on;
    presence.setPaused(on);
    // A question about an app waits for another day.
    if (on && State.writingAsk) answerAsk("later", askHost);
    if (on && State.writingDocsTip) closeDocsTip(false, askHost);
    State.notify();
  });

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        island.alert("settings");
        break;
      case "open":
        island.alert(State.defaultView());
        break;
      case "recap":
        void Recap.open(island);
        break;
      case "resume":
        void showResume();
        break;
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());
  // The dock on the island's display: a drop of the pill, Settings, `--dock`.
  await onEvent<{ dock: string }>("placement", ({ dock }) => {
    if (isDock(dock)) island.setDock(dock);
  });
  const placed = await Bridge.placement();
  if (placed && isDock(placed.dock)) island.setDock(placed.dock);
  // Pill, ember, hidden or the recording indicator (presence.rs decides).
  await onEvent<{ kind: string }>("presence", ({ kind }) => {
    if (!isPresenceKind(kind)) return;
    if (kind !== "pill") island.closeMenu();
    presence.apply(kind);
  });
  const shown = await Bridge.presenceInfo();
  if (shown && isPresenceKind(shown.kind)) presence.apply(shown.kind);
  // Dev sessions only: `glim.exe --mascot-state <state|auto>` (see lib.rs).
  await onEvent<string>("mascot-force", (s) => island.forceLanternState(isLanternState(s) ? s : null));
  // Dev sessions only: `glim.exe --dev-chat "<prompt>"` (see lib.rs). Connects
  // Ollama exactly as Settings → Connect does when it isn't yet, then sends the
  // prompt through the chat view's own input and Send button (DOM events inside
  // this webview — nothing reaches the desktop).
  await onEvent<string>("dev-chat", async (prompt) => {
    const settings = State.settings;
    if (!settings.ollamaUrl) {
      const server = await Bridge.localConnect("ollama", "");
      if (!server.models.length) return;
      settings.ollamaUrl = server.url;
      if (!server.models.includes(settings.chatModels.ollama ?? "")) {
        settings.chatModels = { ...settings.chatModels, ollama: server.models[0] };
      }
      settings.chatProvider = "ollama";
      await Bridge.saveSettings(settings);
    }
    island.alert("prompt");
    await new Promise((r) => setTimeout(r, 600));
    const input = document.querySelector<HTMLInputElement>(".chat-bar input");
    const send = document.querySelector<HTMLButtonElement>(".chat-bar .send-btn");
    if (!input || !send) return;
    input.value = prompt;
    send.click();
  });

  // Dev sessions only: `glim.exe --dev-fix "<replacement>"` (see lib.rs) opens
  // the list and clicks that fix button (DOM events inside this webview, like
  // a mouse click on the non-activating island; nothing reaches the desktop).
  await onEvent<string>("dev-fix", (fix) => {
    island.alert("suggestions");
    window.setTimeout(() => {
      const label = t("Replace with “{fix}”", { fix });
      const button = [...document.querySelectorAll<HTMLButtonElement>("button.fix-btn")].find((b) => b.getAttribute("aria-label") === label);
      button?.click();
    }, 600);
  });

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    const previousMain = State.mainPillId;
    State.settings = { ...State.settings, ...s };
    applyLanguage();
    island.applySettings();
    State.loadIntegrationTasks();
    // A new main tool comes to the front, as on macOS.
    if (State.mainPillId !== previousMain) State.setFocus(State.mainPillId);
    void refreshConfigured();
  });

  registerHookHandlers(island);
  void refreshConfigured();
  // Opening Glim from a key no longer lifts Pause: only Resume or the pause key do (1d).
  registerShortcutHandlers(island, () => {});

  // Monday recap: app start (ignite over), an agent starting work, waking up.
  const checkRecap = () => void Recap.check(island);
  island.onGreetingDone = checkRecap;
  island.onWake = checkRecap;
  await onEvent<null>("recap-check", checkRecap);
  // The writing checker's suggestions for the focused field (Phase 1a).
  await onEvent<{ items: WritingSuggestion[]; checkable?: boolean } & WritingFix>("writing-suggestions", ({ items, checkId, mode, app, checkable }) => {
    const had = State.writing.length > 0;
    State.writing = items;
    State.writingCheckable = checkable === true;
    State.writingFix = { checkId, mode, app };
    // A finished "Check with AI" note belongs to the list before this one
    // (Rust sends the AI's list just before its "done").
    if (State.aiCheck && State.aiCheck.phase !== "sending") State.aiCheck = null;
    presence.setWriting(items.length);
    // The list emptied while it was open: back to what the island shows.
    // Not when it emptied because a fix just worked: the list stays to say so
    // ("Fixed. Ctrl+Z in Notepad undoes it.").
    if (had && items.length === 0 && State.view === "suggestions" && State.writingResult?.outcome !== "applied") State.view = State.defaultView();
    State.notify();
  });
  // A fix or Copy click's outcome (a code; the island words it).
  let resultTimer: ReturnType<typeof setTimeout> | undefined;
  await onEvent<WritingResult>("writing-fix", (result) => {
    State.writingBusy = false;
    State.writingResult = result;
    clearTimeout(resultTimer);
    resultTimer = setTimeout(() => {
      State.writingResult = null;
      State.notify();
    }, 6000);
    State.notify();
  });

  // The Rewrite card (1c): Rust's answer to a tone click, and what "Use
  // this" or "Copy" did. Stored and redrawn, nothing else.
  // "Check with AI" (core/ai-check.ts): the lantern delegates while it runs.
  await onEvent<AiCheckEvent>("writing-ai-check", (e) => {
    State.aiCheck = nextAiCheck(e);
    State.notify();
  });

  await onEvent<RewriteEvent>("writing-rewrite", (e) => {
    State.rewrite = nextRewrite(State.rewrite, e);
    State.notify();
  });

  // "Check writing in [app]?" (1d, §14.2): Rust asks after 2 s in a field of
  // an app on neither list. Seen now, in the ember, behind an approval card,
  // or not at all (Hidden, fullscreen): see core/ask.ts.
  State.subscribe(() => presence.setAsk(State.writingAsk !== null));
  // "Pick up where you left off" (views/resume.ts). Shown on request only
  // (decided 2026-10-15): the tray's item, the voice action `resume_work`
  // (1.5c), and the dev switch `--resume-card`. Never by itself. Nothing
  // opens without a click on a row.
  await onEvent<null>("resume-show", () => void showResume());

  await onEvent<WritingAsk>("writing-ask", (ask) => {
    const where = askPlacement({
      presence: presence.model.kind,
      paused: State.paused,
      approval: State.pendingApproval !== null,
      current: State.writingAsk,
      incoming: ask,
    });
    if (where === "drop" || where === "keep") return;
    State.writingAsk = { exe: ask.exe, name: ask.name };
    void Bridge.writingAskShown(ask.exe);
    if (where === "show") island.alert("writingAsk");
    // In the ember the "?" is what is in sight: the 30 s run from now.
    if (where === "ember") askClock.start(ask.exe, () => answerAsk("later", askHost));
    State.notify();
  });

  // The Google Docs tip (core/docs-tip.ts): Rust offers it once per run when
  // a Docs field gives no text to check.
  await onEvent<null>("writing-docs-tip", () => {
    const where = docsTipPlacement({
      presence: presence.model.kind,
      paused: State.paused,
      approval: State.pendingApproval !== null,
      ask: State.writingAsk !== null,
      showing: State.writingDocsTip,
    });
    if (where === "drop") return;
    State.writingDocsTip = true;
    if (where === "show") island.alert("writingDocsTip");
    State.notify();
  });

  // Push-to-talk (voice 1.5b, core/voice.ts): Rust's phases. While the key
  // is held the mic level comes ~10 times a second; the lantern reads it each
  // frame, so those need no redraw.
  await onEvent<VoiceEvent>("voice", (e) => {
    const was = State.voice;
    State.voice = nextVoice(State.voice, e);
    if (e.phase === "listening" && was?.phase === "listening") return;
    const v = State.voice;
    if (v && !running(v)) {
      const where = voicePlacement({ presence: presence.model.kind, approval: State.pendingApproval !== null, v });
      if (where === "drop") State.voice = null;
      else {
        voiceShown();
        if (where === "show") island.alert("voice");
      }
    }
    // A result that needed no card: the lantern goes back.
    if (State.voice && !running(State.voice) && !needsCard(State.voice)) State.voice = null;
    State.notify();
  });

  island.launch();
}

void main();
