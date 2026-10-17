// The "Check writing in [app]?" card (1d, docs/phase1-plan.md §14.2): Yes,
// Not now and Never. A click sends the app's exe and the answer, nothing
// else. The island never takes focus, so the user can keep typing and answer
// later; the card closes by itself after 30 s, as "Not now".

import { h } from "./dom";
import { card, stack, type ViewActions, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { AskClock, type AskAnswer } from "../core/ask";
import { t, type Msg } from "../i18n/i18n";

/** Runs while the question is in sight (the card, or the ember's "?"). */
export const askClock = new AskClock();

/** What the island does once the question is answered. */
export interface AskHost {
  setView(v: Parameters<ViewActions["setView"]>[0]): void;
  collapse(): void;
}

/** Sends the answer and closes the card. */
export function answerAsk(answer: AskAnswer, host: AskHost | null) {
  const ask = State.writingAsk;
  if (!ask) return;
  askClock.stop();
  State.writingAsk = null;
  void Bridge.writingAskAnswer(ask.exe, answer);
  if (host && State.view === "writingAsk" && State.mode === "expanded") {
    // Back to what else is waiting, or out of the way.
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

export function buildAsk(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title one-line" });
  const note = h("div", { class: "suggestion-more", role: "status" });
  const button = (label: Msg | string, answer: AskAnswer, cls: string) =>
    h("button", { class: cls, type: "button", text: label, "data-answer": answer, onclick: () => answerAsk(answer, actions) }) as HTMLButtonElement;
  const yes = button(t("Yes"), "yes", "apply-all");
  const later = button(t("Not now"), "later", "link-btn");
  const never = button(t("Never"), "never", "link-btn");
  const row = h("div", { class: "rewrite-actions" }, yes, later, never);
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, note, row)));

  const sync = () => {
    const ask = State.writingAsk;
    if (!ask) return;
    title.textContent = t("Check writing in {app}?", { app: ask.name });
    note.textContent = t("Glim would check what you type there. Nothing is read until you say yes.");
    yes.textContent = t("Yes");
    later.textContent = t("Not now");
    never.textContent = t("Never");
    // In sight now: the 30 s run from here.
    if (State.mode === "expanded") askClock.start(ask.exe, () => answerAsk("later", actions));
  };
  return { el, sync };
}
