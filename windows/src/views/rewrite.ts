// The Rewrite card (1c, docs/phase1-plan.md §13): pick a tone, the paragraph
// goes to the Claude API, the rewrite comes back as a suggestion, and it is
// typed in only on "Use this" (1b's apply path, in Rust).
//
// Like the suggestion list, a click sends ids only: a tone id, or which
// rewrite. Rust takes the paragraph from its own capture state and keeps the
// rewrite's text; the text shown here is never logged or stored.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { card, stack, outcomeNote, type ViewActions, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State, type RewriteState } from "../core/state";
import { costText, chooseTone, orderedTones, sendTone } from "../core/rewrite";
import { t, tl, type Msg } from "../i18n/i18n";

/** The tone buttons' labels, by Rust's tone id. */
export function toneLabel(tone: string): string {
  switch (tone) {
    case "friendlier": return t("Friendlier");
    case "formal": return t("More formal");
    case "shorter": return t("Shorter");
    case "grammar": return t("Fix grammar");
    case "casual": return t("Casual");
    default: return tone;
  }
}

/** Why there is no rewrite, in words, for a code from Rust (ai/rewrite.rs). */
export function failureText(code: string, retryAfter: number | null): string {
  switch (code) {
    case "paused": return t("Glim is paused.");
    case "local-brain": return t("Rewriting with the local model isn't built yet. Choose Claude API in Settings → AI.");
    case "no-key": return t("Add your Anthropic API key in Settings → AI first.");
    case "key-error": return t("Windows couldn't read the API key. Check Settings → AI.");
    case "cap": return t("This month's spending cap is reached, so nothing more is sent this month.");
    case "no-paragraph":
    case "empty": return t("Click in the paragraph you want rewritten first.");
    case "too-long": return t("That paragraph is too long to rewrite (over 2,000 characters).");
    case "sensitive": return t("Not sent: it looks like it has a card, SSN or bank number in it.");
    case "truncated": return t("The rewrite was cut off. Try Shorter.");
    case "refused": return t("Claude declined to rewrite this.");
    case "bad-answer": return t("Claude's answer wasn't a paragraph. Try again.");
    case "network": return t("Couldn't reach Claude. Check the internet connection.");
    case "api-key": return t("Anthropic refused the API key. Check it in Settings → AI.");
    case "api-billing": return t("Anthropic reports a billing problem on your account.");
    case "api-permission": return t("This API key isn't allowed to use that model.");
    case "api-rate":
      return retryAfter
        ? t("Too many requests right now. Try again in {seconds} s.", { seconds: retryAfter })
        : t("Too many requests, or your Anthropic spend limit was reached. Try again later.");
    case "api-busy": return t("Claude is busy. Try again in a moment.");
    case "api-server": return t("Anthropic's servers had a problem. Try again in a moment.");
    default: return t("Anthropic couldn't take this request.");
  }
}

/** The line under a finished rewrite: what a click did, or what it would do. */
export function rewriteNote(r: RewriteState): string {
  if (r.busy) return t("Working…");
  if (r.outcome === "applied") return t("Rewritten. Ctrl+Z in {app} undoes it.", { app: r.app });
  const done = outcomeNote(r.outcome || undefined, r.app);
  if (done) return done;
  const cost = t("{model} · about {cost}", { model: r.model, cost: costText(r.costUsd) });
  if (r.mode === "view") return `${cost} · ${t("Suggestions only in {app} for now.", { app: r.app })}`;
  return cost;
}

/** What the card's buttons are, for a finished rewrite. */
export function rewriteButtons(r: RewriteState): ("use" | "copy")[] {
  if (r.outcome === "applied") return [];
  if (r.mode === "apply" && r.offerable) return ["use"];
  if (r.mode === "apply" || r.mode === "copy" || r.mode === "away") return ["copy"];
  return [];
}

export function buildRewrite(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title one-line" });
  const body = h("div", { class: "rewrite-body" });
  const note = h("div", { class: "suggestion-more", role: "status" });
  const close = h("button", { class: "suggestion-dismiss", title: tl("Close"), "aria-label": tl("Close"), onclick: () => closeCard() }, svg(ICONS.xmark, 12));
  const row = h("div", { class: "rewrite-actions" });
  const pager = h("div", { class: "suggestion-pager" }, row, note, close);
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, body, pager)));

  const closeCard = () => {
    void Bridge.writingRewriteDismiss();
    State.rewrite = null;
    actions.setView(State.defaultView());
  };
  const pick = (tone: string) => {
    sendTone(tone);
    sync();
  };
  const again = () => {
    void Bridge.writingRewriteDismiss();
    State.rewrite = chooseTone(State.rewrite?.defaultTone ?? "");
    sync();
  };
  const act = (what: "use" | "copy") => {
    const r = State.rewrite;
    if (!r || r.phase !== "done" || r.busy) return;
    State.rewrite = { ...r, busy: true, outcome: "" };
    sync();
    if (what === "use") void Bridge.writingRewriteApply(r.id);
    else void Bridge.writingRewriteCopy(r.id);
    // The buttons come back if Rust never answers.
    window.setTimeout(() => {
      if (State.rewrite?.id === r.id && State.rewrite.busy) {
        State.rewrite = { ...State.rewrite, busy: false };
        sync();
      }
    }, 3000);
  };
  const button = (label: Msg | string, onclick: () => void, cls = "apply-all") =>
    h("button", { class: cls, type: "button", text: label, onclick }) as HTMLButtonElement;

  const sync = () => {
    const r = State.rewrite ?? chooseTone();
    clear(body);
    clear(row);
    switch (r.phase) {
      case "choose":
        title.textContent = t("Rewrite this paragraph");
        body.append(h("div", { class: "rewrite-tones" }, ...orderedTones(r.defaultTone).map((tone) => {
          const b = button(toneLabel(tone), () => pick(tone), tone === r.defaultTone ? "fix fix-btn rewrite-tone default" : "fix fix-btn rewrite-tone");
          b.setAttribute("data-tone", tone);
          return b;
        })));
        note.textContent = r.defaultTone
          ? t("Sends this paragraph to Claude. Press the shortcut again for {tone}. Nothing is typed until you click Use this.", { tone: toneLabel(r.defaultTone) })
          : t("Sends this paragraph to Claude. Nothing is typed until you click Use this.");
        break;
      case "sending":
        title.textContent = t("Rewriting with Claude…");
        body.append(h("div", { class: "suggestion-message", text: toneLabel(r.tone) }));
        note.textContent = t("Sending this paragraph to Claude…");
        break;
      case "done": {
        title.textContent = t("Rewrite: {tone}", { tone: toneLabel(r.tone) });
        body.append(h("div", { class: "rewrite-text", text: r.text }));
        for (const b of rewriteButtons(r)) {
          const btn = b === "use" ? button(t("Use this"), () => act("use")) : button(t("Copy"), () => act("copy"));
          btn.disabled = r.busy;
          row.append(btn);
        }
        if (r.outcome !== "applied") row.append(button(t("Try another tone"), again, "link-btn rewrite-again"));
        note.textContent = rewriteNote(r);
        break;
      }
      case "failed":
        title.textContent = t("Not rewritten");
        body.append(h("div", { class: "suggestion-message rewrite-why", text: failureText(r.code, r.retryAfter) }));
        row.append(button(t("Try again"), again, "link-btn rewrite-again"));
        note.textContent = r.costUsd > 0 ? t("{model} · about {cost}", { model: r.model, cost: costText(r.costUsd) }) : "";
        break;
    }
  };
  return { el, sync };
}
