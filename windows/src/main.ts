// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import "./mascot/lantern.css";
import { Bridge, onEvent } from "./core/bridge";
import { State, type Settings } from "./core/state";
import { Island } from "./island/island";
import { isLanternState } from "./mascot/lantern";
import { registerHookHandlers } from "./island/hooks";
import { refreshConfigured } from "./island/pill-status";
import { registerShortcutHandlers } from "./island/shortcuts";
import { Recap } from "./recap/recap";
import { onLanguageChange, resolveLanguage, setLanguage, systemLanguages } from "./i18n/i18n";

/** Shows the language Settings asks for ("" = the system's, when Glim has it). */
function applyLanguage() {
  setLanguage(resolveLanguage(State.settings.language, systemLanguages()));
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  const island = new Island(root);

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

  // Monday recap: app start (greeting over), an agent starting work, waking up.
  const checkRecap = () => void Recap.check(island);
  island.onGreetingDone = checkRecap;
  island.onWake = checkRecap;
  await onEvent<null>("recap-check", checkRecap);

  island.launch();
}

void main();
