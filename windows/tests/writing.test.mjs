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
const { buildSuggestions, SUGGESTIONS_SHOWN } = await import("../src/views/views.ts");

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
  assert.ok(texts(".suggestion-more").includes("+2 more"));
  State.writing = [];
  view.sync();
  assert.equal(view.el.find(".suggestion").length, 0);
  assert.equal(texts(".title")[0], "Nothing to fix here.");
});

test("the suggestion list is view only and never logs", () => {
  const views = read("src/views/views.ts");
  const start = views.indexOf("// ── Writing suggestions");
  const section = views.slice(start, views.indexOf("// ── In-island settings", start));
  assert.ok(section.length > 200);
  for (const banned of ["onclick", "addEventListener", "Bridge.", "console.", "keydown", "localStorage"]) {
    assert.ok(!section.includes(banned), `the suggestion list uses ${banned}`);
  }
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
