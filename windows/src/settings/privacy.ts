// Settings → Privacy (1d, docs/phase1-plan.md §14.5, §14.8): the apps and
// sites Glim never reads in. The built-in password managers and payment
// sites are shown locked; the user's own entries are added, checked as they
// are typed (core/pause.ts) and removed. Rust rebuilds its pause list as soon
// as settings.json is saved. Also the switch that keeps Glim's windows out of
// screen capture (docs/glim-learns-plan.md §5, on by default; Rust applies
// it on save, screen_capture.rs). And the privacy shield (owner, 2026-11-03;
// src-tauri/src/shield/): its switch, on by default, and the names it always
// hides, one per line (Rust adds Outlook's address book and remembered
// recipients).

import { Bridge, onEvent, type PauseBuiltins } from "../core/bridge";
import { countdown } from "../core/capture-allow";
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

/** The names list as stored: one per line, trimmed, no blanks or repeats (any case). */
export function shieldNamesFrom(text: string): string[] {
  const out: string[] = [];
  for (const line of text.split(/\r?\n/)) {
    const name = line.trim().replace(/\s+/g, " ");
    if (name && !out.some((n) => n.toLowerCase() === name.toLowerCase())) out.push(name);
  }
  return out;
}

let allowTicker: ReturnType<typeof setInterval> | null = null;

/**
 * "Allow screenshots for 10 minutes" under the hide switch (only while it's
 * on): Rust lets Glim be captured, then hides it again by itself
 * (screen_capture.rs). While allowed: the time left and "Hide again now".
 */
export function captureAllowBox(hiding: () => boolean, tickMs = 1000): { el: HTMLElement; redraw: () => void } {
  const el = h("div", { "data-role": "capture-allow" });
  let until = 0;
  let left: HTMLElement | null = null;

  function redraw() {
    clear(el);
    left = null;
    if (!hiding()) return;
    if (countdown(until, Date.now())) {
      left = h("span", { class: "hint", "data-role": "capture-allow-left", role: "status", text: t("Screenshots allowed: {time} left", { time: countdown(until, Date.now()) }) });
      const again = h("button", { type: "button", "data-role": "capture-hide-again", text: t("Hide again now") });
      again.addEventListener("click", () => void Bridge.captureHideAgain().then((u) => set(u ?? 0)));
      el.append(h("div", { class: "row" }, left, again));
    } else {
      const allow = h("button", { type: "button", "data-role": "capture-allow-button", text: t("Allow screenshots for 10 minutes") });
      allow.addEventListener("click", () => void Bridge.captureAllow().then((u) => set(u ?? 0)));
      el.append(h("div", { class: "row" }, allow), h("div", { class: "hint", text: t("Glim hides itself again by itself after 10 minutes, and whenever it restarts.") }));
    }
  }

  function set(u: number) {
    until = u;
    redraw();
  }

  redraw();
  void Bridge.captureStatus().then((u) => set(u ?? 0));
  void onEvent<number>("capture-allowed", (u) => set(u ?? 0));
  if (allowTicker) clearInterval(allowTicker);
  allowTicker = setInterval(() => {
    if (!until) return;
    const text = countdown(until, Date.now());
    if (!text) return set(0);
    if (left) left.textContent = t("Screenshots allowed: {time} left", { time: text });
  }, tickMs);
  (allowTicker as { unref?: () => void }).unref?.();
  return { el, redraw };
}

/**
 * Settings → Privacy → Activity watch (owner 2026-11-24; Rust:
 * activity_watch.rs): a switch that turns a local-only log on for 7 days,
 * then off by itself. Its state is Rust's own file, so this asks Rust for it
 * and never saves it in settings.json.
 */
export function activityWatchBox(): HTMLElement {
  const el = h("div", { "data-role": "activity-watch" });
  const label = t("Activity watch (7 days)");
  let until = 0;
  function draw() {
    clear(el);
    const on = until > Date.now() / 1000;
    const sw = h("button", { class: on ? "switch on" : "switch", type: "button", "aria-pressed": String(on), "data-role": "activity-watch-switch", "aria-label": label }) as HTMLButtonElement;
    sw.addEventListener("click", () => {
      void Bridge.activityWatchSet(!on)
        .then((u) => { until = u ?? 0; draw(); })
        .catch(() => draw());
    });
    el.append(
      h("div", { class: "row" }, h("label", { text: label }), sw),
      h("div", { class: "hint", text: t("On this PC only, logs each new terminal, PowerShell, Word or Outlook process (what started it and its command line, passwords and keys masked) and each window that comes to the front (its app and title). Nothing you type. Turns itself off after 7 days.") }),
    );
    if (on) {
      const show = h("button", { type: "button", "data-role": "activity-watch-show", text: t("Show the log") });
      show.addEventListener("click", () => void Bridge.activityWatchReveal());
      el.append(h("div", { class: "row" }, h("span", { class: "hint", "data-role": "activity-watch-until", role: "status", text: t("On until {time}.", { time: new Date(until * 1000).toLocaleString() }) }), show));
    }
  }
  draw();
  void Bridge.activityWatchStatus().then((u) => { until = u ?? 0; draw(); });
  return el;
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

  const hideLabel = t("Hide Glim in screen shares and recordings");
  const hidden = host.settings().hideFromCapture !== false;
  const hide = h("button", { class: hidden ? "switch on" : "switch", type: "button", "aria-pressed": String(hidden), "data-role": "hide-from-capture", "aria-label": hideLabel }) as HTMLButtonElement;
  const allowBox = captureAllowBox(() => host.settings().hideFromCapture !== false);
  hide.addEventListener("click", () => {
    const next = !hide.classList.contains("on");
    hide.classList.toggle("on", next);
    hide.setAttribute("aria-pressed", String(next));
    host.settings().hideFromCapture = next;
    void host.save();
    allowBox.redraw();
  });

  const shieldLabel = t("Privacy shield");
  const shielded = host.settings().privacyShield !== false;
  const shield = h("button", { class: shielded ? "switch on" : "switch", type: "button", "aria-pressed": String(shielded), "data-role": "privacy-shield", "aria-label": shieldLabel }) as HTMLButtonElement;
  shield.addEventListener("click", () => {
    const next = !shield.classList.contains("on");
    shield.classList.toggle("on", next);
    shield.setAttribute("aria-pressed", String(next));
    host.settings().privacyShield = next;
    void host.save();
  });
  const namesLabel = t("Names to always hide");
  const names = h("textarea", { rows: "4", spellcheck: "false", autocomplete: "off", "data-role": "shield-names", "aria-label": namesLabel }) as HTMLTextAreaElement;
  names.value = (host.settings().shieldNames ?? []).join("\n");
  names.addEventListener("change", () => {
    host.settings().shieldNames = shieldNamesFrom(names.value);
    void host.save();
  });

  const mineLabel = t("My names and businesses");
  const mine = h("textarea", { rows: "3", spellcheck: "false", autocomplete: "off", "data-role": "own-names", "aria-label": mineLabel }) as HTMLTextAreaElement;
  mine.value = (host.settings().ownNames ?? []).join("\n");
  mine.addEventListener("change", () => {
    host.settings().ownNames = shieldNamesFrom(mine.value);
    void host.save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Privacy") })),
    h("div", { class: "row" }, h("label", { text: shieldLabel }), shield),
    h("div", { class: "hint", text: t("Before anything goes to Claude, names, emails, phone numbers, addresses and unit numbers are swapped for placeholders on this PC and put back in the answer.") }),
    h("div", { class: "pill-group" }, h("h3", { text: mineLabel }), mine, h("div", { class: "hint", text: t("One per line: your own names and your businesses' names. Always hidden, in any case, and put back exactly as you wrote them. Add your business names here. Signature blocks (from a \"--\" line, or a sign-off like \"Best,\" at the end) are never sent or rewritten.") })),
    h("div", { class: "pill-group" }, h("h3", { text: namesLabel }), names, h("div", { class: "hint", text: t("One per line. Names in Outlook's address book (while Outlook is open) and people you've drafted emails to are hidden too.") })),
    h("div", { class: "hint", text: t("Glim never reads fields in these apps or on these sites: nothing is checked, kept or rewritten there.") }),
    body,
    h("div", { class: "row" }, h("label", { text: hideLabel }), hide),
    h("div", { class: "hint", text: t("Screen shares, recordings and screenshots show what's behind Glim, not its cards.") }),
    allowBox.el,
    activityWatchBox(),
    h("div", { class: "hint", text: t("Glim keeps nothing you write: only these settings, the API spend numbers, and your API key in Windows Credential Manager.") }),
    h("div", { class: "hint", text: t("Network: this computer only, plus api.anthropic.com when you ask for a rewrite.") }),
  );
}
