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
const { buildSuggestions, SUGGESTIONS_SHOWN, suggestionPage, suggestionNote } = await import("../src/views/views.ts");
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
  assert.equal(view.el.find(".suggestion-pages")[0].style.display, "none");
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
  assert.equal(view.el.find(".suggestion-pages")[0].style.display, "");
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

test("the suggestion list sends ids only and never logs", () => {
  const views = read("src/views/views.ts");
  const start = views.indexOf("// ── Writing suggestions");
  const section = views.slice(start, views.indexOf("// ── In-island settings", start));
  assert.ok(section.length > 200);
  for (const banned of ["addEventListener", "console.", "keydown", "localStorage"]) {
    assert.ok(!section.includes(banned), `the suggestion list uses ${banned}`);
  }
  // Its only calls to Rust: a fix, Apply all, a copy, dismiss; with ids, never words.
  assert.deepEqual(section.match(/Bridge\.\w+\([^)]*\)/g), [
    "Bridge.writingDismiss()",
    "Bridge.writingFixAll(fix.checkId)",
    "Bridge.writingFix(fix.checkId, index, choice)",
    "Bridge.writingCopy(fix.checkId, index, choice)",
  ]);
  // The event handlers only store and redraw.
  const main = read("src/main.ts");
  for (const event of ['"writing-suggestions"', '"writing-fix"']) {
    const handler = main.slice(main.indexOf(event));
    const body = handler.slice(0, handler.indexOf("\n  });") + 5);
    assert.ok(body.length > 50, event);
    for (const banned of ["Bridge.", "console.", "localStorage"]) assert.ok(!body.includes(banned), `${event} uses ${banned}`);
  }
});

// ── 1b: fixes ────────────────────────────────────────────────────────────────

/** A list of 5 in `mode`, drawn on its second page when `page2`. */
function listIn(mode, page2 = false) {
  State.writing = [item("one"), item("two"), item("three"), item("tomorow", { replacements: ["tomorrow", ""] }), item("five")];
  State.writingFix = { checkId: 41, mode, app: "Word" };
  State.writingResult = null;
  State.writingBusy = false;
  const view = buildSuggestions();
  view.sync();
  if (page2) view.el.find(".suggestion-page")[1].fire("click");
  return view;
}
const fixButtons = (view) => view.el.find(".fix-btn");

test("a fix button sends which list, row and replacement it is, and waits for the answer", () => {
  const view = listIn("apply", true);
  const buttons = fixButtons(view);
  // Page 2: "tomorow" (row 3 of the whole list) with "tomorrow" and Remove it; "five".
  assert.deepEqual(buttons.map((b) => b.getAttribute("aria-label")), ["Replace with “tomorrow”", "Remove it", "Replace with “received”", "Replace with “relieved”"]);
  assert.match(view.el.find(".suggestion-more")[0].textContent, /^Click a fix to apply it\. Ctrl\+Z in Word undoes it\.$/);
  const n = sent("writing_fix").length;
  buttons[1].fire("click");
  assert.deepEqual(sent("writing_fix").slice(n), [{ checkId: 41, index: 3, choice: 1 }]);
  // Waiting: "Fixing…", every button off, a second click sends nothing.
  assert.equal(State.writingBusy, true);
  assert.equal(view.el.find(".suggestion-more")[0].textContent, "Fixing…");
  assert.ok(fixButtons(view).every((b) => b.disabled));
  fixButtons(view)[0].fire("click");
  assert.equal(sent("writing_fix").length, n + 1);
  State.writingBusy = false;
});

test("away from the field the buttons are off and say why; view-only apps show plain chips", () => {
  let view = listIn("away");
  assert.ok(fixButtons(view).length > 0 && fixButtons(view).every((b) => b.disabled));
  assert.equal(view.el.find(".suggestion-more")[0].textContent, "Click in the field first.");
  view = listIn("view");
  assert.equal(fixButtons(view).length, 0);
  assert.ok(view.el.find(".fix").length > 0);
  assert.equal(view.el.find(".suggestion-more")[0].textContent, "Suggestions only in Word for now.");
});

test("in a field Glim can't type in, each replacement is a Copy button the user clicks", () => {
  const view = listIn("copy", true);
  const buttons = fixButtons(view);
  // A removal has nothing to copy: a plain chip.
  assert.deepEqual(buttons.map((b) => b.getAttribute("aria-label")), ["Copy “tomorrow”", "Copy “received”", "Copy “relieved”"]);
  assert.equal(view.el.find(".suggestion-more")[0].textContent, "Glim can't type in this field. Click a fix to copy it.");
  const n = sent("writing_copy").length;
  buttons[0].fire("click");
  assert.deepEqual(sent("writing_copy").slice(n), [{ checkId: 41, index: 3, choice: 0 }]);
  assert.equal(State.writingBusy, false, "a copy doesn't wait");
});

test("Dismiss asks Rust to drop the list", () => {
  const view = listIn("apply");
  const n = sent("writing_dismiss").length;
  view.el.find(".suggestion-dismiss")[0].fire("click");
  assert.equal(sent("writing_dismiss").length, n + 1);
  assert.equal(view.el.find(".suggestion-dismiss")[0].getAttribute("aria-label"), "Dismiss suggestions");
});

test("every outcome Rust can send has its own note", () => {
  const fix = { checkId: 1, mode: "apply", app: "Word" };
  const notes = ["applied", "copied", "changed", "away", "paused", "blocked", "elevated", "keys-held", "unsupported", "not-clean", "clipboard-busy"]
    .map((outcome) => suggestionNote(fix, { outcome, app: "Notepad" }, false));
  assert.equal(new Set(notes).size, notes.length);
  assert.equal(notes[0], "Fixed. Ctrl+Z in Notepad undoes it.");
  assert.equal(notes[9], "That didn't apply cleanly. Press Ctrl+Z in Notepad to undo it.");
  for (const n of notes) assert.ok(!n.startsWith("Click a fix"), n);
  // The codes are the ones Rust sends (writing::Outcome::code).
  const rust = read("src-tauri/src/writing/mod.rs");
  for (const code of ["applied", "copied", "changed", "away", "paused", "blocked", "elevated", "keys-held", "unsupported", "not-clean", "clipboard-busy"]) {
    assert.ok(rust.includes(`=> "${code}",`), code);
  }
});

test("Apply all: one click sends the list id only, says it uses each first option, and waits for every fix", () => {
  const view = listIn("apply");
  const all = view.el.find(".apply-all")[0];
  assert.equal(all.style.display, "");
  assert.equal(all.textContent, "Apply all");
  assert.equal(all.getAttribute("title"), "Applies each suggestion's first option, last to first. Ctrl+Z undoes one fix per press.");
  const n = sent("writing_fix_all").length;
  all.fire("click");
  assert.deepEqual(sent("writing_fix_all").slice(n), [{ checkId: 41 }]);
  assert.equal(State.writingBusy, true);
  assert.equal(view.el.find(".suggestion-more")[0].textContent, "Fixing…");
  assert.ok(view.el.find(".apply-all")[0].disabled && fixButtons(view).every((b) => b.disabled));
  all.fire("click");
  assert.equal(sent("writing_fix_all").length, n + 1, "a second click sends nothing");
  State.writingBusy = false;
});

test("Apply all shows only where fixes are typed, for two or more, and is off away from the field", () => {
  for (const [mode, shown, off] of [["apply", true, false], ["away", true, true], ["copy", false, false], ["view", false, false]]) {
    const all = listIn(mode).el.find(".apply-all")[0];
    assert.equal(all.style.display === "", shown, mode);
    if (shown) assert.equal(all.disabled, off, mode);
  }
  State.writing = [item("one")];
  State.writingFix = { checkId: 41, mode: "apply", app: "Word" };
  const view = buildSuggestions();
  view.sync();
  assert.equal(view.el.find(".apply-all")[0].style.display, "none", "one suggestion: its own button is enough");
});

test("after Apply all the note says how many went in, that Ctrl+Z undoes one per press, and why it stopped", () => {
  const fix = { checkId: 1, mode: "apply", app: "Word" };
  assert.equal(suggestionNote(fix, { outcome: "applied", app: "Outlook", done: 3, total: 3 }, false), "Fixed 3 of 3. Ctrl+Z in Outlook undoes one fix per press.");
  assert.equal(suggestionNote(fix, { outcome: "away", app: "Outlook", done: 2, total: 3 }, false), "Fixed 2 of 3. Ctrl+Z in Outlook undoes one fix per press. Click in the field first.");
  assert.equal(suggestionNote(fix, { outcome: "not-clean", app: "Word", done: 1, total: 3 }, false), "Fixed 1 of 3. Ctrl+Z in Word undoes one fix per press. That didn't apply cleanly. Press Ctrl+Z in Word to undo it.");
  // Nothing went in: just why.
  assert.equal(suggestionNote(fix, { outcome: "changed", app: "Word", done: 0, total: 3 }, false), "The text changed, so nothing was fixed. Checked again.");
  // A single fix's answer has no counts.
  assert.equal(suggestionNote(fix, { outcome: "applied", app: "Word", done: 0, total: 0 }, false), "Fixed. Ctrl+Z in Word undoes it.");
  // Rust sends the counts (writing::FixResult).
  const rust = read("src-tauri/src/writing/mod.rs");
  assert.match(rust, /pub done: usize,\r?\n\s+pub total: usize,/);
});

test("Rust's answer clears Fixing… and shows the outcome; Pause reaches Rust", () => {
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('"writing-fix"'));
  assert.match(handler, /State\.writingBusy = false;\n\s+State\.writingResult = result;/);
  assert.match(main, /State\.paused = on;\n\s+\/\/ [^\n]*\n\s+void Bridge\.writingSetPaused\(on\);/);
  // The last fix empties the list: the view stays, so "Fixed…" is seen.
  assert.match(main, /State\.view === "suggestions" && State\.writingResult\?\.outcome !== "applied"\) State\.view = State\.defaultView\(\);/);
  assert.equal(suggestionNote({ checkId: 0, mode: "view", app: "" }, { outcome: "applied", app: "Notepad" }, false), "Fixed. Ctrl+Z in Notepad undoes it.");
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

test("the lantern opens the suggestions over an agent's card; a waiting approval still wins", () => {
  State.pendingApproval = null;
  State.writing = [];
  assert.equal(State.lanternView(), null);
  // An agent finished and its card took the island; the badge shows 4.
  State.view = "finished";
  State.writing = [item("a"), item("b"), item("c"), item("d")];
  assert.equal(State.lanternView(), "suggestions");
  assert.equal(State.defaultView(), "suggestions");
  // Approvals and questions are urgent: they come first.
  State.pendingApproval = { requestId: "r", pillId: "p", questions: null };
  assert.equal(State.lanternView(), "approval");
  State.pendingApproval = { requestId: "r", pillId: "p", questions: [{ question: "?" }] };
  assert.equal(State.lanternView(), "question");
  State.pendingApproval = null;
  State.writing = [];
  State.view = "overview";
});

test("the lantern is a button: mouse, Invoke and keys all go through activateLantern", () => {
  const island = read("src/island/island.ts");
  // Lantern draws its SVG aria-hidden: the island's must be found by screen
  // readers and UI Automation (round 3: the self-test couldn't find it).
  assert.match(island, /this\.lantern\.el\.removeAttribute\("aria-hidden"\);/);
  assert.match(island, /this\.lantern\.el\.setAttribute\("role", "button"\);/);
  assert.match(island, /this\.lantern\.el\.setAttribute\("aria-label", "Glim"\);/);
  assert.match(island, /this\.lantern\.el\.setAttribute\("tabindex", "0"\);/);
  assert.match(island, /this\.lantern\.el\.addEventListener\("click", \(\) => this\.activateLantern\(\)\);/);
  assert.match(island, /if \(e\.key !== "Enter" && e\.key !== " "\) return;\n\s+e\.preventDefault\(\);\n\s+this\.activateLantern\(\);/);
  // #bot-lantern is pointer-events: none, so the mouse is hit-tested on mouseup.
  assert.match(read("src/style.css"), /#bot-lantern \{\n  position: absolute;\n  pointer-events: none;/);
  assert.match(island, /if \(e\.button === 0 && this\.botPress && State\.mode === "expanded" && this\.isBotHit\(e\.clientX, e\.clientY\)\) \{\n\s+this\.activateLantern\(\);/);
  // Closed pill: open (on defaultView). Open island: lanternView.
  assert.match(island, /activateLantern\(\) \{\n\s+if \(this\.uploadActive\) return;\n\s+if \(State\.mode !== "expanded"\) \{\n\s+this\.fsm\.click\(\);\n\s+return;\n\s+\}\n\s+const view = State\.lanternView\(\);\n\s+if \(view && view !== State\.view\) this\.setView\(view\);/);
  // Hidden from readers only while it's invisible.
  assert.match(island, /const hidden = visible \? "false" : "true";/);
  assert.match(read("src/core/state.ts"), /return this\.lanternView\(\) \?\? \(this\.tasks\.length === 0 \? "empty" : "overview"\);/);
});

test("the pager's buttons are at least 28 px", () => {
  const css = read("src/style.css");
  const rule = css.slice(css.indexOf(".suggestion-page {"), css.indexOf("}", css.indexOf(".suggestion-page {")));
  const px = (prop) => Number(rule.match(new RegExp(`\\n  ${prop}: (\\d+)px;`))[1]);
  assert.ok(px("width") >= 28 && px("height") >= 28, rule);
});
