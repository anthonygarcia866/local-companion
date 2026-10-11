// The writing checker in the island (Phase 1a): the lantern shares s-suggest
// with agents (agent states win), the island opens on the suggestion list,
// the list is view only, and suggestion text never goes to a log.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { withWritingSuggestions, lanternStateFor, shouldPop } = await import("../src/mascot/lantern.ts");
const { State } = await import("../src/core/state.ts");
const { buildSuggestions, SUGGESTIONS_SHOWN, suggestionPage } = await import("../src/views/views.ts");
const { PresenceModel, emberWriting } = await import("../src/island/presence.ts");
const { sent, calls } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

const item = (problem, extra = {}) => ({
  start: 3, end: 3 + problem.length, problem, before: "We ", after: " the payment.", kind: "Spelling",
  message: `Did you mean to spell \`${problem}\` this way?`, replacements: ["received", "relieved"], ...extra,
});

test("writing suggestions show s-suggest only while no agent state is showing", () => {
  for (const calm of ["idle", "listen", "done"]) assert.equal(withWritingSuggestions(calm, 2), "suggest");
  // Agent states win: an approval or question, work, an error, a rate limit.
  for (const agent of ["suggest", "think", "error", "paused"]) assert.equal(withWritingSuggestions(agent, 2), agent);
  // No suggestions: the app's own state.
  for (const s of ["idle", "listen", "done", "think"]) assert.equal(withWritingSuggestions(s, 0), s);
  // An agent waiting on approval stays s-suggest, from the agent.
  assert.equal(withWritingSuggestions(lanternStateFor("approval", false), 3), "suggest");
  assert.equal(withWritingSuggestions(lanternStateFor("working", true), 3), "think");
});

test("the pop plays when suggestions appear, not on every recount", () => {
  assert.equal(shouldPop("idle", withWritingSuggestions("idle", 1)), true);
  assert.equal(shouldPop(withWritingSuggestions("idle", 1), withWritingSuggestions("idle", 4)), false);
});

test("the island opens on the suggestions, after a waiting approval", () => {
  State.pendingApproval = null;
  State.writing = [item("recieved")];
  assert.equal(State.defaultView(), "suggestions");
  State.pendingApproval = { requestId: "r", pillId: "p", questions: null };
  assert.equal(State.defaultView(), "approval");
  State.pendingApproval = null;
  State.writing = [];
  assert.notEqual(State.defaultView(), "suggestions");
});

test("the list shows the problem in context, the message and the replacements", () => {
  State.writing = [item("recieved"), item("tomorow", { replacements: ["tomorrow"] }), item("the the", { replacements: [""] }), item("dont"), item("posible")];
  const view = buildSuggestions();
  view.sync();
  const texts = (cls) => view.el.find(cls).map((e) => e.textContent);
  assert.equal(view.el.find(".suggestion").length, SUGGESTIONS_SHOWN);
  assert.deepEqual(texts(".problem"), ["recieved", "tomorow", "the the"]);
  assert.ok(texts(".suggestion-context")[0].startsWith("We recieved the payment."));
  assert.match(texts(".suggestion-message")[0], /Did you mean to spell `recieved`/);
  assert.deepEqual(texts(".fix").slice(0, 3), ["received", "relieved", "tomorrow"]);
  assert.equal(texts(".fix")[3], "Remove it");
  assert.ok(texts(".title")[0].startsWith("5 writing suggestions"));
  assert.deepEqual(texts(".suggestion-range"), ["1–3 of 5"]);
  State.writing = [];
  view.sync();
  assert.equal(view.el.find(".suggestion").length, 0);
  assert.equal(texts(".title")[0], "Nothing to fix here.");
  assert.equal(view.el.find(".suggestion-pager")[0].style.display, "none");
});

test("pages: three at a time, in the order given (nearest the caret first)", () => {
  assert.deepEqual(suggestionPage(0, 0), { page: 0, pages: 1, from: 0, to: 0 });
  assert.deepEqual(suggestionPage(3, 0), { page: 0, pages: 1, from: 1, to: 3 });
  assert.deepEqual(suggestionPage(12, 0), { page: 0, pages: 4, from: 1, to: 3 });
  assert.deepEqual(suggestionPage(12, 3), { page: 3, pages: 4, from: 10, to: 12 });
  assert.deepEqual(suggestionPage(11, 9), { page: 3, pages: 4, from: 10, to: 11 });
  assert.deepEqual(suggestionPage(11, -2), { page: 0, pages: 4, from: 1, to: 3 });
});

test("Next / Previous walk through 12 suggestions without scrolling", () => {
  const words = Array.from({ length: 12 }, (_, i) => `wrd${i}`);
  State.writing = words.map((w) => item(w));
  const view = buildSuggestions();
  view.sync();
  const problems = () => view.el.find(".problem").map((e) => e.textContent);
  const range = () => view.el.find(".suggestion-range")[0].textContent;
  const [prev, next] = view.el.find(".suggestion-page");
  assert.equal(view.el.find(".suggestion-pager")[0].style.display, "");
  assert.deepEqual([problems(), range(), prev.disabled, next.disabled], [words.slice(0, 3), "1–3 of 12", true, false]);
  for (let p = 1; p < 4; p++) {
    next.fire("click");
    assert.deepEqual(problems(), words.slice(p * 3, p * 3 + 3));
  }
  assert.deepEqual([range(), next.disabled, prev.disabled], ["10–12 of 12", true, false]);
  next.fire("click");
  assert.equal(range(), "10–12 of 12");
  prev.fire("click");
  assert.equal(range(), "7–9 of 12");
  // A recount of the same list keeps the page; a new list starts again at
  // the first page (nearest the caret).
  view.sync();
  assert.equal(range(), "7–9 of 12");
  State.writing = words.slice(0, 10).map((w) => item(w));
  view.sync();
  assert.equal(range(), "1–3 of 10");
  State.writing = [];
});

test("the suggestion list is view only and never logs", () => {
  const views = read("src/views/views.ts");
  const start = views.indexOf("// ── Writing suggestions");
  const section = views.slice(start, views.indexOf("// ── In-island settings", start));
  assert.ok(section.length > 200);
  for (const banned of ["addEventListener", "Bridge.", "console.", "keydown", "localStorage"]) {
    assert.ok(!section.includes(banned), `the suggestion list uses ${banned}`);
  }
  // Its only clicks turn pages.
  assert.deepEqual(section.match(/onclick: [^}]*/g), ["onclick: () => turn(-1) ", "onclick: () => turn(1) "]);
  const turn = section.slice(section.indexOf("const turn = "), section.indexOf("return { el, sync };"));
  assert.match(turn, /^const turn = \(by: number\) => \{\s+page \+= by;\s+sync\(\);\s+\};\s+$/);
  // The event handler only stores and redraws.
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('onEvent<{ items: WritingSuggestion[] }>("writing-suggestions"'));
  const body = handler.slice(0, handler.indexOf("\n  });") + 5);
  assert.ok(body.length > 50);
  for (const banned of ["Bridge.", "console.", "localStorage"]) assert.ok(!body.includes(banned), `the handler uses ${banned}`);
});

test("the pill's count is announced and visible to UI Automation", () => {
  // A plain div is left out of the accessibility tree: no screen reader, no
  // scripts/verify-writing-notepad.ps1.
  const island = read("src/island/island.ts");
  assert.match(island, /h\("div", \{ id: "writing-count", role: "status"/);
  assert.match(island, /badge\.setAttribute\("aria-label", tn\("\{count\} writing suggestion", "\{count\} writing suggestions", writing\)\)/);
});

test("the ember brightens and counts while suggestions wait (9+ past nine)", () => {
  assert.deepEqual(emberWriting(0), { lit: false, badge: "" });
  assert.deepEqual(emberWriting(1), { lit: true, badge: "1" });
  assert.deepEqual(emberWriting(9), { lit: true, badge: "9" });
  assert.deepEqual(emberWriting(14), { lit: true, badge: "9+" });
  const css = read("src/style.css");
  const lit = css.slice(css.indexOf("#ember.lit .ember-dot {"), css.indexOf("}", css.indexOf("#ember.lit .ember-dot {")));
  // Brighter, no bounce: no animation, no transform.
  assert.match(lit, /animation: none;/);
  assert.ok(!/transform|scale|translate/.test(lit));
});

test("clicking the ember with suggestions peeks at them, and closing goes back to the ember", () => {
  calls.length = 0;
  let opened = 0;
  let pills = 0;
  const model = new PresenceModel({ showPill: () => pills++, openSuggestions: () => opened++ });
  model.apply("ember");
  // Nothing waiting: the click is the old one, back to Normal.
  model.restore();
  assert.deepEqual(sent("set_visibility"), [{ visibility: "normal" }]);
  assert.deepEqual(sent("set_peek"), []);
  calls.length = 0;
  // Suggestions waiting: a peek, never a saved mode.
  model.writing = 4;
  model.restore();
  assert.deepEqual(sent("set_peek"), [{ on: true }]);
  assert.deepEqual(sent("set_visibility"), []);
  assert.equal(opened, 0);
  // Rust answers with the pill: the island opens on the list, once.
  model.apply("pill");
  assert.deepEqual([pills, opened], [1, 1]);
  model.apply("pill");
  assert.equal(opened, 1);
  // The island closes: the peek ends, Rust brings the ember back.
  model.islandClosed();
  assert.deepEqual(sent("set_peek"), [{ on: true }, { on: false }]);
  model.apply("ember");
  model.islandClosed();
  assert.equal(sent("set_peek").length, 2);
  // In Normal (no peek) a closing island sends nothing.
  model.apply("pill");
  model.islandClosed();
  assert.equal(sent("set_peek").length, 2);
  assert.equal(opened, 1);
});

test("the island tells presence when it closes", () => {
  const island = read("src/island/island.ts");
  assert.match(island, /State\.mode = mode;\n    if \(prev === "expanded"\) this\.onClosed\?\.\(\);/);
});
