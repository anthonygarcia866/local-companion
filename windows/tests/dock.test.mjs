// Docking (src/core/layout.ts, beside src-tauri/src/placement.rs): where the
// island is drawn in its window for each dock, and how an upright pill turns.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  CORNER_GAP, DOCKS, PANEL_V_H, PANEL_W, botPosition, dockedBot, dockedSize, isDock, islandFrame,
  isVerticalDock, pillGeometry,
} from "../src/core/layout.ts";

test("the five docks, and which stand upright", () => {
  assert.deepEqual([...DOCKS], ["top-center", "top-left", "top-right", "left", "right"]);
  assert.deepEqual(DOCKS.filter(isVerticalDock), ["left", "right"]);
  assert.ok(isDock("left") && !isDock("bottom"));
});

test("an upright pill turns on its side; the open island keeps its shape", () => {
  const pill = { w: 352, h: 56 };
  assert.deepEqual(dockedSize("left", "compact", pill), { w: 56, h: 352 });
  assert.deepEqual(dockedSize("right", "hidden", { w: 184, h: 0 }), { w: 0, h: 184 });
  assert.deepEqual(dockedSize("left", "expanded", { w: 640, h: 160 }), { w: 640, h: 160 });
  assert.deepEqual(dockedSize("top-left", "compact", pill), pill);
});

test("each dock hugs its edge inside the window", () => {
  assert.deepEqual(islandFrame("top-center", 352, 56, 14), { x: 184, y: 0, w: 352, h: 56, radius: "0 0 14px 14px" });
  assert.equal(islandFrame("top-left", 352, 56, 14).x, CORNER_GAP);
  assert.equal(islandFrame("top-right", 352, 56, 14).x + 352, PANEL_W - CORNER_GAP);
  // Upright: against the edge, centred on it, rounded on the side away from it.
  const left = islandFrame("left", 56, 352, 14);
  assert.deepEqual([left.x, left.y, left.radius], [0, (PANEL_V_H - 352) / 2, "0 14px 14px 0"]);
  const right = islandFrame("right", 640, 160, 22);
  assert.deepEqual([right.x + 640, right.y, right.radius], [PANEL_W, (PANEL_V_H - 160) / 2, "22px 0 0 22px"]);
  // The open island fits the upright panel, opening away from the edge.
  assert.ok(islandFrame("left", 640, 300, 22).y >= 0);
});

test("an upright pill carries the lantern at its top", () => {
  const g = pillGeometry("large");
  const level = botPosition("compact", "overview", g.h, 0, g);
  const upright = dockedBot("left", "compact", level);
  assert.deepEqual([upright.cx, upright.cy], [level.cy, level.cx]);
  const open = botPosition("expanded", "overview", 160, 0, g);
  assert.deepEqual(dockedBot("right", "expanded", open), open);
});

test("the page and Rust agree on the upright panel and the dock names", async () => {
  const { readFileSync } = await import("node:fs");
  const rs = readFileSync(new URL("../src-tauri/src/placement.rs", import.meta.url), "utf8");
  assert.equal(Number(rs.match(/pub const PANEL_V_H: f64 = ([\d.]+);/)[1]), PANEL_V_H);
  for (const d of DOCKS) assert.ok(rs.includes(`=> "${d}"`), d);
});
