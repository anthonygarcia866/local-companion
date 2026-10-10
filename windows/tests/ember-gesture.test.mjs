// The ember: a click brings the pill back, a drag moves it to another dock
// (live test 2026-10-10: the ember vanished on click, the lantern couldn't be moved).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { EmberGesture } from "../src/island/presence.ts";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("a click without moving restores the pill", () => {
  const g = new EmberGesture();
  g.down(14, 14, 0);
  assert.equal(g.move(15, 15, 1), false);
  assert.equal(g.click(), true);
});

test("a press that moves past the threshold drags, once, and its click doesn't restore", () => {
  const g = new EmberGesture();
  g.down(14, 14, 0);
  assert.equal(g.move(20, 14, 1), true);
  assert.equal(g.move(40, 14, 1), false);
  assert.equal(g.click(), false);
  // The next plain click restores again.
  g.down(14, 14, 0);
  assert.equal(g.click(), true);
});

test("moving with the button up, or a right press, is not a drag", () => {
  const g = new EmberGesture();
  g.down(14, 14, 0);
  assert.equal(g.move(40, 14, 0), false);
  assert.equal(g.move(60, 14, 1), false);
  g.down(14, 14, 2);
  assert.equal(g.move(40, 14, 2), false);
});

test("the ember drag and the open island's lantern drag both start Rust's dock drag", () => {
  const presence = read("src/island/presence.ts");
  assert.match(presence, /if \(gesture\.move\(e\.clientX, e\.clientY, e\.buttons\)\) void Bridge\.dockDragStart\(\);/);
  assert.match(presence, /if \(gesture\.click\(\)\) this\.model\.restore\(\);/);
  const island = read("src/island/island.ts");
  assert.ok(!island.includes("this.desktop.pickUp("), "the lantern no longer flies out to the desktop");
  assert.match(island, /this\.cancelBotHover\(\);\n\s+void Bridge\.dockDragStart\(\);/);
});
