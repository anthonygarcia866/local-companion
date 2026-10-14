// The Google Docs tip card (core/docs-tip.ts): one line on turning on screen
// reader support, "Got it" (never again) and "Not now". The island never
// takes focus, so the user can press Ctrl+Alt+Z in the document right away.

import { h } from "./dom";
import { card, stack, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { AskClock } from "../core/ask";
import { DOCS_TIP_TIMEOUT_MS } from "../core/docs-tip";
import type { AskHost } from "./ask";
import { t } from "../i18n/i18n";

/** Runs while the tip is in sight. */
export const docsTipClock = new AskClock();

/** Closes the tip; `dismiss` (Got it) stores that it never shows again. */
export function closeDocsTip(dismiss: boolean, host: AskHost | null) {
  if (!State.writingDocsTip) return;
  docsTipClock.stop();
  State.writingDocsTip = false;
  if (dismiss) void Bridge.writingDocsTipDismiss();
  if (host && State.view === "writingDocsTip" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

export function buildDocsTip(host: AskHost): ViewHost {
  const title = h("div", { class: "title one-line" });
  const note = h("div", { class: "suggestion-more", role: "status" });
  const gotIt = h("button", { class: "apply-all", type: "button", "data-answer": "dismiss", onclick: () => closeDocsTip(true, host) }) as HTMLButtonElement;
  const later = h("button", { class: "link-btn", type: "button", "data-answer": "later", onclick: () => closeDocsTip(false, host) }) as HTMLButtonElement;
  const row = h("div", { class: "rewrite-actions" }, gotIt, later);
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, note, row)));

  const sync = () => {
    if (!State.writingDocsTip) return;
    title.textContent = t("Google Docs: Glim can't read this document");
    note.textContent = t("Turn on screen reader support (Ctrl+Alt+Z) so Glim can check your writing.");
    gotIt.textContent = t("Got it");
    later.textContent = t("Not now");
    if (State.mode === "expanded") docsTipClock.start("docs", () => closeDocsTip(false, host), DOCS_TIP_TIMEOUT_MS);
  };
  return { el, sync };
}
