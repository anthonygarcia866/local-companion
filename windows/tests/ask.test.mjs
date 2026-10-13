// "Check writing in [app]?" (1d, docs/phase1-plan.md §14.2): where the
// question shows (now, in the ember, behind an approval, or not at all), the
// card's three buttons send only the app's exe and the answer, the card
// closes itself after 30 s as "Not now", and the ember shows "?".

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { askPlacement, AskClock, ASK_TIMEOUT_MS } = await import("../src/core/ask.ts");
const { buildAsk, answerAsk, askClock } = await import("../src/views/ask.ts");
const { emberWriting, PresenceModel } = await import("../src/island/presence.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const ACROBAT = { exe: "Acrobat.exe", name: "Adobe Acrobat" };
const place = (o) => askPlacement({ presence: "pill", paused: false, approval: false, current: null, incoming: ACROBAT, ...o });

test("the question shows now, in the ember, behind a card, or not at all", () => {
  assert.equal(place({}), "show");
  assert.equal(place({ presence: "ember" }), "ember");
  assert.equal(place({ approval: true }), "queue", "it waits behind an approval or question card");
  assert.equal(place({ presence: "hidden" }), "drop", "Hidden, or a fullscreen app in front");
  assert.equal(place({ presence: "indicator" }), "drop");
  assert.equal(place({ paused: true }), "drop");
  assert.equal(place({ current: { exe: "ACROBAT.EXE", name: "x" } }), "keep");
  assert.equal(place({ current: { exe: "Zed.exe", name: "Zed" } }), "drop", "one question at a time");
});

test("it waits behind an approval card: the lantern click opens the approval first", () => {
  State.writingAsk = ACROBAT;
  State.writing = [{}];
  State.pendingApproval = { questions: null };
  assert.equal(State.lanternView(), "approval");
  State.pendingApproval = null;
  assert.equal(State.lanternView(), "writingAsk", "then the question, before the suggestions");
  State.writingAsk = null;
  assert.equal(State.lanternView(), "suggestions");
  State.writing = [];
});

function card() {
  const calls = [];
  const v = buildAsk({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  return { v, calls };
}

test("the buttons send the app's exe and the answer, nothing else, and the card goes", () => {
  for (const [label, answer] of [["Yes", "yes"], ["Not now", "later"], ["Never", "never"]]) {
    State.writingAsk = { ...ACROBAT };
    State.view = "writingAsk";
    State.mode = "expanded";
    const { v, calls } = card();
    v.sync();
    assert.ok(v.el.textContent.includes("Check writing in Adobe Acrobat?"), v.el.textContent);
    const n = sent("writing_ask_answer").length;
    v.el.find("button").find((b) => b.textContent === label).fire("click");
    assert.deepEqual(sent("writing_ask_answer").slice(n), [{ exe: "Acrobat.exe", answer }]);
    assert.equal(State.writingAsk, null);
    assert.deepEqual(calls, [["collapse"]], "nothing else waiting: out of the way");
    assert.equal(askClock.running(), false);
  }
  State.mode = "hidden";
});

test("the card closes itself after 30 s, as Not now", async () => {
  assert.equal(ASK_TIMEOUT_MS, 30_000);
  const clock = new AskClock();
  let fired = 0;
  clock.start("Acrobat.exe", () => fired++, 5);
  clock.start("Acrobat.exe", () => fired++, 5); // already running for that app
  await new Promise((r) => setTimeout(r, 30));
  assert.equal(fired, 1);
  // The card's own clock starts when it is in sight, and answers "later".
  State.writingAsk = { ...ACROBAT };
  State.view = "writingAsk";
  State.mode = "expanded";
  const { v } = card();
  v.sync();
  assert.equal(askClock.running(), true);
  const n = sent("writing_ask_answer").length;
  answerAsk("later", null);
  assert.deepEqual(sent("writing_ask_answer").slice(n), [{ exe: "Acrobat.exe", answer: "later" }]);
  State.mode = "hidden";
});

test("the ember shows ? for a question, and its click opens it", () => {
  assert.deepEqual(emberWriting(0, true), { lit: true, badge: "?" });
  assert.deepEqual(emberWriting(3, true), { lit: true, badge: "?" });
  assert.deepEqual(emberWriting(3, false), { lit: true, badge: "3" });
  let opened = 0;
  const m = new PresenceModel({ showPill() {}, openSuggestions() { opened++; } });
  m.apply("ember");
  m.ask = true;
  m.restore();
  m.apply("pill");
  assert.equal(opened, 1, "a peek that opens on the question");
});

test("the island sends only the exe and the answer; no window title or text anywhere", () => {
  const src = read("src/views/ask.ts") + read("src/core/ask.ts");
  assert.deepEqual(src.match(/Bridge\.\w+\([^)]*\)/g), ["Bridge.writingAskAnswer(ask.exe, answer)"]);
  for (const banned of ["title:", "windowTitle", "console.", "localStorage"]) assert.ok(!src.includes(banned), banned);
});
