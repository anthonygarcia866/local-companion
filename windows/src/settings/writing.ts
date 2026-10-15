// Settings → Writing (1d, docs/phase1-plan.md §14): which apps the checker
// works in, and the rewrite's default tone per kind of app and per app. Rust
// owns the built-in lists (writing::apps, writing::tones) and reads the
// user's choices from settings.json within one reading of saving them.

import type { KnownApp } from "../core/bridge";
import type { Settings } from "../core/state";
import { TONES } from "../core/rewrite";
import { toneLabel } from "../views/rewrite";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

/** What the section needs from the Settings page. */
export interface WritingHost {
  settings(): Settings;
  save(): Promise<void>;
  /** Gets the section's redraw, for settings changed elsewhere (an answer
   *  to the one-time prompt in the island). */
  watch?(redraw: () => void): void;
}

const lower = (s: string) => s.toLowerCase();

/** The stored key for `exe` in `map`, whatever its case. */
function keyOf(map: Record<string, string>, exe: string): string | undefined {
  return Object.keys(map).find((k) => lower(k) === lower(exe));
}

/** Whether the checker is on in `exe`: the user's switch, else the default. */
export function appOn(map: Record<string, string>, exe: string, defaultOn: boolean): boolean {
  const key = keyOf(map, exe);
  const v = key === undefined ? undefined : map[key];
  return v === "on" ? true : v === "off" ? false : defaultOn;
}

/**
 * `map` with `exe` switched. A built-in app back at its default leaves the
 * map; an app the user answered (§14.2) keeps its entry either way, or it
 * would be asked about again.
 */
export function switchApp(map: Record<string, string>, exe: string, on: boolean, builtIn: { defaultOn: boolean } | null): Record<string, string> {
  const next = { ...map };
  const key = keyOf(next, exe);
  if (key !== undefined) delete next[key];
  if (builtIn && on === builtIn.defaultOn) return next;
  next[key ?? exe] = on ? "on" : "off";
  return next;
}

/** The apps the user answered: in the switches, not built in. */
export function answeredApps(map: Record<string, string>, known: KnownApp[]): string[] {
  return Object.keys(map)
    .filter((exe) => !known.some((k) => lower(k.exe) === lower(exe)))
    .sort((a, b) => a.localeCompare(b));
}

/** The strictness levels, quietest first (src-tauri/src/writing/checker.rs). */
export const STRICTNESS = ["spelling", "standard", "full"] as const;

export function strictnessLabel(level: string): string {
  switch (level) {
    case "spelling": return t("Spelling only");
    case "standard": return t("Standard: spelling, grammar, word choice, punctuation");
    default: return t("Full (default): everything Glim checks");
  }
}

/**
 * What a level covers, honestly (the §19 retest, 2026-10-14): Harper finds
 * no agreement error across "and" or in a "that" clause at any level.
 */
export function strictnessCovers(level: string): string {
  switch (level) {
    case "spelling": return t("Misspelled words and repeated words (“the the”).");
    case "standard": return t("Adds grammar, word choice, capitals and punctuation.");
    default: return t("Adds wordiness and style (“8am in the morning”, “for free of charge”).");
  }
}

/** A category's name in Settings → Writing → Rewrite tones. */
export function categoryLabel(id: string): string {
  switch (id) {
    case "email": return t("Email (Outlook, Gmail)");
    case "documents": return t("Documents (Word)");
    case "chat": return t("Chat apps and chat sites");
    case "other": return t("Everything else");
    default: return id;
  }
}

/**
 * `map` with `key` (a category id or an exe) set to `tone`. A category back
 * at its built-in tone (`builtin`) leaves the map; "" removes an app's own.
 */
export function setTone(map: Record<string, string>, key: string, tone: string, builtin: string | null): Record<string, string> {
  const next = { ...map };
  const found = keyOf(next, key);
  if (found !== undefined) delete next[found];
  if (!tone || tone === builtin) return next;
  next[found ?? key] = tone;
  return next;
}

/** The tone a category starts from: the user's, else the built-in one. */
export function categoryTone(map: Record<string, string>, id: string, builtin: string): string {
  const key = keyOf(map, id);
  const v = key === undefined ? "" : map[key];
  return (TONES as readonly string[]).includes(v) ? v : builtin;
}

/** "Acrobat" for "Acrobat.exe". */
const shortName = (exe: string) => exe.replace(/\.exe$/i, "");

export function writingSection(host: WritingHost, known: KnownApp[], toneDefaults: [string, string][] = []): HTMLElement {
  const apps = h("div", { class: "pill-group", id: "settings-writing-apps" });
  const tones = h("div", { class: "pill-group", id: "settings-writing-tones" });

  const strictness = h("select", { "data-role": "strictness" }) as HTMLSelectElement;
  for (const level of STRICTNESS) strictness.append(h("option", { value: level, text: strictnessLabel(level) }));
  strictness.value = (STRICTNESS as readonly string[]).includes(host.settings().checkerStrictness) ? host.settings().checkerStrictness : "full";
  const covers = h("ul", { class: "levels", "data-role": "strictness-covers" });
  function drawCovers() {
    clear(covers);
    for (const level of STRICTNESS) {
      const name = level === "spelling" ? t("Spelling only") : level === "standard" ? t("Standard") : t("Full");
      covers.append(h("li", { class: level === strictness.value ? "current" : "", "data-level": level },
        h("strong", { text: name }), h("span", { text: " " + strictnessCovers(level) })));
    }
  }
  drawCovers();
  strictness.addEventListener("change", () => {
    const v = strictness.value;
    host.settings().checkerStrictness = v === "spelling" || v === "standard" ? v : "full";
    drawCovers();
    void host.save();
  });
  strictness.id = "strictness";
  const strictnessRow = h("div", { class: "row" }, h("label", { text: t("Strictness"), for: "strictness" }), strictness);

  function toneSelect(value: string, withDefault: boolean, onChange: (tone: string) => void): HTMLSelectElement {
    const select = h("select", {}) as HTMLSelectElement;
    if (withDefault) select.append(h("option", { value: "", text: t("Its kind's tone") }));
    for (const tone of TONES) select.append(h("option", { value: tone, text: toneLabel(tone) }));
    select.value = value;
    select.addEventListener("change", () => onChange(select.value));
    return select;
  }

  /** The apps a tone of their own can be set for: those the checker is on in. */
  function onApps(): { exe: string; name: string }[] {
    const map = host.settings().writingApps;
    const builtIn = known.filter((k) => appOn(map, k.exe, k.defaultOn)).map((k) => ({ exe: k.exe, name: k.name }));
    const answered = answeredApps(map, known).filter((exe) => appOn(map, exe, false)).map((exe) => ({ exe, name: shortName(exe) }));
    return [...builtIn, ...answered];
  }

  function drawTones() {
    clear(tones);
    tones.append(h("h3", { text: t("Rewrite tones") }));
    tones.append(h("div", { class: "hint", text: t("The tone that comes first on the Rewrite card. Pressing the rewrite shortcut a second time sends it.") }));
    const set = (key: string, tone: string, builtin: string | null) => {
      host.settings().rewriteTones = setTone(host.settings().rewriteTones, key, tone, builtin);
      void host.save();
      drawTones();
    };
    for (const [id, builtin] of toneDefaults) {
      const value = categoryTone(host.settings().rewriteTones, id, builtin);
      const select = toneSelect(value, false, (tone) => set(id, tone, builtin));
      select.setAttribute("data-category", id);
      tones.append(h("div", { class: "row" }, h("label", { text: categoryLabel(id) }), select));
    }
    // Apps with a tone of their own, then a row to give one another.
    const own = Object.keys(host.settings().rewriteTones).filter((k) => /\.exe$/i.test(k));
    const names = new Map(onApps().map((a) => [lower(a.exe), a.name]));
    for (const exe of own) {
      const select = toneSelect(host.settings().rewriteTones[exe], true, (tone) => set(exe, tone, null));
      select.setAttribute("data-exe", exe);
      tones.append(h("div", { class: "row" }, h("label", { text: names.get(lower(exe)) ?? shortName(exe) }), select));
    }
    const free = onApps().filter((a) => !own.some((k) => lower(k) === lower(a.exe)));
    if (free.length > 0) {
      const app = h("select", { "data-role": "tone-app" }) as HTMLSelectElement;
      for (const a of free) app.append(h("option", { value: a.exe, text: a.name }));
      const tone = toneSelect(TONES[0], false, () => {});
      tone.setAttribute("data-role", "tone-pick");
      const add = h("button", { text: t("Add") });
      add.addEventListener("click", () => set(app.value, tone.value, null));
      tones.append(h("div", { class: "row" }, h("label", { text: t("For one app") }), app, tone, add));
    }
  }

  function row(exe: string, name: string, tags: string[], builtIn: KnownApp | null): HTMLElement {
    const on = appOn(host.settings().writingApps, exe, builtIn?.defaultOn ?? false);
    const sw = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on, "data-exe": exe }) as HTMLButtonElement;
    sw.addEventListener("click", () => {
      host.settings().writingApps = switchApp(host.settings().writingApps, exe, !on, builtIn);
      void host.save();
      draw();
      drawTones();
    });
    return h("div", { class: "pill-row" },
      h("span", { class: "name", text: name }),
      h("span", { class: "state", text: [exe, ...tags].join(" · ") }),
      sw,
    );
  }

  function draw() {
    clear(apps);
    apps.append(h("h3", { text: t("Apps") }));
    for (const k of known) {
      const tags = [k.chat ? t("chat rules") : "", k.defaultOn && !k.fixes ? t("suggestions only") : ""].filter(Boolean);
      apps.append(row(k.exe, k.name, tags, k));
    }
    for (const exe of answeredApps(host.settings().writingApps, known)) {
      apps.append(row(exe, shortName(exe), [t("suggestions only")], null));
    }
  }
  draw();
  drawTones();
  host.watch?.(() => {
    draw();
    drawTones();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Writing") })),
    h("div", { class: "hint", text: t("The apps Glim checks your writing in. Password managers and paused apps are never read, whatever their switch says.") }),
    apps,
    h("div", { class: "settings-group", id: "settings-writing-strictness" },
      strictnessRow,
      covers,
      h("div", { class: "hint", text: t("One level for every app. Chat apps keep their relaxed rules on top.") }),
      h("div", { class: "hint", "data-role": "strictness-ai-note", text: t("Agreement and meaning errors (like “Alex and I was”) can slip through at every level: they need the AI. For now, use Rewrite.") }),
    ),
    tones,
    bubbleRow(host),
  );
}

/**
 * Settings → Writing → "Show the mini lantern next to the text field" (1e,
 * docs/phase1-plan.md §7a.6): off by default until Anthony's live tests pass.
 */
export function bubbleRow(host: WritingHost): HTMLElement {
  const on = host.settings().writingBubble === true;
  const label = t("Show the mini lantern next to the text field");
  const sw = h("button", { class: on ? "switch on" : "switch", type: "button", "aria-pressed": String(on), "aria-label": label, "data-role": "writing-bubble" }) as HTMLButtonElement;
  sw.addEventListener("click", () => {
    const next = !sw.classList.contains("on");
    sw.classList.toggle("on", next);
    sw.setAttribute("aria-pressed", String(next));
    host.settings().writingBubble = next;
    void host.save();
  });
  return h("div", { class: "settings-group", id: "settings-writing-bubble" },
    h("div", { class: "row" }, h("label", { text: label }), sw),
    h("div", { class: "hint", text: t("A small lantern beside the field you type in, with the count; click it for the suggestions. It never takes the focus from your text.") }),
  );
}
