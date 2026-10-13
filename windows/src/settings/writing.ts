// Settings → Writing (1d, docs/phase1-plan.md §14): which apps the checker
// works in. Rust owns the built-in list (writing::apps) and reads the user's
// switches from settings.json within one reading of saving them.

import type { KnownApp } from "../core/bridge";
import type { Settings } from "../core/state";
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

/** "Acrobat" for "Acrobat.exe". */
const shortName = (exe: string) => exe.replace(/\.exe$/i, "");

export function writingSection(host: WritingHost, known: KnownApp[]): HTMLElement {
  const apps = h("div", { class: "pill-group" });

  function row(exe: string, name: string, tags: string[], builtIn: KnownApp | null): HTMLElement {
    const on = appOn(host.settings().writingApps, exe, builtIn?.defaultOn ?? false);
    const sw = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on, "data-exe": exe }) as HTMLButtonElement;
    sw.addEventListener("click", () => {
      host.settings().writingApps = switchApp(host.settings().writingApps, exe, !on, builtIn);
      void host.save();
      draw();
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
  host.watch?.(draw);

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("Writing") })),
    h("div", { class: "hint", text: t("The apps Glim checks your writing in. Password managers and paused apps are never read, whatever their switch says.") }),
    apps,
  );
}
