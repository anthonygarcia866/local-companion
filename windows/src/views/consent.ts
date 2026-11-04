// The ask-first card (core/consent.ts; Rust: consent.rs): the first time a
// cloud feature would send by itself, what it sends and "Turn on" / "Not
// now". Nothing was sent; nothing is until "Turn on". The card stays until
// answered (no timer: the answer is the user's to give).

import { h, clear } from "./dom";
import { card, stack, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { setConsent, SENT_FOOTER, WHAT_IS_SENT } from "../core/consent";
import type { AskHost } from "./ask";
import { t } from "../i18n/i18n";

/** Sends the answer, stores it in the island's copy, and closes the card. */
export function answerConsent(on: boolean, host: AskHost | null) {
  const f = State.consentAsk;
  if (!f) return;
  State.consentAsk = null;
  setConsent(State.settings, f, on);
  void Bridge.consentAnswer(f, on);
  if (host && State.view === "consentAsk" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

export function buildConsent(host: AskHost): ViewHost {
  const title = h("div", { class: "title one-line" });
  const list = h("ul", { class: "consent-list", "data-role": "consent-sent" });
  const note = h("div", { class: "suggestion-more" });
  const on = h("button", { class: "apply-all", type: "button", "data-answer": "on", onclick: () => answerConsent(true, host) }) as HTMLButtonElement;
  const later = h("button", { class: "link-btn", type: "button", "data-answer": "off", onclick: () => answerConsent(false, host) }) as HTMLButtonElement;
  const row = h("div", { class: "rewrite-actions" }, on, later);
  const el = h("div", { class: "view", "data-role": "consent-card" }, card(null, stack(98, 16, title, list, note, row)));

  const sync = () => {
    const f = State.consentAsk;
    if (!f) return;
    const what = WHAT_IS_SENT[f];
    el.dataset.feature = f;
    title.textContent = t(what.title);
    clear(list);
    for (const line of what.lines) list.append(h("li", { text: t(line) }));
    note.textContent = t(SENT_FOOTER);
    on.textContent = t("Turn on");
    later.textContent = t("Not now");
  };
  return { el, sync };
}
