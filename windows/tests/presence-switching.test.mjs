// Switching between Pill, Ember and Hidden, in every order, always ends with
// the right thing on screen (live-test bugs 2026-10-10: Settings → Pill showed
// nothing; Ctrl+Alt+H needed two presses from Hidden; clicking the ember made
// it vanish). All three were the island's state machine sitting folded in its
// invisible wake strip while Rust said "pill".

import { afterEach, beforeEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { sent } from "./tauri.mjs";
import { IslandStateMachine } from "../src/island/fsm.ts";
import { PresenceModel } from "../src/island/presence.ts";

const KINDS = ["pill", "ember", "hidden"];
const MODE_OF = { normal: "pill", ember: "ember", hidden: "hidden" };
const NEXT = { normal: "ember", ember: "hidden", hidden: "normal" };

beforeEach(() => mock.timers.enable({ apis: ["setTimeout"] }));
afterEach(() => mock.timers.reset());

/** The island as main.ts builds it: Glim's FSM, and presence revealing the pill. */
function island(start) {
  const fsm = new IslandStateMachine();
  fsm.foldsToHidden = false;
  if (start !== "hidden") {
    fsm.launch();
    fsm.greetComplete();
    mock.timers.tick(1000);
    if (start === "home") fsm.click();
  }
  const model = new PresenceModel({ showPill: () => fsm.reveal() });
  return { fsm, model };
}

/** What the user sees: the pill (island drawn, not folded), the dot, or nothing. */
function onScreen({ fsm, model }) {
  if (model.kind === "pill") return fsm.state === "hidden" ? "nothing (folded strip)" : "pill";
  if (model.kind === "ember") return "ember";
  return "nothing";
}
const expected = (kind) => (kind === "hidden" ? "nothing" : kind);

function* sequences(len) {
  if (len === 0) return yield [];
  for (const rest of sequences(len - 1)) for (const k of KINDS) yield [...rest, k];
}

test("every order of Pill, Ember and Hidden ends with the right thing on screen", () => {
  let checked = 0;
  for (const start of ["hidden", "petit", "home"]) {
    for (let len = 1; len <= 4; len++) {
      for (const seq of sequences(len)) {
        const g = island(start);
        for (const kind of seq) {
          g.model.apply(kind);
          assert.equal(onScreen(g), expected(kind), `${start}: ${seq.join(" → ")} at ${kind}`);
          // A long while between switches: nothing may fold it meanwhile.
          mock.timers.tick(10 * 60_000);
          assert.equal(onScreen(g), expected(kind), `${start}: ${seq.join(" → ")}, ${kind} after 10 min`);
        }
        checked++;
      }
    }
  }
  assert.equal(checked, 3 * (3 + 9 + 27 + 81));
});

test("Ctrl+Alt+H: one press from Hidden brings the pill back, even after a long idle", () => {
  const g = island("petit");
  let vis = "normal";
  for (let press = 0; press < 9; press++) {
    vis = NEXT[vis];
    g.model.apply(MODE_OF[vis]);
    assert.equal(onScreen(g), expected(MODE_OF[vis]), `press ${press + 1} → ${vis}`);
    mock.timers.tick(5 * 60_000);
  }
});

test("clicking the ember asks for Normal, and Normal shows the pill", () => {
  const g = island("hidden");
  g.model.apply("ember");
  mock.timers.tick(10 * 60_000);
  const before = sent("set_visibility").length;
  g.model.restore();
  assert.deepEqual(sent("set_visibility").slice(before), [{ visibility: "normal" }]);
  // Rust answers with "pill".
  g.model.apply("pill");
  assert.equal(onScreen(g), "pill");
});

test("the ember's click is wired to restore", () => {
  const src = readFileSync(new URL("../src/island/presence.ts", import.meta.url), "utf8");
  assert.match(src, /this\.el\.addEventListener\("click", \(\) => \{\s+if \(gesture\.click\(\)\) this\.model\.restore\(\);/);
  const main = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
  assert.match(main, /new PresenceView\(\{ showPill: \(\) => \{ if \(!State\.paused\) island\.reveal\(\); \} \}\)/);
});
