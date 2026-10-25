// Settings → Memory (docs/glim-learns-plan.md §3, decided 2026-10-24): the
// rules and "about me" facts the user asked Glim to remember, each with
// Remove, and Forget everything. Rust keeps the file and its filter
// (memory.rs); the page only shows it. The privacy change is said here:
// these go to api.anthropic.com with Ask Glim requests.

import { Bridge, onEvent, type Memory } from "../core/bridge";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

let listening = false;
let reload: (() => void) | null = null;

export function memorySection(): HTMLElement {
  const lists = h("div", { style: "display:flex;flex-direction:column;gap:8px" });

  function list(kind: "rule" | "fact", entries: string[]): HTMLElement {
    const box = h("div", { class: "pill-group", "data-role": `memory-${kind}s` });
    box.append(h("h3", { text: kind === "rule" ? t("Rules") : t("About you") }));
    if (entries.length === 0) {
      box.append(h("div", {
        class: "hint",
        text: kind === "rule"
          ? t("No rules yet. With Ask Glim, say “Remember: when I say the office I mean Outlook”.")
          : t("Nothing yet. With Ask Glim, say “Remember that I prefer short replies”."),
      }));
    }
    for (const entry of entries) {
      const remove = h("button", { class: "danger", type: "button", "data-role": "memory-remove", text: t("Remove") });
      remove.addEventListener("click", async () => {
        await Bridge.memoryDelete(kind, entry);
        load();
      });
      box.append(h("div", { class: "pill-row" }, h("span", { class: "name", text: entry }), remove));
    }
    return box;
  }

  function draw(m: Memory) {
    clear(lists);
    lists.append(list("rule", m.rules), list("fact", m.facts));
    if (m.rules.length + m.facts.length > 0) {
      const all = h("button", { class: "danger", type: "button", "data-role": "memory-forget-all", text: t("Forget everything") });
      all.addEventListener("click", async () => {
        await Bridge.memoryForgetAll();
        load();
      });
      lists.append(h("div", { class: "row" }, all));
    }
  }

  function load() {
    void Bridge.memoryList().then((m) => draw(m ?? { rules: [], facts: [] }));
  }
  reload = load;
  load();
  // Something remembered by voice while Settings is open.
  if (!listening) {
    listening = true;
    void onEvent<null>("memory-changed", () => reload?.());
  }

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Memory") })),
    h("div", { class: "hint", "data-role": "memory-privacy", text: t("What you asked Glim to remember. Every fact, and the rules that match what you ask, go to api.anthropic.com with your Ask Glim requests; never with dictation. Kept on this PC.") }),
    h("div", { class: "hint", text: t("Glim never keeps health, money, ID numbers or tenant and customer details.") }),
    lists,
  );
}
