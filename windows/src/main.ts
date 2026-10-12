// Entry point: boot the bridge, wire the island, light the lantern.

import "./style.css";
import "./mascot/lantern.css";
import { isDock } from "./core/layout";
import { Bridge, onEvent } from "./core/bridge";
import { State, type Settings, type WritingFix, type WritingSuggestion } from "./core/state";
import { Island } from "./island/island";
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
  // Pill: the compact island on screen (it may have been folded or paused).
  const presence = new PresenceView({
    showPill: () => { if (!State.paused) island.reveal(); },
    // The ember's click with suggestions waiting: straight to the list.
    openSuggestions: () => island.setView(State.lanternView() ?? "suggestions"),
  });
  // A peek from the ember ends when the island closes: back to the ember.
  island.onClosed = () => presence.model.islandClosed();
  root.append(presence.el);

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

  const setPaused = (on: boolean) => {
    State.paused = on;
    // Rust stops reading fields, checking and typing fixes while paused.
    void Bridge.writingSetPaused(on);
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "recap":
        setPaused(false);
        void Recap.open(island);
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
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
  registerShortcutHandlers(island, () => setPaused(false));

  // Monday recap: app start (ignite over), an agent starting work, waking up.
  const checkRecap = () => void Recap.check(island);
  island.onGreetingDone = checkRecap;
  island.onWake = checkRecap;
  await onEvent<null>("recap-check", checkRecap);
  // The writing checker's suggestions for the focused field (Phase 1a).
  await onEvent<{ items: WritingSuggestion[] } & WritingFix>("writing-suggestions", ({ items, checkId, mode, app }) => {
    const had = State.writing.length > 0;
    State.writing = items;
    State.writingFix = { checkId, mode, app };
    presence.setWriting(items.length);
    // The list emptied while it was open: back to what the island shows.
    // Not when it emptied because a fix just worked: the list stays to say so
    // ("Fixed. Ctrl+Z in Notepad undoes it.").
    if (had && items.length === 0 && State.view === "suggestions" && State.writingResult?.outcome !== "applied") State.view = State.defaultView();
    State.notify();
  });
  // A fix or Copy click's outcome (a code; the island words it).
  let resultTimer: ReturnType<typeof setTimeout> | undefined;
  await onEvent<{ outcome: string; app: string }>("writing-fix", (result) => {
    State.writingBusy = false;
    State.writingResult = result;
    clearTimeout(resultTimer);
    resultTimer = setTimeout(() => {
      State.writingResult = null;
      State.notify();
    }, 6000);
    State.notify();
  });

  island.launch();
}

void main();
