// Settings section navigation (1d retest, 2026-10-14; src/settings/nav.ts):
// the sidebar's entries and order, the scroll spy, the search that hides
// settings by their text, and keyboard use (links, Esc clears the search).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { NAV, flatNav, currentSection, fold, filterSettings, buildNav } = await import("../src/settings/nav.ts");
const { h } = await import("../src/views/dom.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("the sidebar lists the sections in the asked order, Writing with its three parts", () => {
  assert.deepEqual(NAV.map((e) => e.label), ["General", "Presence", "Writing", "Ask Glim", "Calendar", "Privacy", "Memory", "Shortcuts", "Claude Code & Agents", "Phone", "About"]);
  assert.deepEqual(NAV[2].children.map((e) => e.label), ["Apps", "Tones", "Strictness"]);
  // Every entry's target exists in the Settings code.
  const src = read("src/settings/main.ts") + read("src/settings/writing.ts") + read("src/settings/phone.ts");
  for (const e of flatNav()) assert.ok(src.includes(`"${e.id}"`), e.id);
});

test("the page's sections follow the sidebar's order", () => {
  const main = read("src/settings/main.ts");
  const order = main.slice(main.indexOf("sections = ["), main.indexOf("];", main.indexOf("sections = [")));
  const at = (s) => order.indexOf(s);
  assert.ok(at("general") < at("presence") && at("presence") < at('"settings-writing"') && at('"settings-writing"') < at("calendar,") && at("calendar,") < at("privacy")
    && at("privacy") < at("shortcuts") && at("shortcuts") < at('"settings-agents"') && at('"settings-agents"') < at("phoneEl") && at("phoneEl") < at("about"), order);
});

test("the current entry is the last section whose top has reached the top of the view", () => {
  const tops = (...ts) => ["a", "b", "c"].map((id, i) => ({ id, top: ts[i] }));
  assert.equal(currentSection(tops(0, 400, 900)), "a");
  assert.equal(currentSection(tops(-300, 20, 500)), "b");
  assert.equal(currentSection(tops(-900, -400, 10)), "c");
  assert.equal(currentSection(tops(50, 400, 900)), "a", "nothing scrolled yet: the first");
  assert.equal(currentSection([]), null);
});

test("search matches case- and accent-insensitively", () => {
  assert.equal(fold("  Écran  Principal "), "ecran principal");
  assert.ok(fold("Démarrage au lancement").includes(fold("demarrage")));
});

function page() {
  const general = h("section", { id: "settings-general" },
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" }, h("label", { text: "Launch at startup" })),
    h("div", { class: "row" }, h("label", { text: "Language" })),
  );
  const word = h("div", { class: "pill-row" }, h("span", { class: "name", text: "Microsoft Word" }));
  const notepad = h("div", { class: "pill-row" }, h("span", { class: "name", text: "Notepad" }));
  const apps = h("div", { class: "pill-group" }, h("h3", { text: "Apps" }), word, notepad);
  const strict = h("div", { class: "settings-group" }, h("div", { class: "row" }, h("label", { text: "Strictness" })));
  const writing = h("div", { class: "settings-group", id: "settings-writing" },
    h("section", {}, h("h2", {}, h("span", { text: "Writing" })), apps, strict),
  );
  return { general, writing, word, notepad, apps, strict };
}

test("search hides what doesn't match, keeps a matching heading's whole block, and comes back", () => {
  const p = page();
  const all = [p.general, p.writing];
  const shown = (el) => !el.hidden;

  for (const s of all) filterSettings(s, "language");
  assert.ok(shown(p.general) && !shown(p.writing));
  assert.deepEqual(p.general.children.map(shown), [true, false, true], "the heading stays with its match");

  for (const s of all) filterSettings(s, "WORD");
  assert.ok(!shown(p.general) && shown(p.writing) && shown(p.apps) && shown(p.word) && !shown(p.notepad) && !shown(p.strict));

  for (const s of all) filterSettings(s, "apps");
  assert.ok(shown(p.word) && shown(p.notepad), "a matching heading keeps its whole block");

  for (const s of all) filterSettings(s, "writing");
  assert.ok(shown(p.apps) && shown(p.strict));

  for (const s of all) filterSettings(s, "");
  assert.ok([p.general, ...p.general.children, p.writing, p.word, p.notepad, p.strict].every(shown));
});

test("the sidebar's links scroll and mark; typing filters; Esc clears; nothing found says so", () => {
  const p = page();
  const went = [];
  const nav = buildNav((id) => went.push(id), () => [p.general, p.writing]);
  const links = nav.el.find("a");
  assert.equal(links.length, flatNav().length);
  assert.ok(links.every((a) => a.getAttribute("href") === `#${a.getAttribute("data-target")}`), "real links: Tab and Enter work");
  assert.equal(nav.el.tagName, "NAV");
  assert.equal(nav.el.getAttribute("aria-label"), "Settings sections");
  assert.equal(nav.search.getAttribute("aria-label"), "Search settings");

  const strictness = links.find((a) => a.textContent === "Strictness");
  const ev = strictness.fire("click");
  assert.ok(ev.defaultPrevented);
  assert.deepEqual(went, ["settings-writing-strictness"]);
  assert.ok(strictness.classList.contains("current"));
  assert.equal(strictness.getAttribute("aria-current"), "true");
  nav.mark("settings-general");
  assert.equal(strictness.getAttribute("aria-current"), "false");

  nav.search.value = "notepad";
  nav.search.fire("input");
  assert.ok(p.general.hidden && !p.notepad.hidden && p.word.hidden);
  assert.equal(nav.empty.hidden, true);

  nav.search.value = "zzz";
  nav.search.fire("input");
  assert.equal(nav.empty.hidden, false);
  assert.equal(nav.empty.textContent, "No setting matches “zzz”.");

  let prevented = false;
  for (const fn of nav.search.listeners.get("keydown")) fn({ key: "Escape", preventDefault() { prevented = true; } });
  assert.ok(prevented);
  assert.equal(nav.search.value, "");
  assert.ok(!p.general.hidden && !p.word.hidden && nav.empty.hidden);
});

test("Ctrl+F goes to the search, and a jump focuses the section's heading", () => {
  const main = read("src/settings/main.ts");
  assert.ok(main.includes('ev.key.toLowerCase() === "f"'));
  assert.ok(main.includes('heading.setAttribute("tabindex", "-1")'));
});
