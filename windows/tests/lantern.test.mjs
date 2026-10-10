// The lantern mascot (src/mascot): generated from the approved design, unique
// ids per instance, the notch drops the large-context groups, compact below
// 48 px, and every island state maps to one of the design's seven.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { portCss, portSvg } from "../scripts/port-lantern.mjs";
import {
  COMPACT_BELOW_PX,
  LANTERN_STATES,
  lanternClass,
  lanternIds,
  lanternMarkup,
  lanternStateFor,
} from "../src/mascot/lantern.ts";

const WINDOWS = join(dirname(fileURLToPath(import.meta.url)), "..");
const DESIGN = readFileSync(join(WINDOWS, "../docs/brand/glim-mascot.html"), "utf8");
const read = (p) => readFileSync(join(WINDOWS, p), "utf8").replace(/\r\n/g, "\n");

test("src/mascot is what the port makes of the approved design", () => {
  assert.equal(read("src/mascot/lantern.css"), portCss(DESIGN));
  assert.equal(read("src/mascot/lantern-svg.ts"), portSvg(DESIGN));
});

test("every group and class the design names is in the port", () => {
  const markup = lanternMarkup("t", "full");
  for (const cls of [
    "glow", "tone", "ribs-h", "ribs-v", "inner", "flame-g", "flame", "core", "eyes-wrap", "eyes", "eye", "hl",
    "brows", "happy", "face", "hands", "hand-l", "hand-r", "embers", "hw", "ring", "spark", "lift", "aura", "twinkles",
  ]) {
    assert.match(markup, new RegExp(`class="(?:[^"]* )?${cls}(?: [^"]*)?"`), cls);
  }
});

test("the CSS has a rule for each of the seven states, compact, and reduced motion", () => {
  const css = read("src/mascot/lantern.css");
  for (const s of LANTERN_STATES) assert.ok(css.includes(`.lantern.s-${s} `), s);
  assert.ok(css.includes(".lantern.compact "));
  assert.ok(css.includes("@media (prefers-reduced-motion: reduce){.lantern *{animation:none !important}}"));
  // Nothing unscoped could reach the rest of the app.
  for (const rule of css.split("\n").slice(1)) {
    if (!rule || rule.startsWith("@")) continue;
    for (const sel of rule.slice(0, rule.indexOf("{")).split(",")) assert.ok(sel.startsWith(".lantern"), sel);
  }
});

test("ids are unique per instance and every reference follows", () => {
  const a = lanternMarkup("a1", "full");
  const b = lanternMarkup("b2", "full");
  for (const id of lanternIds("a1")) {
    assert.ok(a.includes(`id="${id}"`), id);
    assert.ok(!b.includes(`id="${id}"`), id);
  }
  assert.ok(!a.includes("{ID}"));
  // No reference is left pointing at an id this instance doesn't define.
  for (const [, ref] of a.matchAll(/url\(#([^)]+)\)/g)) assert.ok(lanternIds("a1").includes(ref), ref);
});

test("the notch sprite has no aura or twinkles; large contexts keep them", () => {
  const notch = lanternMarkup("n", "notch");
  const full = lanternMarkup("f", "full");
  assert.ok(!notch.includes('class="aura"') && !notch.includes('class="twinkles"'));
  assert.ok(full.includes('class="aura"') && full.includes('class="twinkles"'));
  // Everything else is identical.
  const strip = (m) => m.replace(/<g class="aura"[\s\S]*?<\/g>/, "").replace(/<g class="twinkles"[\s\S]*?<\/g>/, "");
  assert.equal(notch, strip(lanternMarkup("n", "full")));
});

test("compact below 48 px of rendered height", () => {
  assert.equal(COMPACT_BELOW_PX, 48);
  assert.equal(lanternClass("think", 47.9), "lantern s-think compact");
  assert.equal(lanternClass("think", 48), "lantern s-think");
});

test("every island state maps to one of the seven", () => {
  const states = ["idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping", "dizzy"];
  const want = {
    idle: "idle", working: "think", thinking: "think", searching: "think", approval: "suggest", question: "suggest",
    error: "record", finished: "delegate", ratelimit: "paused", sleeping: "paused", dizzy: "think",
  };
  for (const s of states) assert.equal(lanternStateFor(s, false), want[s], s);
  assert.equal(lanternStateFor("idle", true), "listen", "an open island listens");
  for (const s of states) assert.ok(LANTERN_STATES.includes(lanternStateFor(s, true)));
});
