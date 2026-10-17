// Settings → section navigation (1d retest, 2026-10-14: "Settings is one long
// scroll"). A sticky sidebar lists the sections (and Writing's three parts);
// a click or Enter scrolls there, and the entry for the part on screen is
// marked (aria-current). A search box above it hides every setting whose text
// doesn't match: a heading that matches keeps its whole block.

import { h } from "../views/dom";
import { N_, t } from "../i18n/i18n";

export interface NavEntry {
  /** The id of the element it scrolls to. */
  id: string;
  /** English text (marked with N_), shown through t(). */
  label: string;
  children?: NavEntry[];
}

/** The order of the sidebar, and of the sections on the page. */
export const NAV: NavEntry[] = [
  { id: "settings-general", label: N_("General") },
  { id: "settings-presence", label: N_("Presence") },
  {
    id: "settings-writing",
    label: N_("Writing"),
    children: [
      { id: "settings-writing-apps", label: N_("Apps") },
      { id: "settings-writing-tones", label: N_("Tones") },
      { id: "settings-writing-strictness", label: N_("Strictness") },
    ],
  },
  { id: "settings-calendar", label: N_("Calendar") },
  { id: "settings-privacy", label: N_("Privacy") },
  { id: "settings-shortcuts", label: N_("Shortcuts") },
  { id: "settings-agents", label: N_("Claude Code & Agents") },
  { id: "settings-about", label: N_("About") },
];

/** Every entry, parents before their children. */
export function flatNav(entries: NavEntry[] = NAV): NavEntry[] {
  return entries.flatMap((e) => [e, ...flatNav(e.children ?? [])]);
}

/**
 * The entry for what is on screen: the last one whose top is at or above
 * `offset` px from the top of the view (the first one before any has
 * scrolled past). `tops` are in page order, relative to the view.
 */
export function currentSection(tops: { id: string; top: number }[], offset = 24): string | null {
  let current: string | null = tops[0]?.id ?? null;
  for (const { id, top } of tops) {
    if (top <= offset) current = id;
  }
  return current;
}

/** Lowercase, accents and extra spaces gone: "Écran " matches "ecran". */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/\s+/g, " ").trim();
}

/** Containers the search looks inside; anything else is one setting. */
const CONTAINER_TAGS = new Set(["SECTION"]);
const CONTAINER_CLASSES = ["pill-group", "settings-group"];
const HEADINGS = new Set(["H2", "H3"]);

function isContainer(el: HTMLElement): boolean {
  return CONTAINER_TAGS.has(el.tagName) || CONTAINER_CLASSES.some((c) => el.classList.contains(c));
}

function showAll(el: HTMLElement) {
  el.hidden = false;
  for (const c of Array.from(el.children) as HTMLElement[]) showAll(c);
}

/**
 * Hides what doesn't match `query` under `el` and returns whether anything
 * under it is still shown. A container shows if its heading matches (all of
 * it) or any of its children does (those only, with the heading).
 */
export function filterSettings(el: HTMLElement, query: string): boolean {
  const q = fold(query);
  if (!q) {
    showAll(el);
    return true;
  }
  if (!isContainer(el)) {
    el.hidden = !fold(el.textContent ?? "").includes(q);
    return !el.hidden;
  }
  const kids = Array.from(el.children) as HTMLElement[];
  const heading = kids.find((k) => HEADINGS.has(k.tagName));
  if (heading && fold(heading.textContent ?? "").includes(q)) {
    showAll(el);
    return true;
  }
  let any = false;
  for (const k of kids) {
    if (k === heading) continue;
    if (filterSettings(k, query)) any = true;
  }
  if (heading) heading.hidden = !any;
  el.hidden = !any;
  return any;
}

export interface Nav {
  /** The sidebar: search box, then the links. */
  el: HTMLElement;
  /** Marks the entry for `id` as the current one. */
  mark(id: string | null): void;
  /** Runs the search again (after a redraw). */
  refilter(): void;
  search: HTMLInputElement;
  /** The "nothing matches" line, for the page to place. */
  empty: HTMLElement;
}

/**
 * The sidebar. `go(id)` scrolls to an entry; `content()` is what the search
 * filters (the sections).
 */
export function buildNav(go: (id: string) => void, content: () => HTMLElement[]): Nav {
  const search = h("input", {
    type: "search",
    class: "nav-search",
    placeholder: t("Search settings"),
    "aria-label": t("Search settings"),
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;
  const empty = h("div", { class: "hint nav-empty", role: "status" });
  empty.hidden = true;
  const links = new Map<string, HTMLAnchorElement>();
  const list = h("ul", { class: "nav-list" });

  function link(e: NavEntry, sub: boolean): HTMLElement {
    const a = h("a", {
      href: `#${e.id}`,
      class: sub ? "nav-link sub" : "nav-link",
      "data-target": e.id,
      text: t(e.label),
    }) as HTMLAnchorElement;
    a.addEventListener("click", (ev: Event) => {
      ev.preventDefault();
      go(e.id);
      mark(e.id);
    });
    links.set(e.id, a);
    return a;
  }
  for (const e of NAV) {
    const item = h("li", {}, link(e, false));
    if (e.children?.length) item.append(h("ul", { class: "nav-sub" }, ...e.children.map((c) => h("li", {}, link(c, true)))));
    list.append(item);
  }

  function mark(id: string | null) {
    for (const [key, a] of links) {
      const on = key === id;
      a.classList.toggle("current", on);
      if (on) a.setAttribute("aria-current", "true");
      else a.setAttribute("aria-current", "false");
    }
  }

  function refilter() {
    const q = search.value;
    let any = false;
    for (const s of content()) if (filterSettings(s, q)) any = true;
    empty.hidden = any || !fold(q);
    empty.textContent = any ? "" : t("No setting matches “{query}”.", { query: q.trim() });
  }
  search.addEventListener("input", refilter);
  search.addEventListener("keydown", (ev: Event) => {
    if ((ev as KeyboardEvent).key === "Escape" && search.value) {
      ev.preventDefault();
      search.value = "";
      refilter();
    }
  });

  const el = h("nav", { class: "settings-nav", "aria-label": t("Settings sections") }, search, list);
  return { el, mark, refilter, search, empty };
}

