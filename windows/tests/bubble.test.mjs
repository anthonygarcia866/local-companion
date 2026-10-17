// The mini lantern's popover (1e, docs/phase1-plan.md §7a.3), logic only:
// open/close, paging 3 at a time in the list's reading order, the 20 s
// auto-close that never runs with the pointer over it, and "Rewrite…"
// routed to the island's existing 1c path. No window exists yet.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const { stepBubble, bubbleRows, BUBBLE_CLOSED, BUBBLE_AUTO_CLOSE_MS } = await import("../src/core/bubble.ts");
const { suggestionPage, SUGGESTIONS_SHOWN } = await import("../src/core/paging.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

/** Runs events from the closed state; returns the last state and every effect. */
function run(events, from = BUBBLE_CLOSED) {
  let state = from;
  const effects = [];
  for (const e of events) {
    const out = stepBubble(state, e);
    state = out.state;
    if (out.effect) effects.push(out.effect);
  }
  return { state, effects };
}

const twelve = Array.from({ length: 12 }, (_, i) => `s${i + 1}`);

test("decided 2026-10-13: 20 s auto-close, 3 a page", () => {
  assert.equal(BUBBLE_AUTO_CLOSE_MS, 20_000);
  assert.equal(SUGGESTIONS_SHOWN, 3);
});

test("a click on the mini lantern opens it on the first page; another click closes it", () => {
  let { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }]);
  assert.equal(state.open, true);
  assert.equal(state.page, 0);
  state = run([{ kind: "turn", by: 2 }, { kind: "lantern-click", now: 5 }], state).state;
  assert.equal(state.open, false);
  state = run([{ kind: "lantern-click", now: 9 }], state).state;
  assert.equal(state.page, 0, "reopens on the first page");
});

test("Close, the caret leaving the field, or the field losing focus close it", () => {
  for (const e of [{ kind: "close" }, { kind: "field-left" }]) {
    const { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }, e]);
    assert.equal(state.open, false, e.kind);
  }
});

test("paging: three rows a page in reading order, clamped at both ends", () => {
  let { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }]);
  const page = () => bubbleRows(twelve, state.page);
  assert.deepEqual(page().rows, ["s1", "s2", "s3"]);
  assert.equal(page().canPrev, false);
  state = run([{ kind: "turn", by: 1 }], state).state;
  assert.deepEqual(page().rows, ["s4", "s5", "s6"]);
  assert.deepEqual([page().from, page().to, page().total], [4, 6, 12]);
  state = run([{ kind: "turn", by: 9 }], state).state;
  assert.deepEqual(page().rows, ["s10", "s11", "s12"]);
  assert.equal(page().canNext, false);
  state = run([{ kind: "turn", by: -9 }], state).state;
  assert.equal(state.page, 0);
  // The same paging as the island's list.
  for (const [n, p] of [[0, 0], [3, 0], [12, 3], [11, 9], [11, -2]]) {
    const r = bubbleRows(Array(n).fill(0), p);
    const at = suggestionPage(n, p);
    assert.deepEqual([r.from, r.to], [at.from, at.to]);
  }
  assert.equal(bubbleRows(["a", "b"], 0).paged, false, "no pager for one page");
});

test("turning pages does nothing while closed", () => {
  const { state } = run([{ kind: "list", items: twelve }, { kind: "turn", by: 2 }]);
  assert.equal(state.page, 0);
});

test("a new check starts on page one; the same list keeps the page; an empty list closes it", () => {
  let { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }, { kind: "turn", by: 2 }]);
  state = run([{ kind: "list", items: twelve }], state).state;
  assert.equal(state.page, 2, "same list");
  state = run([{ kind: "list", items: twelve.slice(1) }], state).state;
  assert.equal(state.page, 0, "a new list");
  assert.equal(state.open, true);
  state = run([{ kind: "list", items: [] }], state).state;
  assert.equal(state.open, false, "the last one fixed or ignored");
  state = run([{ kind: "lantern-click", now: 1 }], state).state;
  assert.equal(state.open, false, "nothing to open on");
});

test("20 s with no pointer over it closes it", () => {
  let { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 1000 }]);
  state = run([{ kind: "tick", now: 1000 + BUBBLE_AUTO_CLOSE_MS - 1 }], state).state;
  assert.equal(state.open, true);
  state = run([{ kind: "tick", now: 1000 + BUBBLE_AUTO_CLOSE_MS }], state).state;
  assert.equal(state.open, false);
});

test("the clock never runs while the pointer is over it, and restarts when it leaves", () => {
  let { state } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }, { kind: "pointer", over: true, now: 5_000 }]);
  state = run([{ kind: "tick", now: 120_000 }], state).state;
  assert.equal(state.open, true, "pointer over it: two minutes, still open");
  state = run([{ kind: "pointer", over: false, now: 120_000 }, { kind: "tick", now: 139_999 }], state).state;
  assert.equal(state.open, true, "19.999 s after it left");
  state = run([{ kind: "tick", now: 140_000 }], state).state;
  assert.equal(state.open, false);
});

test("pointer moves and ticks while closed change nothing", () => {
  const before = run([{ kind: "list", items: twelve }]).state;
  const after = run([{ kind: "pointer", over: true, now: 3 }, { kind: "tick", now: 99_999 }], before).state;
  assert.deepEqual(after, before);
});

test("Rewrite… closes the popover and routes to the island's existing Rewrite path", () => {
  const { state, effects } = run([{ kind: "list", items: twelve }, { kind: "lantern-click", now: 0 }, { kind: "rewrite" }]);
  assert.equal(state.open, false);
  assert.deepEqual(effects, ["rewrite"]);
  // The effect is the hotkey's own action id: Rust's shortcuts::dispatch hands
  // "rewrite" to the island, whose handler opens the 1c tone card.
  assert.match(read("src-tauri/src/shortcuts.rs"), /action\("rewrite", "Ctrl\+Alt\+Shift\+R"/);
  assert.match(read("src/island/shortcuts.ts"), /case "rewrite": \{[\s\S]*?rewriteKey\(/);
});

test("the state machine is pure: no DOM, no Bridge, no timers, nothing logged", () => {
  const src = read("src/core/bubble.ts");
  for (const banned of ["document", "window.", "Bridge", "setTimeout", "setInterval", "console.", "invoke("]) {
    assert.ok(!src.includes(banned), banned);
  }
});

test("the setting defaults to off on both sides", async () => {
  const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");
  assert.equal(DEFAULT_SETTINGS.writingBubble, false);
  assert.match(read("src-tauri/src/settings.rs"), /writing_bubble: false,/);
});
