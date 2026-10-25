// Settings → Ask Glim (docs/glim-learns-plan.md §1-§2, decided 2026-10-24):
// the "Learn shortcuts from what I ask" switch (on by default), the learned
// shortcuts (each can be forgotten, or all at once) and the routines (their
// steps; Rename, Remove). Everything here is on this PC only; Rust keeps the
// files and checks every change (learn/mod.rs).

import { Bridge, onEvent } from "../core/bridge";
import type { Settings } from "../core/state";
import { keyText, type LearnView } from "../core/learn";
import { stepText } from "../core/voice";
import { routineProblemText } from "../views/voice";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

export interface AskHost {
  settings(): Settings;
  save(): Promise<void>;
}

let listening = false;
let reload: (() => void) | null = null;

export function askSection(host: AskHost): HTMLElement {
  const label = t("Learn shortcuts from what I ask");
  const on = host.settings().learnShortcuts !== false;
  const sw = h("button", { class: on ? "switch on" : "switch", type: "button", "aria-pressed": String(on), "data-role": "learn-shortcuts", "aria-label": label }) as HTMLButtonElement;
  sw.addEventListener("click", () => {
    const next = !sw.classList.contains("on");
    sw.classList.toggle("on", next);
    sw.setAttribute("aria-pressed", String(next));
    host.settings().learnShortcuts = next;
    void host.save();
  });

  const shortcuts = h("div", { class: "pill-group", "data-role": "learn-shortcut-list" });
  const routines = h("div", { class: "pill-group", "data-role": "learn-routine-list" });

  function drawShortcuts(view: LearnView) {
    clear(shortcuts);
    shortcuts.append(h("h3", { text: t("Learned shortcuts") }));
    if (view.shortcuts.length === 0) {
      shortcuts.append(h("div", { class: "hint", text: t("Nothing learned yet.") }));
      return;
    }
    for (const s of view.shortcuts) {
      const forget = h("button", { class: "danger", type: "button", "data-role": "learn-forget", text: t("Forget") });
      forget.addEventListener("click", async () => {
        await Bridge.learnForgetShortcut(s.key, s.slot);
        load();
      });
      shortcuts.append(h("div", { class: "pill-row", "data-key": s.key },
        h("span", { class: "name", text: `“${keyText(s)}” → ${stepText(s.step)}` }),
        forget,
      ));
    }
    const all = h("button", { class: "danger", type: "button", "data-role": "learn-forget-all", text: t("Forget all shortcuts") });
    all.addEventListener("click", async () => {
      await Bridge.learnForgetShortcuts();
      load();
    });
    shortcuts.append(h("div", { class: "row" }, all));
  }

  function drawRoutines(view: LearnView) {
    clear(routines);
    routines.append(h("h3", { text: t("Routines") }));
    if (view.routines.length === 0) {
      routines.append(h("div", { class: "hint", text: t("No routines yet. With Ask Glim, say “When I say work mode, open Outlook and Codex”.") }));
      return;
    }
    for (const r of view.routines) {
      const why = h("div", { class: "hint", role: "status" });
      const input = h("input", { type: "text", spellcheck: "false", autocomplete: "off", maxlength: "40", "data-role": "learn-rename-input", value: r.name }) as HTMLInputElement;
      input.value = r.name;
      const rename = h("button", { type: "button", "data-role": "learn-rename", text: t("Rename") });
      rename.addEventListener("click", async () => {
        const to = input.value.trim();
        if (!to || to === r.name) return;
        const problem = await Bridge.learnRoutineRename(r.name, to);
        if (problem) why.textContent = routineProblemText(problem);
        else load();
      });
      const remove = h("button", { class: "danger", type: "button", "data-role": "learn-remove", text: t("Remove") });
      remove.addEventListener("click", async () => {
        await Bridge.learnRoutineDelete(r.name);
        load();
      });
      routines.append(h("div", { class: "learn-routine", "data-routine": r.name },
        h("div", { class: "row" }, input, rename, remove),
        h("div", { class: "hint", text: r.steps.map((s, i) => `${i + 1}. ${stepText(s)}`).join("  ") }),
        why,
      ));
    }
  }

  function load() {
    void Bridge.learnList().then((view) => {
      const v = view ?? { shortcuts: [], routines: [] };
      drawShortcuts(v);
      drawRoutines(v);
    });
  }
  reload = load;
  load();
  // A shortcut learned or a routine saved while Settings is open.
  if (!listening) {
    listening = true;
    void onEvent<null>("learn-changed", () => reload?.());
  }

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Ask Glim") })),
    h("div", { class: "row" }, h("label", { text: label }), sw),
    h("div", { class: "hint", text: t("When something you ask with Ask Glim works, Glim keeps it and does it straight away the next time, without asking Claude. Only opening apps and picking up work. Kept on this PC; one unused for 30 days is forgotten.") }),
    shortcuts,
    routines,
    h("div", { class: "hint", text: t("Routines run when you say their name with Ask Glim, even with learning off.") }),
  );
}
