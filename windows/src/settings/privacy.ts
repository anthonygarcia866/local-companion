// Settings → Privacy (1d, docs/phase1-plan.md §14.5, §14.8): the apps and
// sites Glim never reads in. The built-in password managers and payment
// sites are shown locked; the user's own entries are added, checked as they
// are typed (core/pause.ts) and removed. Rust rebuilds its pause list as soon
// as settings.json is saved.

import type { PauseBuiltins } from "../core/bridge";
import type { Settings } from "../core/state";
import { addEntry, removeEntry, userApp, userSite, type PauseRefusal } from "../core/pause";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

export interface PrivacyHost {
  settings(): Settings;
  save(): Promise<void>;
}

/** Why an entry was refused, in words. */
export function refusalText(reason: PauseRefusal): string {
  switch (reason) {
    case "empty": return t("Type an app or a site first.");
    case "not-exe": return t("An app is its program's file name, ending in .exe (Quicken.exe).");
    case "bad-name": return t("Just the file name, without a folder.");
    case "built-in": return t("Already paused: it's built in.");
    case "bad-host": return t("That isn't a site's address (example.com).");
    case "bad-path": return t("That page path can't be matched.");
    case "bad-segment": return t("Use */word to pause any page with that word in its path.");
    case "bad-site": return t("A site is example.com, example.com/path or */word, without ? # @ or a port.");
  }
}

export function privacySection(host: PrivacyHost, builtIn: PauseBuiltins): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:8px" });

  function list(kind: "apps" | "sites"): HTMLElement {
    const key = kind === "apps" ? "pauseApps" : "pauseSites";
    const box = h("div", { class: "pill-group" });
    box.append(h("h3", { text: kind === "apps" ? t("Paused apps") : t("Paused sites") }));
    for (const entry of host.settings()[key]) {
      const remove = h("button", { class: "danger", text: t("Remove"), "data-entry": entry });
      remove.addEventListener("click", () => {
        host.settings()[key] = removeEntry(host.settings()[key], entry);
        void host.save();
        draw();
      });
      box.append(h("div", { class: "pill-row" }, h("span", { class: "name", text: entry }), remove));
    }
    const input = h("input", {
      type: "text",
      spellcheck: "false",
      autocomplete: "off",
      "data-role": `pause-${kind}`,
      placeholder: kind === "apps" ? "Quicken.exe" : "example.com",
    }) as HTMLInputElement;
    const why = h("div", { class: "hint", role: "status" });
    const add = h("button", { text: t("Add"), "data-role": `pause-${kind}-add` });
    const check = () => (kind === "apps" ? userApp(input.value, builtIn.apps) : userSite(input.value, builtIn.sites));
    input.addEventListener("input", () => {
      const c = check();
      why.textContent = c.ok || !input.value.trim() ? "" : refusalText(c.reason);
    });
    add.addEventListener("click", () => {
      const c = check();
      if (!c.ok) {
        why.textContent = refusalText(c.reason);
        return;
      }
      host.settings()[key] = addEntry(host.settings()[key], c.entry);
      void host.save();
      draw();
    });
    box.append(h("div", { class: "row" }, input, add), why);
    // The built-in ones: shown, locked.
    const locked = kind === "apps" ? builtIn.apps : builtIn.sites;
    box.append(h("div", { class: "hint", "data-role": `pause-${kind}-builtin`, text: `🔒 ${t("Built in, always paused:")} ${locked.join(", ")}` }));
    return box;
  }

  function draw() {
    clear(body);
    body.append(list("apps"), list("sites"));
  }
  draw();

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Privacy") })),
    h("div", { class: "hint", text: t("Glim never reads fields in these apps or on these sites: nothing is checked, kept or rewritten there.") }),
    body,
    h("div", { class: "hint", text: t("Glim keeps nothing you write: only these settings, the API spend numbers, and your API key in Windows Credential Manager.") }),
    h("div", { class: "hint", text: t("Network: this computer only, plus api.anthropic.com when you ask for a rewrite.") }),
  );
}
