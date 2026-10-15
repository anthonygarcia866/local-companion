// The mini lantern's window (1e): wires BubbleView to Rust. Listens only
// (capabilities/bubble.json); acts only through Glim's own commands.

import "../style.css";
import "../mascot/lantern.css";
import "./bubble.css";
import { Bridge, onEvent } from "../core/bridge";
import { State, type WritingFix, type WritingResult, type WritingSuggestion } from "../core/state";
import { BubbleView, type BubbleWindowState } from "./view";
import { suggestionNote } from "../views/views";
import { resolveLanguage, setLanguage, systemLanguages } from "../i18n/i18n";

async function main() {
  const boot = await Bridge.boot();
  if (boot) State.settings = { ...State.settings, ...boot.settings };
  setLanguage(resolveLanguage(State.settings.language, systemLanguages()));
  const view = new BubbleView({
    setOpen: (open) => void Bridge.bubblePopover(open),
    fix: (checkId, index, choice) => void Bridge.writingFix(checkId, index, choice),
    copy: (checkId, index, choice) => void Bridge.writingCopy(checkId, index, choice),
    rewrite: () => void Bridge.bubbleRewrite(),
    now: () => performance.now(),
  });
  document.getElementById("bubble-root")!.append(view.el);
  await onEvent<BubbleWindowState>("bubble-state", (w) => view.setWindow(w));
  await onEvent<null>("bubble-field-left", () => view.step({ kind: "field-left" }));
  await onEvent<{ items: WritingSuggestion[] } & WritingFix>("writing-suggestions", ({ items, checkId, mode, app }) => view.setList(items, { checkId, mode, app }));
  await onEvent<WritingResult>("writing-fix", (result) => view.setNote(suggestionNote(State.writingFix, result, false)));
  // The 20 s no-pointer clock (core/bubble.ts).
  setInterval(() => view.step({ kind: "tick", now: performance.now() }), 1000);
}

void main();
